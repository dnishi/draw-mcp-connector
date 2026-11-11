//! WebTransport server that syncs with the canvas page.
//!
//! The canvas page opens one bidirectional stream and exchanges one JSON object per line (protocol.rs).
//! It first sends the token in hello, and syncing starts after approval on the PC.
//! An approved connection gets a resume key (resume.rs) and can reconnect with it without approval for a while after disconnecting.

use std::io;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use wtransport::endpoint::IncomingSession;
use wtransport::endpoint::endpoint_side::Server;
use wtransport::{
    Connection, Endpoint as WtEndpoint, Identity, RecvStream, SendStream, ServerConfig,
};

use crate::app::{App, BoxError};
use crate::canvas::{CanvasView, Origin};
use crate::link_status::LinkStatus;
use crate::net;
use crate::pairing::Endpoint;
use crate::protocol::{ClientMessage, ServerMessage};
use crate::resume::Lease;
use crate::{config, ui};

/// Maximum size of one message. Fits long strokes and rejects malicious huge lines
const MAX_MESSAGE_BYTES: u64 = 4 * 1024 * 1024;

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
    loop {
        let incoming = server.accept().await;
        let id = next_id.fetch_add(1, Ordering::Relaxed);
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_session(&app, incoming, id).await {
                warn!("接続 {id} が終了: {error}");
            }
        });
    }
}

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

impl Messages {
    /// Reads the next line. None when the peer closes the stream
    async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, BoxError> {
        self.line.clear();
        let read = (&mut self.reader)
            .take(MAX_MESSAGE_BYTES)
            .read_until(b'\n', &mut self.line)
            .await?;
        if read == 0 {
            return Ok(None);
        }
        if self.line.last() != Some(&b'\n') {
            return Err("メッセージが大きすぎるか、途中で切れている".into());
        }
        Ok(Some(serde_json::from_slice(&self.line)?))
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

async fn handle_session(app: &App, incoming: IncomingSession, id: u64) -> Result<(), BoxError> {
    let connection = incoming.await?.accept().await?;
    let remote = connection.remote_address();
    let (writer, reader) = connection.accept_bi().await?;
    let mut messages = Messages {
        reader: BufReader::new(reader),
        writer,
        line: Vec::new(),
    };

    let hello = tokio::time::timeout(config::HELLO_TIMEOUT, messages.read()).await;
    let Ok(Ok(Some(ClientMessage::Hello {
        token,
        viewport,
        page,
        resume,
    }))) = hello
    else {
        return messages.reject("Send hello first.").await;
    };
    if !app.pairing.token_matches(&token) {
        return messages
            .reject("The QR code is outdated or invalid. Scan it again.")
            .await;
    }

    // It listens on both IP versions, so IPv4 arrives as ::ffff:a.b.c.d. Convert back to IPv4 for display
    let ip = remote.ip().to_canonical();
    if resume.is_some_and(|key| app.resume_keys.redeem(&key)) {
        info!("{ip} が再接続用の鍵で繋ぎ直した。許可を求めずに受ける");
    } else if !approve(app, &connection, &mut messages, ip, id).await? {
        return Ok(());
    }

    let mut resume_key = app.resume_keys.issue();
    messages
        .write(&ServerMessage::ResumeKey {
            key: resume_key.key().into(),
        })
        .await?;
    app.status.set(id, LinkStatus::Connected(ip));
    app.canvas.set_view(CanvasView {
        id,
        ip,
        page,
        viewport,
    });
    let result = sync(app, &mut messages, id, ip, &mut resume_key).await;
    app.canvas.remove_view(id);
    app.status.remove(id);
    result
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
        // Dropping ask_approval closes the dialog
        _ = connection.closed() => {
            app.status.remove(id);
            return Ok(false);
        }
    };
    if allowed {
        info!("{ip} の接続を許可した");
    } else {
        app.status.remove(id);
        messages
            .reject("The connection was denied on the PC.")
            .await?;
    }
    Ok(allowed)
}

/// Streams canvas changes both ways on an approved connection. Also keeps replacing the resume key
async fn sync(
    app: &App,
    messages: &mut Messages,
    id: u64,
    ip: IpAddr,
    resume_key: &mut Lease<'_>,
) -> Result<(), BoxError> {
    let origin = Origin::Connection(id);
    let (strokes, mut changes) = app.canvas.subscribe();
    messages.write(&ServerMessage::Snapshot { strokes }).await?;
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
            message = messages.read::<ClientMessage>() => match message? {
                None => return Ok(()),
                Some(ClientMessage::Op { op }) => {
                    app.canvas.apply(origin, op);
                }
                Some(ClientMessage::Resize { viewport, page }) => {
                    app.canvas.set_view(CanvasView { id, ip, page, viewport });
                }
                Some(ClientMessage::Hello { .. }) => {}
            },
            change = changes.recv() => match change {
                Ok(change) if change.origin != origin => {
                    messages.write(&ServerMessage::Op { op: change.op }).await?;
                }
                Ok(_) => {}
                // If too many changes were missed, resend the whole canvas
                Err(RecvError::Lagged(_)) => {
                    let (strokes, receiver) = app.canvas.subscribe();
                    changes = receiver;
                    messages.write(&ServerMessage::Snapshot { strokes }).await?;
                }
                Err(RecvError::Closed) => {
                    return Err(io::Error::other("キャンバスが閉じた").into());
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Author, CanvasOp, PageSize, Stroke, StrokePoint, Viewport};
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicUsize;
    use wtransport::ClientConfig;

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
    async fn connect(app: &App, resume: Option<String>) -> (Client, Connection) {
        let url = app.pairing.canvas_url().unwrap();
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
        let connection = WtEndpoint::client(config)
            .unwrap()
            .connect(format!("https://127.0.0.1:{}/", config::WEBTRANSPORT_PORT))
            .await
            .unwrap();
        let (send, recv) = connection.open_bi().await.unwrap().await.unwrap();
        let mut client = Client {
            send,
            recv: BufReader::new(recv),
        };
        client
            .send(&ClientMessage::Hello {
                token: params["token"].to_string(),
                viewport: VIEWPORT,
                page: PAGE,
                resume,
            })
            .await;
        (client, connection)
    }

    /// The received resume key
    async fn receive_resume_key(client: &mut Client) -> String {
        match client.receive().await {
            ServerMessage::ResumeKey { key } => key,
            other => panic!("resumeKey のはず: {other:?}"),
        }
    }

    /// Actually connects over WebTransport and checks hello → approval → bidirectional sync.
    /// Then reconnects with the resume key (the port is fixed, so the tests that start a server are combined into one)
    #[tokio::test]
    async fn syncs_after_approval_and_resumes() {
        let (app, asked) = serve();
        let (mut client, connection) = connect(&app, None).await;
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
            ServerMessage::Snapshot { strokes: vec![] }
        );

        let op = CanvasOp::Insert {
            stroke: stroke("page-1"),
            index: 0,
        };
        client.send(&ClientMessage::Op { op }).await;
        // AI changes reach the canvas, and the canvas's own changes are not sent back
        let ai_op = CanvasOp::Remove {
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

        // Reconnecting with the key returns to syncing without asking for approval.
        // If the old connection the key was handed to is still alive, it is told and closed
        let (mut resumed, _resumed_connection) = connect(&app, Some(key.clone())).await;
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
        let (mut client, _connection) = connect(&app, Some(key)).await;
        assert_eq!(
            client.receive().await,
            ServerMessage::AwaitingApproval,
            "使った鍵では許可を求める"
        );
    }
}
