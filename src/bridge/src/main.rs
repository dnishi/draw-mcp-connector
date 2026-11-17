// Do not open a console window on Windows (the only UI is the tray icon)
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

// About in the menu (only on OSes with a tray)
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod about;
mod ai_inbox;
mod app;
#[cfg(test)]
mod app_icon;
// The connection approval dialog is for macOS / Windows (tests also run on the development Linux)
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod approval_dialog;
mod canvas;
mod color;
mod config;
mod disconnect;
mod geometry;
mod glyphs;
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
mod icon;
mod link_status;
mod lock;
mod mcp;
mod net;
mod pairing;
mod paths;
mod protocol;
mod render;
mod resume;
mod shapes;
mod store;
// The QR code is used only in the window (macOS / Windows). Tests also run on Linux for development
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

use tracing::{error, info, warn};

use app::App;
use store::Store;

/// Runs the WebTransport and MCP servers. Exits when either stops
async fn serve(app: Arc<App>) {
    // The save task does not stop the servers even if it ends
    if let Some(store) = app.store.clone() {
        tokio::spawn(store::run(app.clone(), store));
    }
    let result = tokio::try_join!(webtransport::run(app.clone()), mcp::run(app));
    if let Err(error) = result {
        error!("サーバーが止まった: {error}");
        std::process::exit(1);
    }
}

fn main() {
    tracing_subscriber::fmt::init();
    info!("{} を起動", config::APP_NAME);
    let app = match paths::data_dir() {
        Some(dir) => Arc::new(App::with_store(Store::new(dir))),
        None => {
            warn!("データの置き場所が決まらないため、キャンバスの状態を保存しない");
            Arc::new(App::new())
        }
    };
    let runtime = tokio::runtime::Runtime::new().expect("tokio を起動できない");

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // The main thread runs the tray event loop, so the servers run on another thread
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
