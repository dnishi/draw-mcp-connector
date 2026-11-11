//! Connection state with the canvas. Used for the menu bar icon and menu, and for waiting in MCP's connect_canvas.

use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::watch;

use crate::lock::lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Disconnected,
    /// Waiting for approval on the PC
    Waiting(IpAddr),
    Connected(IpAddr),
}

type Listener = Box<dyn Fn() + Send + Sync>;

pub struct LinkStatusCell {
    /// State to display: the state of the connection whose state changed last. Disconnected if there are no connections
    status: watch::Sender<LinkStatus>,
    /// State of each ongoing connection (waiting for approval or connected). Later entries changed more recently.
    /// After a reload reconnects, the end of the old connection may arrive after the new one, so state is kept per connection
    links: Mutex<Vec<(u64, LinkStatus)>>,
    /// Called when a connection's state changes (redraws the menu bar icon and menu)
    listener: OnceLock<Listener>,
}

impl Default for LinkStatusCell {
    fn default() -> Self {
        Self {
            status: watch::channel(LinkStatus::Disconnected).0,
            links: Mutex::default(),
            listener: OnceLock::new(),
        }
    }
}

impl LinkStatusCell {
    /// State to display (the state of the connection whose state changed last)
    pub fn get(&self) -> LinkStatus {
        *self.status.borrow()
    }

    /// States of all ongoing connections, in connection order (by connection number)
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn all(&self) -> Vec<LinkStatus> {
        let mut links = lock(&self.links).clone();
        links.sort_by_key(|(id, _)| *id);
        links.into_iter().map(|(_, status)| status).collect()
    }

    /// Changes the state of connection `connection` (Waiting or Connected)
    pub fn set(&self, connection: u64, status: LinkStatus) {
        self.update(|links| {
            links.retain(|(id, _)| *id != connection);
            links.push((connection, status));
        });
    }

    /// Connection `connection` ended (denied or disconnected)
    pub fn remove(&self, connection: u64) {
        self.update(|links| links.retain(|(id, _)| *id != connection));
    }

    /// Changes the list of connections and notifies if it changed (the menu shows every connection,
    /// so it notifies when connections are added or removed even if the displayed state is the same)
    fn update(&self, change: impl FnOnce(&mut Vec<(u64, LinkStatus)>)) {
        let changed = {
            let mut links = lock(&self.links);
            let before = links.clone();
            change(&mut links);
            let shown = links
                .last()
                .map_or(LinkStatus::Disconnected, |(_, status)| *status);
            self.status.send_replace(shown);
            *links != before
        };
        if changed && let Some(listener) = self.listener.get() {
            listener();
        }
    }

    /// Registers the function called when a connection's state changes (once). Only OSes with a tray register one (tray.rs).
    /// Read the current state with get and all
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn set_listener(&self, listener: impl Fn() + Send + Sync + 'static) {
        let _ = self.listener.set(Box::new(listener));
    }

    /// Waits until a connection is established (approved). None after timeout
    pub async fn wait_connected(&self, timeout: Duration) -> Option<IpAddr> {
        let mut receiver = self.status.subscribe();
        let connected = receiver.wait_for(|status| matches!(status, LinkStatus::Connected(_)));
        match tokio::time::timeout(timeout, connected).await {
            Ok(Ok(status)) => match *status {
                LinkStatus::Connected(ip) => Some(ip),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::{Arc, Mutex};

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 2));
    const OTHER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 3));

    /// Records the state of all connections each time a notification arrives
    fn record_notifications(cell: &Arc<LinkStatusCell>) -> Arc<Mutex<Vec<Vec<LinkStatus>>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (log, weak) = (seen.clone(), Arc::downgrade(cell));
        cell.set_listener(move || {
            let cell = weak.upgrade().unwrap();
            log.lock().unwrap().push(cell.all());
        });
        seen
    }

    #[test]
    fn notifies_only_on_change() {
        let cell = Arc::new(LinkStatusCell::default());
        let seen = record_notifications(&cell);

        cell.set(1, LinkStatus::Waiting(IP));
        cell.set(1, LinkStatus::Waiting(IP));
        cell.remove(2);
        cell.set(1, LinkStatus::Connected(IP));
        cell.remove(1);

        assert_eq!(
            *seen.lock().unwrap(),
            [
                vec![LinkStatus::Waiting(IP)],
                vec![LinkStatus::Connected(IP)],
                vec![],
            ]
        );
        assert_eq!(cell.get(), LinkStatus::Disconnected);
    }

    /// When a second device connects, notifies even if the displayed state is the same kind, and returns all connections in connection order
    #[test]
    fn lists_every_connection_in_connection_order() {
        let cell = Arc::new(LinkStatusCell::default());
        let seen = record_notifications(&cell);

        cell.set(2, LinkStatus::Connected(OTHER));
        cell.set(1, LinkStatus::Connected(IP));
        assert_eq!(
            cell.all(),
            [LinkStatus::Connected(IP), LinkStatus::Connected(OTHER)]
        );
        assert_eq!(seen.lock().unwrap().len(), 2);

        cell.remove(2);
        assert_eq!(cell.all(), [LinkStatus::Connected(IP)]);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    /// Even if the end of the old connection arrives after reconnecting, the new connection's state stays
    #[test]
    fn keeps_the_newer_connection_when_an_older_one_ends() {
        let cell = LinkStatusCell::default();
        cell.set(1, LinkStatus::Connected(IP));
        cell.set(2, LinkStatus::Connected(IP));
        cell.remove(1);
        assert_eq!(cell.get(), LinkStatus::Connected(IP));

        // If another device waits for approval while connected and is denied, it goes back to connected
        cell.set(3, LinkStatus::Waiting(OTHER));
        assert_eq!(cell.get(), LinkStatus::Waiting(OTHER));
        cell.remove(3);
        assert_eq!(cell.get(), LinkStatus::Connected(IP));
    }

    #[tokio::test]
    async fn waits_until_connected() {
        let cell = Arc::new(LinkStatusCell::default());
        let waiter = tokio::spawn({
            let cell = cell.clone();
            async move { cell.wait_connected(Duration::from_secs(5)).await }
        });
        cell.set(1, LinkStatus::Waiting(IP));
        cell.set(1, LinkStatus::Connected(IP));
        assert_eq!(waiter.await.unwrap(), Some(IP));

        let empty = LinkStatusCell::default();
        assert_eq!(empty.wait_connected(Duration::from_millis(10)).await, None);
    }
}
