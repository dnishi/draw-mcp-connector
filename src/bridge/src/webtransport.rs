//! WebTransport server that syncs with the canvas page.
//!
//! The canvas page opens one bidirectional stream and exchanges one JSON object per line (protocol.rs).
//! It first sends the token in hello, and syncing starts after approval on the PC.
//! An approved connection gets a resume key (resume.rs) and can reconnect with it without approval for a while after disconnecting.

use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tracing::{info, warn};
use wtransport::endpoint::IncomingSession;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::{
    Connection, Endpoint as WtEndpoint, Identity, RecvStream, SendStream, ServerConfig,
};

use crate::ai_inbox;
use crate::app::{App, BoxError};
use crate::canvas::{CanvasView, Origin};
use crate::disconnect::DisconnectWatch;
use crate::link_status::LinkStatus;
use crate::lock::lock;
use crate::net;
use crate::pairing::Endpoint;
use crate::protocol::{CanvasOp, ClientMessage, ImageStream, PROTOCOL_VERSION, ServerMessage};
use crate::render::StoredImage;
use crate::resume::Lease;
use crate::{config, ui};

/// Maximum size of one message. Fits long strokes and rejects malicious huge lines
const MAX_MESSAGE_BYTES: u64 = 4 * 1024 * 1024;
/// Upper limit of image byte size. The page shrinks the longer side to 2000px into a PNG, so that fits
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

pub async fn run(app: Arc<App>) -> Result<(), BoxError> {
    let server = start(&app)?;
    ui::show_on_first_run(&app);
    accept_sessions(app, server).await
}

/// Creates the certificate, starts listening, and registers the endpoint put in the QR code
fn start(app: &Arc<App>) -> Result<Arc<WtEndpoint<Server>>, BoxError> {
    let (identity, endpoint) = new_identity()?;
    let server = Arc::new(WtEndpoint::server(server_config(identity))?);
    app.pairing.set_endpoint(endpoint);
    info!("WebTransport: {}:{} で待ち受け", endpoint.ip, endpoint.port);
    tokio::spawn(renew_certificate_periodically(app.clone(), server.clone()));
    Ok(server)
}

async fn accept_sessions(app: Arc<App>, server: Arc<WtEndpoint<Server>>) -> Result<(), BoxError> {
    let next_id = AtomicU64::new(1);
    // Up to config::MAX_SESSIONS at once, counting both approved connections and those awaiting approval.
    // A permit is held until the connection ends and is returned then
    let sessions = Arc::new(tokio::sync::Semaphore::new(config::MAX_SESSIONS));
    loop {
        let incoming = server.accept().await;
        let id = next_id.fetch_add(1, Ordering::Relaxed);
        let permit = match sessions.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                // Dropping incoming closes the connection
                warn!(
                    "同時に扱う接続の上限({})を超えたので、接続 {id} を断る",
                    config::MAX_SESSIONS
                );
                continue;
            }
        };
        let app = app.clone();
        tokio::spawn(async move {
            // The permit is held until handle_session ends
            let _permit = permit;
            if let Err(error) = handle_session(app, incoming, id).await {
                warn!("接続 {id} が終了: {error}");
            }
        });
    }
}

/// Origin of the canvas page (`config::CANVAS_URL`). Connection requests from any other Origin are rejected.
/// Built once so it is not parsed every time
static CANVAS_ORIGIN: LazyLock<String> = LazyLock::new(|| {
    url::Url::parse(config::CANVAS_URL)
        .expect("config::CANVAS_URL が URL として不正")
        .origin()
        .ascii_serialization()
});

fn server_config(identity: Identity) -> ServerConfig {
    ServerConfig::builder()
        .with_bind_default(config::WEBTRANSPORT_PORT)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build()
}

/// Creates an ECDSA P-256 self-signed certificate (wtransport's self_signed_builder creates P-256)
fn new_identity() -> Result<(Identity, Endpoint), BoxError> {
    let ip = net::lan_ip()?;
    let identity = Identity::self_signed_builder()
        .subject_alt_names([ip.to_string()])
        .from_now_utc()
        .validity_days(config::CERTIFICATE_VALIDITY_DAYS)
        .build()?;
    let certificate = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or("証明書が空")?;
    let endpoint = Endpoint {
        ip,
        port: config::WEBTRANSPORT_PORT,
        certificate_hash: *certificate.hash().as_ref(),
    };
    Ok((identity, endpoint))
}

/// Regenerates the certificate before it expires. The hash changes, so the QR code must be scanned again.
/// Canvases already connected keep working (the new certificate is used from new connections)
async fn renew_certificate_periodically(app: Arc<App>, server: Arc<WtEndpoint<Server>>) {
    loop {
        tokio::time::sleep(config::CERTIFICATE_RENEW_INTERVAL).await;
        let renewed = new_identity().and_then(|(identity, endpoint)| {
            server.reload_config(server_config(identity), false)?;
            Ok(endpoint)
        });
        match renewed {
            Ok(endpoint) => {
                app.pairing.set_endpoint(endpoint);
                info!("証明書を作り直した。QR コードの読み直しが必要");
            }
            Err(error) => warn!("証明書を作り直せなかった: {error}"),
        }
    }
}

struct Messages {
    reader: BufReader<RecvStream>,
    writer: SendStream,
    line: Vec<u8>,
}

/// Turns the whole canvas (snapshot) into the form sent to the page. Only the sheets and the current sheet are sent;
/// the revision and the per-page views (for the AI) are left out
impl From<crate::canvas::Snapshot> for ServerMessage {
    fn from(snapshot: crate::canvas::Snapshot) -> Self {
        Self::Snapshot {
            sheets: snapshot.sheets,
            current: snapshot.current,
        }
    }
}

/// Turns the current AI waiting state into a message for the page. Marks the sent value as seen
fn ai_status_message(ai_status: &mut watch::Receiver<ai_inbox::AiStatus>) -> ServerMessage {
    let status = ai_status.borrow_and_update();
    ServerMessage::AiStatus {
        waiting: status.waiting,
        queued: status.queued,
    }
}

/// Result of `read_line`
#[derive(Debug, PartialEq, Eq)]
enum LineRead {
    /// A whole line was read (into `line`, including the newline)
    Line,
    /// The peer closed the stream with nothing partly read
    Closed,
}

/// Appends one line, up to the newline, to `line`.
/// If interrupted by `select!`, the partial line stays in `line` (and the reader's buffer),
/// so calling again continues from there (`read_until` is cancel safe).
/// When `line` grows beyond `limit`, or the peer closes mid-line, the error is
/// "the message is too large or was cut off"
async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
    limit: u64,
) -> io::Result<LineRead> {
    // Read only what remains up to the limit (the limit does not reset on each call)
    let read = (&mut *reader)
        .take(limit.saturating_sub(line.len() as u64))
        .read_until(b'\n', line)
        .await?;
    if line.ends_with(b"\n") {
        return Ok(LineRead::Line);
    }
    match read {
        // Closed with nothing read
        0 if line.is_empty() => Ok(LineRead::Closed),
        // Beyond limit, or closed mid-line
        _ => Err(io::Error::other(
            "メッセージが大きすぎるか、途中で切れている",
        )),
    }
}

impl Messages {
    /// Reads the next line. None when the peer closes the stream
    async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, BoxError> {
        if matches!(
            read_line(&mut self.reader, &mut self.line, MAX_MESSAGE_BYTES).await?,
            LineRead::Closed
        ) {
            return Ok(None);
        }
        let message = serde_json::from_slice(&self.line);
        // Clear the finished line even if parsing the JSON fails (do not leave it as a partial line)
        self.line.clear();
        Ok(Some(message?))
    }

    async fn write(&mut self, message: &ServerMessage) -> Result<(), BoxError> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        self.writer.write_all(&line).await?;
        Ok(())
    }

    async fn reject(&mut self, reason: &str) -> Result<(), BoxError> {
        self.write(&ServerMessage::Rejected {
            reason: reason.into(),
        })
        .await?;
        self.writer.finish().await?;
        Ok(())
    }
}

/// Result from accepting a connection to reading hello
struct Handshake {
    connection: Connection,
    messages: Messages,
    remote: SocketAddr,
    hello: Option<ClientMessage>,
}

/// Rejects the connection request with 403 if its Origin is not the canvas page's;
/// otherwise opens the stream and reads hello
async fn handshake(incoming: IncomingSession) -> Result<Handshake, BoxError> {
    let request = incoming.await?;
    match request.origin() {
        // Accept only connections from the canvas page
        Some(origin) if origin == CANVAS_ORIGIN.as_str() => {}
        other => {
            warn!(
                "Origin が {other:?} の接続を断る(期待: {})",
                CANVAS_ORIGIN.as_str()
            );
            request.forbidden().await;
            return Err("Origin が違う".into());
        }
    }
    let connection = request.accept().await?;
    let remote = connection.remote_address();
    let (writer, reader) = connection.accept_bi().await?;
    let mut messages = Messages {
        reader: BufReader::new(reader),
        writer,
        line: Vec::new(),
    };
    // An unreadable or broken hello is rejected by the caller with a reason, like a missing one
    let hello = messages.read().await.ok().flatten();
    Ok(Handshake {
        connection,
        messages,
        remote,
        hello,
    })
}

async fn handle_session(app: Arc<App>, incoming: IncomingSession, id: u64) -> Result<(), BoxError> {
    // One time limit covers everything from accepting the connection to receiving hello, so a connection
    // that never opens a stream does not linger. A timeout is a normal event, so it is logged at info
    let handshake = match tokio::time::timeout(config::HELLO_TIMEOUT, handshake(incoming)).await {
        Err(_) => {
            info!("接続 {id} は hello が届く前に時間切れ");
            return Ok(());
        }
        Ok(handshake) => handshake?,
    };
    let Handshake {
        connection,
        mut messages,
        remote,
        hello,
    } = handshake;
    let Some(ClientMessage::Hello {
        token,
        viewport,
        page,
        resume,
        protocol,
    }) = hello
    else {
        return messages.reject("Send hello first.").await;
    };
    // Mixed versions: reject pages with an older version and pages with a newer one
    if protocol < PROTOCOL_VERSION {
        return messages
            .reject("This canvas page is out of date. Reload the page.")
            .await;
    }
    if protocol > PROTOCOL_VERSION {
        return messages
            .reject("The PC app is out of date. Update it.")
            .await;
    }

    // It listens on both IP versions, so IPv4 arrives as ::ffff:a.b.c.d. Convert back to IPv4 for display
    let ip = remote.ip().to_canonical();
    // Once a connection sets a status, clear it however the connection ends (even when leaving with `?` on an error)
    let _status = StatusGuard { app: &app, id };
    // Check the resume key before the token: when an approved page is reloaded in the same tab, it sends
    // the old token left in the URL along with the resume key. The token was regenerated after approval,
    // so the page could not reconnect unless the key is checked first.
    // Checking the key first is safe: it is a secret given only to approved connections, and it is invalid after one use
    if resume.is_some_and(|key| app.resume_keys.redeem(&key)) {
        info!("{ip} が再接続用の鍵で繋ぎ直した。許可を求めずに受ける");
    } else if !app.pairing.token_matches(&token) {
        return messages
            .reject("The QR code is outdated or invalid. Scan it again.")
            .await;
    } else if !approve(&app, &connection, &mut messages, ip, id).await? {
        return Ok(());
    } else {
        // Regenerate the token after approval (so the old QR code and URLs left in history can no longer be used)
        app.pairing.renew_token();
    }

    let mut resume_key = app.resume_keys.issue();
    messages
        .write(&ServerMessage::ResumeKey {
            key: resume_key.key().into(),
        })
        .await?;
    // Be ready for a close request from the PC's menu as soon as the connection is shown as connected
    let disconnect = app.disconnects.watch(id);
    app.status.set(id, LinkStatus::Connected(ip));
    app.canvas.set_view(CanvasView {
        id,
        ip,
        page,
        viewport,
    });
    let result = sync(
        app.clone(),
        &connection,
        &mut messages,
        id,
        ip,
        &mut resume_key,
        &disconnect,
    )
    .await;
    app.canvas.remove_view(id);
    result
}

/// Always clears the status when a connection that set it ends (however it ends).
/// Keep it after creating it; it clears the status on `Drop`
struct StatusGuard<'a> {
    app: &'a App,
    id: u64,
}

impl Drop for StatusGuard<'_> {
    fn drop(&mut self) {
        self.app.status.remove(self.id);
    }
}

/// Asks for approval on the PC. true if approved.
/// If refused, sends the reason to the page. If the connection closes before the answer, closes the dialog and returns false
async fn approve(
    app: &App,
    connection: &Connection,
    messages: &mut Messages,
    ip: IpAddr,
    id: u64,
) -> Result<bool, BoxError> {
    messages.write(&ServerMessage::AwaitingApproval).await?;
    info!("{ip} から接続の要求。許可を待つ");
    app.status.set(id, LinkStatus::Waiting(ip));
    let allowed = tokio::select! {
        allowed = app.ui.ask_approval(ip) => allowed,
        // Dropping ask_approval closes the dialog. StatusGuard clears the status
        _ = connection.closed() => {
            return Ok(false);
        }
    };
    if allowed {
        info!("{ip} の接続を許可した");
    } else {
        messages
            .reject("The connection was denied on the PC.")
            .await?;
    }
    Ok(allowed)
}

/// Streams canvas changes both ways over an approved connection, also replacing the reconnect key.
/// Image bytes are exchanged on separate (unidirectional) streams
async fn sync(
    app: Arc<App>,
    connection: &Connection,
    messages: &mut Messages,
    id: u64,
    ip: IpAddr,
    resume_key: &mut Lease<'_>,
    disconnect: &DisconnectWatch<'_>,
) -> Result<(), BoxError> {
    let origin = Origin::Connection(id);
    // Receiving and sending image streams are each handled in a separate task (so that large image transfers
    // do not stop the sync loop and a failure does not close the connection). Finished tasks are taken out in select
    // and panics are logged (otherwise tasks keep piling up while the connection lasts).
    // Leaving the loop drops the JoinSet, which stops any remaining tasks
    let mut image_streams = tokio::task::JoinSet::new();
    // IDs of images that were requested but have not arrived yet. Sent to this connection when the contents arrive
    // (the same connection that sent a Request for contents to its page may receive a Request from another page)
    let wanted: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut images_ready = app.canvas.subscribe_images();
    let (state, mut changes) = app.canvas.subscribe();
    messages.write(&state.into()).await?;
    // Whether the AI is waiting (Send to AI) is sent once right after snapshot, then whenever it changes
    let mut ai_status = app.ai_inbox.subscribe();
    messages.write(&ai_status_message(&mut ai_status)).await?;
    let rotation_start = tokio::time::Instant::now() + config::RESUME_KEY_LIFETIME;
    let mut rotation = tokio::time::interval_at(rotation_start, config::RESUME_KEY_LIFETIME);

    loop {
        tokio::select! {
            _ = rotation.tick() => {
                let key = resume_key.rotate().into();
                messages.write(&ServerMessage::ResumeKey { key }).await?;
            }
            () = resume_key.superseded() => {
                info!("接続 {id} に渡した鍵で別の接続が繋ぎ直したので、この接続を切る");
                messages.write(&ServerMessage::Superseded).await?;
                messages.writer.finish().await?;
                return Ok(());
            }
            () = disconnect.requested() => {
                info!("接続 {id} を PC のメニューから切る");
                resume_key.revoke();
                messages.write(&ServerMessage::Disconnected).await?;
                messages.writer.finish().await?;
                return Ok(());
            }
            Ok(()) = ai_status.changed() => {
                messages.write(&ai_status_message(&mut ai_status)).await?;
            }
            message = messages.read::<ClientMessage>() => match message? {
                None => return Ok(()),
                Some(ClientMessage::Op { op, seq }) => {
                    // An image's contents arrive only once, when imported. For an image it does not have
                    // (e.g. one brought back by undo after the oldest were dropped), ask the page that placed it
                    let missing = match &op {
                        CanvasOp::SetImage { image, .. }
                            if app.canvas.image_data(&image.id).is_none() =>
                        {
                            Some(image.id.clone())
                        }
                        _ => None,
                    };
                    app.canvas.apply(origin, op);
                    if let Some(missing) = missing {
                        spawn_image_send(
                            &mut image_streams,
                            id,
                            connection.clone(),
                            ImageStream::Request { id: missing },
                            None,
                        );
                    }
                    // Report that the operation was handled even if it was ignored (not applied)
                    messages.write(&ServerMessage::Ack { seq }).await?;
                }
                Some(ClientMessage::Resize { viewport, page }) => {
                    app.canvas.set_view(CanvasView { id, ip, page, viewport });
                }
                // Send to AI was pressed. Hand it to MCP's wait_for_canvas (the next call if none is waiting)
                Some(ClientMessage::SendToAi) => {
                    app.ai_inbox.press(id);
                }
                Some(ClientMessage::Hello { .. }) => {}
            },
            stream = connection.accept_uni() => {
                // Comes over the same connection as the pen strokes, so only an approved peer can use it
                match stream {
                    Ok(stream) => {
                        let app = Arc::clone(&app);
                        let connection = connection.clone();
                        let wanted = Arc::clone(&wanted);
                        image_streams.spawn(async move {
                            // An image failure does not close the connection (stroke sync can continue)
                            if let Err(error) = handle_image_stream(&app, &connection, &wanted, stream).await {
                                warn!("接続 {id} の画像ストリームが失敗: {error}");
                            }
                        });
                    }
                    // A failure of accept_uni itself is treated as the connection being closed
                    Err(error) => return Err(error.into()),
                }
            }
            change = changes.recv() => match change {
                Ok(change) => {
                    // When the placement is removed, replaced with another image, or its sheet removed, the remembered contents
                    // become meaningless. Keep only images still placed (so as not to drop those waited for on
                    // other sheets). The canvas lock and the wanted lock are never held together
                    match &change.op {
                        CanvasOp::SetImage { .. }
                        | CanvasOp::RemoveImage { .. }
                        | CanvasOp::RemoveSheet { .. }
                        | CanvasOp::ResetSheets => {
                            let placed = app.canvas.placed_image_ids();
                            lock(&wanted).retain(|id| placed.contains(id));
                        }
                        _ => {}
                    }
                    if change.origin != origin {
                        messages.write(&ServerMessage::Op { op: change.op }).await?;
                    }
                }
                // When too many changes were missed, send the whole canvas again
                Err(RecvError::Lagged(_)) => {
                    let (state, receiver) = app.canvas.subscribe();
                    changes = receiver;
                    messages.write(&state.into()).await?;
                }
                Err(RecvError::Closed) => {
                    return Err(io::Error::other("キャンバスが閉じた").into());
                }
            },
            ready = images_ready.recv() => match ready {
                Ok(ready) => {
                    // Send only the IDs it remembers (other connections may receive the same notice)
                    if lock(&wanted).remove(&ready)
                        && let Some(image) = app.canvas.image_data(&ready)
                    {
                        spawn_image_send(
                            &mut image_streams,
                            id,
                            connection.clone(),
                            ImageStream::Provide { id: ready },
                            Some(image),
                        );
                    }
                }
                // If notices were missed, recount the remembered ones (send those that have arrived)
                Err(RecvError::Lagged(_)) => {
                    let remembered: Vec<String> = lock(&wanted).drain().collect();
                    for remembered in remembered {
                        match app.canvas.image_data(&remembered) {
                            Some(image) => spawn_image_send(
                                &mut image_streams,
                                id,
                                connection.clone(),
                                ImageStream::Provide { id: remembered },
                                Some(image),
                            ),
                            None => {
                                lock(&wanted).insert(remembered);
                            }
                        }
                    }
                }
                // The notice sender exists as long as the app is alive, so it never closes
                Err(RecvError::Closed) => {}
            },
            // Take out finished image tasks (otherwise they keep piling up while the connection lasts).
            // Send/receive failures are logged as warn inside the task. Panics are only logged here and the connection is kept
            Some(finished) = image_streams.join_next(), if !image_streams.is_empty() => {
                if let Err(error) = finished {
                    warn!("接続 {id} の画像のタスクが終了: {error}");
                }
            }
        }
    }
}

/// Handles a stream of image bytes. The first line is the header (ImageStream); for provide the PNG follows.
/// If the contents of a requested image have not arrived yet, remembers it in `wanted`,
/// and the sync loop sends it to this connection when the contents arrive
async fn handle_image_stream(
    app: &App,
    connection: &Connection,
    wanted: &Mutex<HashSet<String>>,
    stream: RecvStream,
) -> Result<(), BoxError> {
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let read = (&mut reader)
        .take(MAX_MESSAGE_BYTES)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 || line.last() != Some(&b'\n') {
        return Err("画像のストリームのヘッダーが不正".into());
    }
    match serde_json::from_slice::<ImageStream>(&line)? {
        ImageStream::Provide { id } => {
            let mut bytes = Vec::new();
            // Read one more byte; if it exceeds the limit, notice before it gets truncated
            let count = match reader
                .take(MAX_IMAGE_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
            {
                Ok(count) => count,
                Err(error) => {
                    // The contents were cut off. The page ignores a Request it is in the middle of sending, so
                    // if the image is placed and has no contents, ask this connection again to resend it
                    if app.canvas.placed_image(&id).is_some()
                        && app.canvas.image_data(&id).is_none()
                        && let Err(send_error) =
                            send_image_stream(connection, &ImageStream::Request { id }, None).await
                    {
                        warn!("中身が途中で切れた画像を求め直せなかった: {send_error}");
                    }
                    return Err(error.into());
                }
            };
            if count as u64 > MAX_IMAGE_BYTES {
                return Err(format!("画像が大きすぎる: {count} バイト").into());
            }
            app.canvas.put_image_data(&id, bytes);
        }
        ImageStream::Request { id } => {
            if let Some(image) = app.canvas.image_data(&id) {
                send_image_stream(
                    connection,
                    &ImageStream::Provide { id },
                    Some(image.bytes()),
                )
                .await?;
                return Ok(());
            }
            // No contents yet. If the image is placed (the requesting page should be showing it),
            // remember it until the arrival notice. Remembering missing ones is pointless
            if app.canvas.placed_image(&id).is_some() {
                lock(wanted).insert(id.clone());
                // If it arrived right after being remembered, send it without waiting for the notice (in case the notice was missed)
                if let Some(image) = app.canvas.image_data(&id)
                    && lock(wanted).remove(&id)
                {
                    send_image_stream(
                        connection,
                        &ImageStream::Provide { id },
                        Some(image.bytes()),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

/// Opens a unidirectional stream, writes the header on the first line and the bytes after it if there is a body, then closes it
async fn send_image_stream(
    connection: &Connection,
    header: &ImageStream,
    body: Option<&[u8]>,
) -> Result<(), BoxError> {
    let mut sender = connection.open_uni().await?.await?;
    let mut line = serde_json::to_vec(header)?;
    line.push(b'\n');
    sender.write_all(&line).await?;
    if let Some(body) = body {
        sender.write_all(body).await?;
    }
    sender.finish().await?;
    Ok(())
}

/// Sends image bytes in a separate task. Does not stop the sync loop and does not close the connection on failure
/// (the order of sending is not guaranteed; the receiver treats each stream as self-contained)
fn spawn_image_send(
    image_streams: &mut tokio::task::JoinSet<()>,
    id: u64,
    connection: Connection,
    header: ImageStream,
    body: Option<Arc<StoredImage>>,
) {
    image_streams.spawn(async move {
        let body = body.as_deref().map(StoredImage::bytes);
        if let Err(error) = send_image_stream(&connection, &header, body).await {
            warn!("接続 {id} への画像ストリームの送信が失敗: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        Author, CanvasOp, INITIAL_SHEET_ID, ImagePlacement, PageSize, Sheet, Stroke, StrokePoint,
        Viewport,
    };
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::AsyncWriteExt;
    use wtransport::endpoint::ConnectOptions;
    use wtransport::{ClientConfig, VarInt};

    fn stroke(id: &str) -> Stroke {
        Stroke {
            id: id.into(),
            author: Author::Human,
            color: crate::color::BLACK.into(),
            points: vec![StrokePoint {
                x: 1.0,
                y: 2.0,
                pressure: 0.5,
            }],
        }
    }

    struct Client {
        send: SendStream,
        recv: BufReader<RecvStream>,
    }

    impl Client {
        async fn send(&mut self, message: &ClientMessage) {
            let mut line = serde_json::to_vec(message).unwrap();
            line.push(b'\n');
            self.send.write_all(&line).await.unwrap();
        }

        async fn receive(&mut self) -> ServerMessage {
            let mut line = Vec::new();
            self.recv.read_until(b'\n', &mut line).await.unwrap();
            serde_json::from_slice(&line).unwrap()
        }
    }

    const VIEWPORT: Viewport = Viewport {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    const PAGE: Option<PageSize> = Some(PageSize {
        width: 800.0,
        height: 600.0,
    });

    /// Runs a server that approves every connection and counts approval requests
    fn serve() -> (Arc<App>, Arc<AtomicUsize>) {
        let app = Arc::new(App::new());
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = asked.clone();
        // Instead of the approval dialog
        app.ui.set_approver(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        });
        let server = start(&app).unwrap();
        tokio::spawn(accept_sessions(app.clone(), server));
        (app, asked)
    }

    /// Connects with the connection info from the QR code URL and sends hello
    /// Waits until the condition holds (fails after 5 seconds)
    async fn wait_until(condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("条件が満たされない");
    }

    /// Connects with the canvas page's Origin (the URL has the current token)
    async fn connect(app: &App, resume: Option<String>, protocol: u32) -> (Client, Connection) {
        connect_with_origin(app, resume, protocol, CANVAS_ORIGIN.as_str())
            .await
            .unwrap()
    }

    /// Connects with the given token (to test an old token)
    async fn connect_with_token(
        app: &App,
        token: &str,
        resume: Option<String>,
        protocol: u32,
    ) -> (Client, Connection) {
        let url = app.pairing.canvas_url().unwrap();
        connect_with(&url, token, resume, protocol, CANVAS_ORIGIN.as_str())
            .await
            .unwrap()
    }

    /// Connects with the connection info from the QR code URL and the given Origin, and sends hello
    async fn connect_with_origin(
        app: &App,
        resume: Option<String>,
        protocol: u32,
        origin: &str,
    ) -> Result<(Client, Connection), BoxError> {
        let url = app.pairing.canvas_url().unwrap();
        let token = url.fragment().unwrap().rsplit_once("token=").unwrap().1;
        connect_with(&url, token, resume, protocol, origin).await
    }

    /// Connects with the connection info from the QR code URL (hash, IP and port) and sends hello with the given token
    async fn connect_with(
        url: &url::Url,
        token: &str,
        resume: Option<String>,
        protocol: u32,
        origin: &str,
    ) -> Result<(Client, Connection), BoxError> {
        let params: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(url.fragment().unwrap().as_bytes()).collect();
        let hash: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&params["hash"][i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let hash: [u8; 32] = hash.try_into().unwrap();

        let config = ClientConfig::builder()
            .with_bind_default()
            .with_server_certificate_hashes([hash.into()])
            .build();
        let options =
            ConnectOptions::builder(format!("https://127.0.0.1:{}/", config::WEBTRANSPORT_PORT))
                .add_header("origin", origin)
                .build();
        let connection = WtEndpoint::client(config).unwrap().connect(options).await?;
        let (send, recv) = connection.open_bi().await?.await?;
        let mut client = Client {
            send,
            recv: BufReader::new(recv),
        };
        client
            .send(&ClientMessage::Hello {
                token: token.to_string(),
                viewport: VIEWPORT,
                page: PAGE,
                resume,
                protocol,
            })
            .await;
        Ok((client, connection))
    }

    /// The received resume key
    async fn receive_resume_key(client: &mut Client) -> String {
        match client.receive().await {
            ServerMessage::ResumeKey { key } => key,
            other => panic!("resumeKey のはず: {other:?}"),
        }
    }

    /// The ack returned for an operation (the number of the operation sent)
    async fn receive_ack(client: &mut Client, seq: u64) {
        assert_eq!(client.receive().await, ServerMessage::Ack { seq });
    }

    /// The AI waiting state that arrives right after snapshot (false / false, since nothing has happened yet)
    async fn receive_ai_status(client: &mut Client) {
        assert_eq!(
            client.receive().await,
            ServerMessage::AiStatus {
                waiting: false,
                queued: false
            }
        );
    }

    /// Sends a Provide header and part of the contents on a unidirectional stream, then cuts it off (reset).
    /// RESET_STREAM discards data not yet delivered, so after sending the header,
    /// it waits for the server to read it before starting to write the contents and cutting them off
    async fn send_truncated_provide(connection: &Connection, id: &str) {
        let mut stream = connection.open_uni().await.unwrap().await.unwrap();
        let header = serde_json::to_vec(&ImageStream::Provide { id: id.to_string() }).unwrap();
        stream.write_all(&header).await.unwrap();
        stream.write_all(b"\n").await.unwrap();
        // Wait for the header to reach the peer
        tokio::time::sleep(Duration::from_millis(200)).await;
        stream.write_all(&b"half"[..]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        stream.reset(VarInt::from_u32(1)).unwrap();
    }

    /// Actually connects over WebTransport and checks hello → approval → bidirectional sync.
    /// Then reconnects with the reconnect key (the port is fixed, so tests that start a server are kept in one)
    #[tokio::test]
    async fn syncs_after_approval_and_resumes() {
        let (app, asked) = serve();
        // The token in the QR code before approval
        let first_token = app
            .pairing
            .canvas_url()
            .unwrap()
            .fragment()
            .unwrap()
            .rsplit_once("token=")
            .unwrap()
            .1
            .to_string();
        let (mut client, connection) = connect(&app, None, PROTOCOL_VERSION).await;
        assert_eq!(client.receive().await, ServerMessage::AwaitingApproval);

        let connected = app.status.wait_connected(Duration::from_secs(5)).await;
        assert_eq!(
            connected,
            Some(Ipv4Addr::LOCALHOST.into()),
            "許可されて接続中になる"
        );
        let key = receive_resume_key(&mut client).await;
        assert_eq!(
            client.receive().await,
            ServerMessage::Snapshot {
                sheets: vec![Sheet {
                    id: INITIAL_SHEET_ID.into(),
                    strokes: vec![],
                    image: None
                }],
                current: INITIAL_SHEET_ID.into()
            }
        );
        receive_ai_status(&mut client).await;

        // The token is regenerated after approval: a new connection with the old token is rejected
        let (mut stale, _stale_connection) =
            connect_with_token(&app, &first_token, None, PROTOCOL_VERSION).await;
        assert_eq!(
            stale.receive().await,
            ServerMessage::Rejected {
                reason: "The QR code is outdated or invalid. Scan it again.".into(),
            }
        );

        let op = CanvasOp::Insert {
            sheet: INITIAL_SHEET_ID.into(),
            stroke: stroke("page-1"),
            index: 0,
        };
        client
            .send(&ClientMessage::Op {
                op: op.clone(),
                seq: 1,
            })
            .await;
        receive_ack(&mut client, 1).await;
        // AI changes reach the canvas, and the canvas's own changes are not sent back
        let ai_op = CanvasOp::Remove {
            sheet: INITIAL_SHEET_ID.into(),
            ids: vec!["page-1".into()],
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.canvas.apply(Origin::Ai, ai_op.clone()).is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(client.receive().await, ServerMessage::Op { op: ai_op });
        let views = app.canvas.snapshot().views;
        assert_eq!(views.len(), 1);
        assert_eq!(
            (views[0].ip, views[0].viewport, views[0].page),
            (Ipv4Addr::LOCALHOST.into(), VIEWPORT, PAGE)
        );

        // Send to AI: the page's press arrives as AiStatus and is handed to MCP's wait_for_canvas
        let pressed_by = app
            .status
            .all()
            .into_iter()
            .filter(|(_, status)| matches!(status, LinkStatus::Connected(_)))
            .map(|(id, _)| id)
            .max()
            .expect("この接続は接続中になっている");
        client.send(&ClientMessage::SendToAi).await;
        assert_eq!(
            client.receive().await,
            ServerMessage::AiStatus {
                waiting: false,
                queued: true
            }
        );
        assert_eq!(
            app.ai_inbox.wait(Duration::from_millis(500)).await,
            Some(pressed_by),
            "wait は押した接続の番号を返す"
        );
        // After it is picked up, waiting: false and queued: false are sent.
        // The waiting: true notice while waiting may be merged with the next one and not arrive
        loop {
            match client.receive().await {
                ServerMessage::AiStatus {
                    waiting: false,
                    queued: false,
                } => break,
                ServerMessage::AiStatus { .. } => {}
                other => panic!("AiStatus のはず: {other:?}"),
            }
        }

        // Images: the bytes arrive on a separate (unidirectional) stream and the placement in an op
        let placement = ImagePlacement {
            id: "img1".into(),
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 20.0,
        };
        let mut provide = connection.open_uni().await.unwrap().await.unwrap();
        provide
            .write_all(&serde_json::to_vec(&ImageStream::Provide { id: "img1".into() }).unwrap())
            .await
            .unwrap();
        provide.write_all(b"\n").await.unwrap();
        provide.write_all(b"fake-png").await.unwrap();
        provide.finish().await.unwrap();
        // The order between streams is not fixed, so send the placement after the bytes arrive
        // (if the placement arrives first, the bridge requests the bytes, which gets mixed up with the reply to the request below)
        wait_until(|| app.canvas.image_data("img1").is_some()).await;
        client
            .send(&ClientMessage::Op {
                op: CanvasOp::SetImage {
                    sheet: INITIAL_SHEET_ID.into(),
                    image: placement.clone(),
                },
                seq: 2,
            })
            .await;
        receive_ack(&mut client, 2).await;
        wait_until(|| app.canvas.snapshot().current_sheet().image.is_some()).await;
        assert_eq!(
            app.canvas
                .image_data("img1")
                .as_deref()
                .map(StoredImage::bytes),
            Some(&b"fake-png"[..])
        );
        assert_eq!(
            app.canvas.snapshot().current_sheet().image,
            Some(placement.clone())
        );

        // A request from a page that does not have the bytes: the bridge sends them back on a unidirectional stream
        let mut request = connection.open_uni().await.unwrap().await.unwrap();
        request
            .write_all(&serde_json::to_vec(&ImageStream::Request { id: "img1".into() }).unwrap())
            .await
            .unwrap();
        request.write_all(b"\n").await.unwrap();
        request.finish().await.unwrap();
        let mut reply = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        reply.read_to_end(&mut received).await.unwrap();
        let split = received
            .iter()
            .position(|&byte| byte == b'\n')
            .expect("ヘッダー行の後にバイト列");
        let (header, body) = received.split_at(split + 1);
        assert_eq!(
            serde_json::from_slice::<ImageStream>(header).unwrap(),
            ImageStream::Provide { id: "img1".into() }
        );
        assert_eq!(body, b"fake-png");

        // When a placement arrives for an image the bridge does not have, it asks the placing page for the bytes
        client
            .send(&ClientMessage::Op {
                op: CanvasOp::SetImage {
                    sheet: INITIAL_SHEET_ID.into(),
                    image: ImagePlacement {
                        id: "img2".into(),
                        ..placement.clone()
                    },
                },
                seq: 3,
            })
            .await;
        receive_ack(&mut client, 3).await;
        let mut wanted = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        wanted.read_to_end(&mut received).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<ImageStream>(&received).unwrap(),
            ImageStream::Request { id: "img2".into() }
        );

        // Several image streams with an invalid header: only logged; the connection is kept and stroke sync continues
        // (one that is not JSON, one that is JSON but not an ImageStream, one that does not end with a newline)
        for header in ["not a header\n", "null\n", "{ truncated"] {
            let mut broken = connection.open_uni().await.unwrap().await.unwrap();
            broken.write_all(header.as_bytes()).await.unwrap();
            broken.finish().await.unwrap();
        }
        let keep = CanvasOp::Insert {
            sheet: INITIAL_SHEET_ID.into(),
            stroke: stroke("keep-going"),
            index: 0,
        };
        assert!(app.canvas.apply(Origin::Ai, keep.clone()).is_some());
        let message = tokio::time::timeout(Duration::from_secs(5), client.receive())
            .await
            .expect("画像ストリームの失敗で接続が切れない");
        assert_eq!(message, ServerMessage::Op { op: keep });

        // An image requested before its contents: place it (after receiving the Request the bridge sends back),
        // and once the contents arrive they are sent to the requesting connection
        client
            .send(&ClientMessage::Op {
                op: CanvasOp::SetImage {
                    sheet: INITIAL_SHEET_ID.into(),
                    image: ImagePlacement {
                        id: "img3".into(),
                        ..placement.clone()
                    },
                },
                seq: 4,
            })
            .await;
        receive_ack(&mut client, 4).await;
        wait_until(|| {
            app.canvas
                .snapshot()
                .current_sheet()
                .image
                .as_ref()
                .is_some_and(|image| image.id == "img3")
        })
        .await;
        // The bridge that received the placement asks for the contents (the page discards it as something it cannot send)
        let mut asked_back = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        asked_back.read_to_end(&mut received).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<ImageStream>(&received).unwrap(),
            ImageStream::Request { id: "img3".into() }
        );
        // Another page asks for the contents (not there yet)
        let mut request = connection.open_uni().await.unwrap().await.unwrap();
        request
            .write_all(&serde_json::to_vec(&ImageStream::Request { id: "img3".into() }).unwrap())
            .await
            .unwrap();
        request.write_all(b"\n").await.unwrap();
        request.finish().await.unwrap();
        // Then the contents arrive
        let mut provide = connection.open_uni().await.unwrap().await.unwrap();
        provide
            .write_all(&serde_json::to_vec(&ImageStream::Provide { id: "img3".into() }).unwrap())
            .await
            .unwrap();
        provide.write_all(b"\n").await.unwrap();
        provide.write_all(b"late-png").await.unwrap();
        provide.finish().await.unwrap();
        // The contents are sent back to the requesting connection
        let mut reply = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .expect("中身が届いたら、求めていた接続へ送られる")
            .unwrap();
        let mut received = Vec::new();
        reply.read_to_end(&mut received).await.unwrap();
        let split = received
            .iter()
            .position(|&byte| byte == b'\n')
            .expect("ヘッダー行の後にバイト列");
        let (header, body) = received.split_at(split + 1);
        assert_eq!(
            serde_json::from_slice::<ImageStream>(header).unwrap(),
            ImageStream::Provide { id: "img3".into() }
        );
        assert_eq!(body, b"late-png");

        // A stream whose contents were cut off midway: for a placed image without contents, ask this connection again
        // (the page ignores Requests while still sending, so it would not be resent unless asked again)
        client
            .send(&ClientMessage::Op {
                op: CanvasOp::SetImage {
                    sheet: INITIAL_SHEET_ID.into(),
                    image: ImagePlacement {
                        id: "img4".into(),
                        ..placement.clone()
                    },
                },
                seq: 5,
            })
            .await;
        receive_ack(&mut client, 5).await;
        wait_until(|| {
            app.canvas
                .snapshot()
                .current_sheet()
                .image
                .as_ref()
                .is_some_and(|image| image.id == "img4")
        })
        .await;
        // The request the bridge sent on receiving the placement (this test page does not have the contents)
        let mut asked_back = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        asked_back.read_to_end(&mut received).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<ImageStream>(&received).unwrap(),
            ImageStream::Request { id: "img4".into() }
        );
        // Start sending the contents and cut them off midway
        send_truncated_provide(&connection, "img4").await;
        // After the cut-off, it asks this connection again
        let mut asked_again = tokio::time::timeout(Duration::from_secs(5), connection.accept_uni())
            .await
            .expect("中身が切れたら求め直す")
            .unwrap();
        let mut received = Vec::new();
        asked_again.read_to_end(&mut received).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<ImageStream>(&received).unwrap(),
            ImageStream::Request { id: "img4".into() }
        );

        // When the contents of an image that is not placed are cut off, it does not ask again
        send_truncated_provide(&connection, "img5").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(500), connection.accept_uni())
                .await
                .is_err(),
            "置かれていない画像は求め直さない"
        );

        // An ignored operation (unknown sheet) also gets an ack with the same seq (applied operations are tested with 1–5 above)
        client
            .send(&ClientMessage::Op {
                op: CanvasOp::RemoveSheet {
                    id: "no-such-sheet".into(),
                },
                seq: 6,
            })
            .await;
        receive_ack(&mut client, 6).await;

        // Reconnecting with the old token (left in the URL) and the key resumes syncing without asking for approval.
        // If the old connection given the key is still open, notify it and close it
        let (mut resumed, _resumed_connection) =
            connect_with_token(&app, &first_token, Some(key.clone()), PROTOCOL_VERSION).await;
        let next_key = receive_resume_key(&mut resumed).await;
        assert_ne!(next_key, key, "繋ぎ直すたびに新しい鍵を渡す");
        assert!(matches!(
            resumed.receive().await,
            ServerMessage::Snapshot { .. }
        ));
        assert_eq!(asked.load(Ordering::Relaxed), 1, "許可は最初の1回だけ");
        assert_eq!(client.receive().await, ServerMessage::Superseded);
        connection.closed().await;
        assert_eq!(
            app.status.get(),
            LinkStatus::Connected(Ipv4Addr::LOCALHOST.into()),
            "古い接続が切れても、繋ぎ直した接続の状態のまま"
        );
        // The closed connection's view is forgotten; only the reconnected connection's remains
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.canvas.snapshot().views.len() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("繋いでいるページは 1 つだけになる");

        // A key can be used only once
        let (mut client, _connection) = connect(&app, Some(key), PROTOCOL_VERSION).await;
        assert_eq!(
            client.receive().await,
            ServerMessage::AwaitingApproval,
            "使った鍵では許可を求める"
        );

        // Disconnect from the PC's menu: invalidate the key, notify the page, and close the connection
        let (mut client, connection) = connect(&app, None, PROTOCOL_VERSION).await;
        assert_eq!(client.receive().await, ServerMessage::AwaitingApproval);
        let key = receive_resume_key(&mut client).await;
        assert!(matches!(
            client.receive().await,
            ServerMessage::Snapshot { .. }
        ));
        receive_ai_status(&mut client).await;
        // Close the connected connection with the largest number (the last one this test connected)
        let id = app
            .status
            .all()
            .into_iter()
            .filter(|(_, status)| matches!(status, LinkStatus::Connected(_)))
            .map(|(id, _)| id)
            .max()
            .expect("この接続は接続中になっている");
        assert!(app.disconnects.request(id));
        assert_eq!(client.receive().await, ServerMessage::Disconnected);
        connection.closed().await;
        // The key given to the closed connection is invalid, so approval is asked again
        let (mut client, _connection) = connect(&app, Some(key), PROTOCOL_VERSION).await;
        assert_eq!(
            client.receive().await,
            ServerMessage::AwaitingApproval,
            "切った接続の鍵では許可を求める"
        );
        // After it closes (after the watch is dropped), it can no longer be requested
        wait_until(|| !app.disconnects.request(id)).await;
        // When an approved connection drops, its status is cleared too
        wait_until(|| {
            !app.status
                .all()
                .into_iter()
                .any(|(connection_id, _)| connection_id == id)
        })
        .await;

        // Pages with an older version (0: before sheets, 1: no acks) are rejected before the token is checked
        for protocol in [0, 1] {
            let (mut client, _connection) = connect(&app, None, protocol).await;
            assert_eq!(
                client.receive().await,
                ServerMessage::Rejected {
                    reason: "This canvas page is out of date. Reload the page.".into(),
                }
            );
        }

        // A connection request from an Origin other than the canvas page is rejected with 403 (no connection is established)
        // (the port is fixed, so the tests that start a server are combined into this one)
        assert!(
            connect_with_origin(&app, None, PROTOCOL_VERSION, "https://evil.example")
                .await
                .is_err(),
            "Origin が違う接続は確立しない"
        );
    }

    #[tokio::test]
    async fn read_line_keeps_partial_reads_across_cancellation() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        writer.write_all(b"hel").await.unwrap();
        // The line is incomplete, so the read is interrupted by the timeout. The partial line stays in line
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                read_line(&mut reader, &mut line, 100)
            )
            .await
            .is_err(),
            "1 行がそろっていない"
        );
        writer.write_all(b"lo\n").await.unwrap();
        assert_eq!(
            read_line(&mut reader, &mut line, 100).await.unwrap(),
            LineRead::Line,
            "呼び直すと続きから読んで 1 行がそろう"
        );
        assert_eq!(&line[..], b"hello\n");
    }

    #[tokio::test]
    async fn read_line_rejects_lines_over_the_limit() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        writer.write_all(b"abc\n").await.unwrap();
        assert!(
            read_line(&mut reader, &mut line, 3).await.is_err(),
            "limit を超えた 1 行はエラー"
        );
    }

    #[tokio::test]
    async fn read_line_reports_closing() {
        // Closing with nothing partly read means "the peer closed"
        let (writer, reader) = tokio::io::duplex(64);
        drop(writer);
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        assert_eq!(
            read_line(&mut reader, &mut line, 100).await.unwrap(),
            LineRead::Closed
        );
        // Closing mid-line is an error
        let (mut writer, reader) = tokio::io::duplex(64);
        writer.write_all(b"ab").await.unwrap();
        drop(writer);
        let mut reader = BufReader::new(reader);
        assert!(
            read_line(&mut reader, &mut line, 100).await.is_err(),
            "読みかけのまま閉じたらエラー"
        );
    }
}
