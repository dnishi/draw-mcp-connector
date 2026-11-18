//! Information used to accept connections from the canvas: the endpoint and one-time token put in the QR code.
//! Connection approval is handled by ui.rs (dialog), and connection state by link_status.rs.

use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};

use url::Url;

use crate::config;
use crate::lock::lock;
use crate::net;

/// Where the canvas page connects. Changes every time the certificate is regenerated
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub ip: IpAddr,
    pub port: u16,
    /// SHA-256 of the certificate (DER)
    pub certificate_hash: [u8; 32],
}

struct State {
    endpoint: Option<Endpoint>,
    token: String,
}

type Listener = Box<dyn Fn() + Send + Sync>;

pub struct Pairing {
    state: Mutex<State>,
    /// Called when the QR contents (address or token) change (redraws an open QR window)
    listener: OnceLock<Listener>,
}

impl Pairing {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                endpoint: None,
                token: new_token(),
            }),
            listener: OnceLock::new(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Updates the endpoint. Also regenerates the token so that old QR codes stop working
    pub fn set_endpoint(&self, endpoint: Endpoint) {
        let mut state = self.state();
        state.endpoint = Some(endpoint);
        state.token = new_token();
        drop(state);
        self.notify_changed();
    }

    /// Regenerates only the token (called after a connection is approved).
    /// So the old token (which may remain in the browser history) can no longer be used
    pub fn renew_token(&self) {
        let mut state = self.state();
        state.token = new_token();
        drop(state);
        self.notify_changed();
    }

    /// Calls the registered function, after releasing the state lock
    fn notify_changed(&self) {
        if let Some(listener) = self.listener.get() {
            listener();
        }
    }

    /// Registers the function called when the QR contents (address or token) change (once).
    /// Only registered on OSes with a tray (tray.rs)
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn set_listener(&self, listener: impl Fn() + Send + Sync + 'static) {
        let _ = self.listener.set(Box::new(listener));
    }

    pub fn token_matches(&self, token: &str) -> bool {
        net::constant_time_eq(&self.state().token, token)
    }

    /// URL of the canvas page put in the QR code.
    /// Connection info goes after # in the URL (the browser never sends the part after # to the server)
    pub fn canvas_url(&self) -> Option<Url> {
        let state = self.state();
        let endpoint = state.endpoint?;
        let fragment = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("ip", &endpoint.ip.to_string())
            .append_pair("port", &endpoint.port.to_string())
            .append_pair("hash", &net::to_hex(&endpoint.certificate_hash))
            .append_pair("token", &state.token)
            .finish();
        let mut url = Url::parse(config::CANVAS_URL).expect("CANVAS_URL は正しい URL");
        url.set_fragment(Some(&fragment));
        Some(url)
    }
}

fn new_token() -> String {
    net::random_hex(16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn urls_carry_connection_info_in_fragment() {
        let pairing = Pairing::new();
        assert!(pairing.canvas_url().is_none());

        pairing.set_endpoint(Endpoint {
            ip: Ipv4Addr::new(192, 168, 0, 10).into(),
            port: 47100,
            certificate_hash: [0xab; 32],
        });
        let url = pairing.canvas_url().unwrap();
        let fragment = url.fragment().unwrap();
        assert!(fragment.starts_with("ip=192.168.0.10&port=47100&hash=abab"));
        let token = fragment.rsplit_once("token=").unwrap().1;
        assert!(pairing.token_matches(token));
    }

    #[test]
    fn set_endpoint_invalidates_token() {
        let pairing = Pairing::new();
        let endpoint = Endpoint {
            ip: Ipv4Addr::LOCALHOST.into(),
            port: 1,
            certificate_hash: [0; 32],
        };
        pairing.set_endpoint(endpoint);
        let url = pairing.canvas_url().unwrap();
        let token = url.fragment().unwrap().rsplit_once("token=").unwrap().1;
        pairing.set_endpoint(endpoint);
        assert!(!pairing.token_matches(token));
    }

    #[test]
    fn renew_token_invalidates_old_token() {
        let pairing = Pairing::new();
        pairing.set_endpoint(Endpoint {
            ip: Ipv4Addr::LOCALHOST.into(),
            port: 1,
            certificate_hash: [0; 32],
        });
        let url = pairing.canvas_url().unwrap();
        let token = url.fragment().unwrap().rsplit_once("token=").unwrap().1;
        pairing.renew_token();
        assert!(!pairing.token_matches(token));
    }

    #[test]
    fn notifies_when_the_qr_content_changes() {
        let pairing = Pairing::new();
        let notified = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = notified.clone();
        pairing.set_listener(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        pairing.set_endpoint(Endpoint {
            ip: Ipv4Addr::LOCALHOST.into(),
            port: 1,
            certificate_hash: [0; 32],
        });
        assert_eq!(
            notified.load(Ordering::Relaxed),
            1,
            "set_endpoint で呼ばれる"
        );
        pairing.renew_token();
        assert_eq!(
            notified.load(Ordering::Relaxed),
            2,
            "renew_token でも呼ばれる"
        );
    }
}
