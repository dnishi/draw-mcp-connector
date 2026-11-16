//! Lets the menu (main thread) ask a connection task (tokio) to close.
//! Only connections with a registered watch can be closed; a connection that is asked invalidates its key on the PC and closes (webtransport.rs)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::lock::lock;

#[derive(Default)]
pub struct DisconnectRequests {
    /// Connection number → notifier. Registered only while the watch (DisconnectWatch) is held
    requests: Mutex<HashMap<u64, Arc<Notify>>>,
}

impl DisconnectRequests {
    /// Registers and returns a watch that waits until the connection is asked to close. Dropping the watch removes the registration
    pub fn watch(&self, connection: u64) -> DisconnectWatch<'_> {
        lock(&self.requests).insert(connection, Arc::new(Notify::new()));
        DisconnectWatch {
            requests: &self.requests,
            connection,
        }
    }

    /// Asks a registered connection to close. false if it is not registered (already closed).
    /// Called only from the menu (tray.rs, on OSes with a tray) and tests
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn request(&self, connection: u64) -> bool {
        let requests = lock(&self.requests);
        let Some(request) = requests.get(&connection) else {
            return false;
        };
        // notify_one leaves a permit even when nobody is waiting,
        // so a request made before the connection task enters select! is not lost
        request.notify_one();
        true
    }
}

/// A watch the connection task holds only while connected. Dropping it removes the registration
pub struct DisconnectWatch<'a> {
    requests: &'a Mutex<HashMap<u64, Arc<Notify>>>,
    connection: u64,
}

impl DisconnectWatch<'_> {
    /// Waits until asked to close
    pub async fn requested(&self) {
        // The notifier is an Arc, so clone it and do not hold the Mutex guard across the await (keeps the task's future Send)
        let request = lock(self.requests)
            .get(&self.connection)
            .expect("見張りを落としている間は登録がある")
            .clone();
        request.notified().await
    }
}

impl Drop for DisconnectWatch<'_> {
    fn drop(&mut self) {
        lock(self.requests).remove(&self.connection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn request_wakes_the_watch() {
        let requests = DisconnectRequests::default();
        let watch = requests.watch(1);
        // Even if asked before waiting (before the connection task enters select!),
        // the permit from notify_one remains, so the request is not lost
        assert!(requests.request(1));
        tokio::time::timeout(Duration::from_secs(1), watch.requested())
            .await
            .expect("頼むと requested が返る");
    }

    #[test]
    fn request_is_false_after_the_watch_is_dropped() {
        let requests = DisconnectRequests::default();
        {
            let _watch = requests.watch(1);
        }
        assert!(!requests.request(1), "見張りを落とした後は切れている");
    }

    #[test]
    fn request_is_false_for_an_unwatched_connection() {
        let requests = DisconnectRequests::default();
        assert!(!requests.request(1));
    }
}
