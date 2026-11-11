// Do not open a console window on Windows (the only UI is the tray icon)
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

// About in the menu (only on OSes with a tray)
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod about;
mod app;
#[cfg(test)]
mod app_icon;
// The connection approval dialog is macOS only (tests also run on other OSes)
#[cfg(any(test, target_os = "macos"))]
mod approval_dialog;
mod canvas;
mod color;
mod config;
mod geometry;
mod glyphs;
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod icon;
mod link_status;
mod lock;
mod mcp;
mod net;
mod pairing;
mod protocol;
mod render;
mod resume;
mod shapes;
// The QR code is used only in the window (macOS / Windows). Tests also run on other OSes
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod qr;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod qr_window;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod tray;
#[cfg(test)]
mod typescript;
mod ui;
mod webtransport;

use std::sync::Arc;

use tracing::{error, info};

use app::App;

/// Runs the WebTransport and MCP servers. Exits when either stops
async fn serve(app: Arc<App>) {
    let result = tokio::try_join!(webtransport::run(app.clone()), mcp::run(app));
    if let Err(error) = result {
        error!("サーバーが止まった: {error}");
        std::process::exit(1);
    }
}

fn main() {
    tracing_subscriber::fmt::init();
    info!("{} を起動", config::APP_NAME);
    let app = Arc::new(App::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio を起動できない");

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // The main thread runs the tray event loop, so the servers run on another thread
        #[cfg(target_os = "macos")]
        app.ui.set_approver(approval_dialog::ask);
        let server_app = app.clone();
        tray::run(app, move || {
            std::thread::spawn(move || runtime.block_on(serve(server_app)));
        });
    }

    // On OSes without a tray (Linux, etc.), run only the servers
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    runtime.block_on(serve(app));
}
