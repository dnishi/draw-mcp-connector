//! connect_canvas tool: shows the QR code in the app's window and waits until a canvas page connects and is approved in the app.
//! The user approves in the Draw MCP Connector dialog (approval is never done on the AI agent side).

use crate::app::App;
use crate::config;
use crate::link_status::LinkStatus;

pub async fn connect_canvas(app: &App) -> String {
    if let LinkStatus::Connected(ip) = app.status.get() {
        return format!("キャンバス({ip})は接続済みです。");
    }
    if app.pairing.canvas_url().is_none() {
        return "キャンバスと接続する準備ができていません。少し待ってから、もう一度実行してください。"
            .into();
    }
    if !app.ui.open_qr() {
        return "QR コードのウィンドウを開けません(この OS ではメニューバーのアプリとして動いていません)。".into();
    }
    match app.status.wait_connected(config::USER_RESPONSE_TIMEOUT).await {
        Some(ip) => format!("キャンバス({ip})を接続しました。"),
        None => "制限時間内に接続されませんでした。PC に表示した QR コードをカメラで読み取ってキャンバスを開き、Draw MCP Connector のダイアログで許可してから、もう一度実行してください。".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::Endpoint;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 2));

    fn app_with_endpoint() -> App {
        let app = App::new();
        app.pairing.set_endpoint(Endpoint {
            ip: Ipv4Addr::LOCALHOST.into(),
            port: 1,
            certificate_hash: [0; 32],
        });
        app
    }

    #[tokio::test]
    async fn opens_qr_window_and_waits_for_approval() {
        let app = Arc::new(app_with_endpoint());
        let opened = Arc::new(AtomicBool::new(false));
        let flag = opened.clone();
        app.ui.set_qr_opener(move || {
            flag.store(true, Ordering::Relaxed);
            true
        });
        // The canvas opened by scanning the QR code is approved in the app and becomes connected
        tokio::spawn({
            let app = app.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                app.status.set(1, LinkStatus::Connected(IP));
            }
        });

        assert_eq!(
            connect_canvas(&app).await,
            "キャンバス(192.168.0.2)を接続しました。"
        );
        assert!(opened.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn reports_already_connected_without_opening_qr() {
        let app = app_with_endpoint();
        app.status.set(1, LinkStatus::Connected(IP));
        assert_eq!(
            connect_canvas(&app).await,
            "キャンバス(192.168.0.2)は接続済みです。"
        );
    }

    #[tokio::test]
    async fn reports_when_window_cannot_open() {
        let text = connect_canvas(&app_with_endpoint()).await;
        assert!(text.contains("ウィンドウを開けません"));
    }
}
