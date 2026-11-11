//! Entry point to the app's UI (the menu bar app): the QR code window, the connection approval dialog, and the first-run guide.
//!
//! The window is owned by the main-thread event loop (tray.rs), and the approval dialog is shown by the per-OS implementation
//! (approval_dialog.rs). The servers (WebTransport, MCP) run on another thread,
//! so they ask through functions registered at startup. On OSes without a tray (Linux, etc.), nothing is registered.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;
use tracing::warn;

use crate::app::App;

type Opener = Box<dyn Fn() -> bool + Send + Sync>;
/// Shows an approval dialog (does not wait for the answer)
type Approver = Arc<dyn Fn(IpAddr) -> Arc<dyn ApprovalPrompt> + Send + Sync>;

/// A shown approval dialog
pub trait ApprovalPrompt: Send + Sync {
    /// Waits for the user's answer (blocks the thread). true if approved, false if closed
    fn answer(&self) -> bool;
    /// Closes without waiting for the answer. Does nothing if called after the answer is out
    fn close(&self);
}

/// For tests: a dialog that immediately returns a fixed answer
#[cfg(test)]
impl ApprovalPrompt for bool {
    fn answer(&self) -> bool {
        *self
    }

    fn close(&self) {}
}

/// Closes the dialog when dropped
struct CloseOnDrop(Arc<dyn ApprovalPrompt>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Default)]
pub struct Ui {
    qr_opener: OnceLock<Opener>,
    approver: OnceLock<Approver>,
    /// Shows only one approval dialog at a time
    asking: Mutex<()>,
}

impl Ui {
    /// Registers the function that opens the QR code window (once)
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn set_qr_opener(&self, opener: impl Fn() -> bool + Send + Sync + 'static) {
        if self.qr_opener.set(Box::new(opener)).is_err() {
            warn!("QR コードのウィンドウを開く関数は登録済み");
        }
    }

    /// Registers the function that shows the approval dialog (once)
    #[cfg(any(test, target_os = "macos"))]
    pub fn set_approver<P: ApprovalPrompt + 'static>(
        &self,
        approver: impl Fn(IpAddr) -> P + Send + Sync + 'static,
    ) {
        let approver: Approver = Arc::new(move |ip| Arc::new(approver(ip)));
        if self.approver.set(approver).is_err() {
            warn!("許可を求める関数は登録済み");
        }
    }

    /// Asks to open the QR code window. true if the request went through
    pub fn open_qr(&self) -> bool {
        self.qr_opener.get().is_some_and(|open| open())
    }

    /// Asks for approval of a connection. Denies if there is no way to ask.
    /// If this is dropped before the answer (the connecting party disconnected), the dialog is closed
    pub async fn ask_approval(&self, ip: IpAddr) -> bool {
        let Some(approver) = self.approver.get() else {
            warn!("この OS では接続の許可を求められないため、{ip} からの接続を断る");
            return false;
        };
        let _one_at_a_time = self.asking.lock().await;
        let prompt = approver(ip);
        let _close_when_dropped = CloseOnDrop(prompt.clone());
        tokio::task::spawn_blocking(move || prompt.answer())
            .await
            .unwrap_or(false)
    }
}

/// Marker file showing the guide has been shown. Stored in the OS's settings directory
fn marker_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
    };
    Some(base.join(env!("CARGO_BIN_NAME")).join("first-run-done"))
}

/// Opens the QR code window on first launch. Marks the guide as done only if it opened
pub fn show_on_first_run(app: &App) {
    let Some(marker) = marker_path() else {
        return;
    };
    if marker.exists() || !app.ui.open_qr() {
        return;
    }
    let written = marker
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&marker, ""));
    if let Err(error) = written {
        warn!("{} を書けない: {error}", marker.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 2));

    #[test]
    fn opens_qr_only_when_an_opener_is_registered() {
        let ui = Ui::default();
        assert!(!ui.open_qr());

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        ui.set_qr_opener(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            true
        });
        assert!(ui.open_qr());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn denies_without_an_approver() {
        assert!(!Ui::default().ask_approval(IP).await);
    }

    #[tokio::test]
    async fn asks_the_registered_approver() {
        let ui = Ui::default();
        ui.set_approver(|ip| ip == IP);
        assert!(ui.ask_approval(IP).await);
        assert!(!ui.ask_approval(Ipv4Addr::LOCALHOST.into()).await);
    }

    /// A dialog that never answers until it is closed
    struct Unanswered(Arc<AtomicBool>);

    impl ApprovalPrompt for Unanswered {
        fn answer(&self) -> bool {
            while !self.0.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            false
        }

        fn close(&self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn closes_the_dialog_when_no_longer_waited_for() {
        let ui = Ui::default();
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        ui.set_approver(move |_| Unanswered(flag.clone()));

        let asked = tokio::time::timeout(Duration::from_millis(50), ui.ask_approval(IP)).await;
        assert!(asked.is_err(), "答えは出ていない");
        assert!(closed.load(Ordering::Relaxed), "待つのをやめたら閉じる");
    }
}
