//! connect_canvas tool: shows the QR code in the app's window and waits until a canvas page connects and is approved in the app.
//! The user approves in the Draw MCP Connector dialog (approval is never done on the AI agent side).

use std::time::Duration;

use crate::app::App;
use crate::config;
use crate::link_status::LinkStatus;

pub async fn connect_canvas(app: &App) -> String {
    connect_canvas_within(app, config::MCP_TOOL_WAIT_TIMEOUT).await
}

/// Body of connect_canvas. Takes the maximum time to wait for a connection as an argument (so tests can use a short one)
async fn connect_canvas_within(app: &App, timeout: Duration) -> String {
    if let LinkStatus::Connected(ip) = app.status.get() {
        return format!("The canvas ({ip}) is already connected.");
    }
    if app.pairing.canvas_url().is_none() {
        return "Not ready to connect to the canvas yet. Wait a moment and run this again.".into();
    }
    if !app.ui.open_qr() {
        return "Cannot open the QR code window (the app is not running as a menu bar app on this OS)."
            .into();
    }
    match app.status.wait_connected(timeout).await {
        Some(ip) => format!("The canvas ({ip}) is connected."),
        // The QR code window and the approval dialog on the PC stay open, so calling again keeps waiting
        None => "Not connected yet. The QR code is still shown on the PC. Call connect_canvas again to keep waiting."
            .into(),
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
            "The canvas (192.168.0.2) is connected."
        );
        assert!(opened.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn reports_already_connected_without_opening_qr() {
        let app = app_with_endpoint();
        app.status.set(1, LinkStatus::Connected(IP));
        assert_eq!(
            connect_canvas(&app).await,
            "The canvas (192.168.0.2) is already connected."
        );
    }

    #[tokio::test]
    async fn reports_when_window_cannot_open() {
        let text = connect_canvas(&app_with_endpoint()).await;
        assert!(text.contains("Cannot open the QR code window"));
    }

    /// On timeout, returns text asking the AI to call again
    #[tokio::test]
    async fn prompts_to_call_again_when_the_time_runs_out() {
        let app = Arc::new(app_with_endpoint());
        app.ui.set_qr_opener(|| true);
        assert_eq!(
            connect_canvas_within(&app, Duration::from_millis(100)).await,
            "Not connected yet. The QR code is still shown on the PC. Call connect_canvas again to keep waiting."
        );
    }
}
