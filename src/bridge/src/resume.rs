//! Resume keys. Handed only to connections approved on the PC; a reconnect presenting the key is accepted
//! without the approval dialog (reloading the page, or reconnecting from the connection status in the sidebar).
//!
//! To keep a stolen key from being usable for long:
//! - Keys are short-lived (config::RESUME_KEY_LIFETIME). While connected, the key is replaced at this interval
//!   and the old key is invalidated immediately. After disconnecting, the last key handed out stays usable for this long
//! - A key is invalidated after one use, and the reconnected connection gets a new key
//! - If the key is used while the connection it was handed to is still alive, the old connection is closed (and its page is told)

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::lock::lock;
use crate::net;

struct Entry {
    key: String,
    /// While the connection the key was handed to is alive, a notifier telling that connection to close. None after disconnecting
    holder: Option<Arc<Notify>>,
    /// Expiry. None while connected; decided when the connection ends
    expires: Option<Instant>,
}

impl Entry {
    fn usable(&self, now: Instant) -> bool {
        self.expires.is_none_or(|expires| now < expires)
    }
}

pub struct ResumeKeys {
    /// How long a key stays usable after disconnecting
    lifetime: Duration,
    entries: Mutex<Vec<Entry>>,
}

impl ResumeKeys {
    pub fn new(lifetime: Duration) -> Self {
        Self {
            lifetime,
            entries: Mutex::new(Vec::new()),
        }
    }

    /// List after discarding expired keys
    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        let mut entries = lock(&self.entries);
        let now = Instant::now();
        entries.retain(|entry| entry.usable(now));
        entries
    }

    fn add(&self, holder: &Arc<Notify>) -> String {
        let key = net::random_hex(16);
        self.entries().push(Entry {
            key: key.clone(),
            holder: Some(holder.clone()),
            expires: None,
        });
        key
    }

    /// Creates a key for an approved connection. Keep the returned Lease until the connection ends
    pub fn issue(&self) -> Lease<'_> {
        let superseded = Arc::new(Notify::new());
        let key = self.add(&superseded);
        Lease {
            keys: self,
            key,
            superseded,
        }
    }

    /// If the key is usable, invalidates it and returns true. If its connection is still alive, tells that connection to close
    pub fn redeem(&self, key: &str) -> bool {
        let mut entries = self.entries();
        let Some(index) = entries
            .iter()
            .position(|entry| net::constant_time_eq(&entry.key, key))
        else {
            return false;
        };
        if let Some(holder) = entries.swap_remove(index).holder {
            holder.notify_one();
        }
        true
    }

    fn remove(&self, key: &str) {
        self.entries().retain(|entry| entry.key != key);
    }

    fn start_countdown(&self, key: &str) {
        let expires = Instant::now() + self.lifetime;
        if let Some(entry) = self.entries().iter_mut().find(|entry| entry.key == key) {
            entry.holder = None;
            entry.expires = Some(expires);
        }
    }
}

/// Key of an ongoing connection. Dropping it (ending the connection) sets the expiry of the last key
pub struct Lease<'a> {
    keys: &'a ResumeKeys,
    key: String,
    /// Another connection reconnected with the key handed to this connection
    superseded: Arc<Notify>,
}

impl Lease<'_> {
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Replaces the key with a new one and invalidates the old one immediately. Returns the new key
    pub fn rotate(&mut self) -> &str {
        let next = self.keys.add(&self.superseded);
        self.keys.remove(&self.key);
        self.key = next;
        &self.key
    }

    /// Waits until another connection reconnects with the key handed to this connection
    pub async fn superseded(&self) {
        self.superseded.notified().await;
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.keys.start_countdown(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFETIME: Duration = Duration::from_secs(60);

    #[test]
    fn redeems_a_key_only_once() {
        let keys = ResumeKeys::new(LIFETIME);
        let key = keys.issue().key().to_owned();
        assert!(keys.redeem(&key));
        assert!(!keys.redeem(&key));
    }

    #[tokio::test]
    async fn tells_the_old_connection_when_its_key_is_used() {
        // A page may reconnect before the PC notices the previous connection is gone
        let keys = ResumeKeys::new(LIFETIME);
        let lease = keys.issue();
        assert!(keys.redeem(lease.key()));
        tokio::time::timeout(Duration::from_secs(1), lease.superseded())
            .await
            .expect("古い接続に知らせる");
        drop(lease);
        assert!(keys.entries().is_empty(), "使った鍵は切断後も戻らない");
    }

    #[test]
    fn rotating_invalidates_the_old_key() {
        let keys = ResumeKeys::new(LIFETIME);
        let mut lease = keys.issue();
        let old = lease.key().to_owned();
        let new = lease.rotate().to_owned();
        assert_ne!(old, new);
        assert!(!keys.redeem(&old));
        assert!(keys.redeem(&new));
    }

    #[test]
    fn expires_after_the_lifetime() {
        let keys = ResumeKeys::new(Duration::ZERO);
        let key = keys.issue().key().to_owned();
        assert!(!keys.redeem(&key));
        assert!(keys.entries().is_empty(), "期限の切れた鍵は捨てる");
    }

    #[test]
    fn rejects_unknown_keys() {
        let keys = ResumeKeys::new(LIFETIME);
        let _lease = keys.issue();
        assert!(!keys.redeem("unknown"));
    }
}
