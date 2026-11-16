//! Hands presses of the canvas's Send to AI to MCP's wait_for_canvas.
//!
//! When the user finishes writing on the canvas page, they press Send to AI and the bridge remembers it (press).
//! MCP's wait_for_canvas (long polling) waits for a press and returns the changes with the page that pressed.
//! One press goes to only one of the waiting wait_for_canvas calls (so that two AIs waiting at the same
//! time do not both start on the same instruction). pending is taken with take(), so only one of the
//! calls woken at the same time receives it and the others keep waiting.
//! A press made while no AI is waiting is remembered, and the next wait returns right away.
//! Any number of presses before one is picked up are merged into one (the page that pressed last).
//!
//! After a wait returns without a press, the AI is still treated as waiting for a grace period
//! (config::AI_WAITING_GRACE), so that the page's icon does not stop pulsing and flicker in the few seconds
//! between the AI receiving "not yet" and calling again. No grace period after a press is picked up
//! (the AI is working, so the pulsing stops right away).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use crate::lock::lock;

/// The state sent to the pages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiStatus {
    /// The AI is waiting in wait_for_canvas (also true during the grace period after a timeout)
    pub waiting: bool,
    /// Pressed, but no wait_for_canvas has picked it up yet
    pub queued: bool,
}

struct State {
    /// The connection number of the page that pressed. The last press (overwrites an earlier one = merged into one)
    pending: Option<u64>,
    /// The number of wait calls waiting
    waiting: usize,
    /// When the grace period after returning without a press ends
    grace_until: Option<Instant>,
}

/// A guard that adds 1 to waiting while it lives.
/// It always undoes it, also when the wait call is dropped midway (the AI cancelled the call)
struct WaitingGuard<'a> {
    inbox: &'a AiInbox,
}

impl<'a> WaitingGuard<'a> {
    fn new(inbox: &'a AiInbox) -> Self {
        lock(&inbox.state).waiting += 1;
        Self { inbox }
    }
}

impl Drop for WaitingGuard<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.inbox.state);
        state.waiting -= 1;
        self.inbox.distribute_locked(&state);
    }
}

#[derive(Clone)]
pub struct AiInbox {
    state: Arc<Mutex<State>>,
    /// Wakes a waiting call on a press
    pressed: Arc<Notify>,
    /// Sent to the pages
    status: Arc<watch::Sender<AiStatus>>,
    /// The grace period after returning without a press
    grace: Duration,
}

impl AiInbox {
    pub fn new(grace: Duration) -> Self {
        let state = Arc::new(Mutex::new(State {
            pending: None,
            waiting: 0,
            grace_until: None,
        }));
        let (status, _) = watch::channel(current_status(&lock(&state)));
        Self {
            state,
            pressed: Arc::new(Notify::new()),
            status: Arc::new(status),
            grace,
        }
    }

    /// A page pressed Send to AI (connection. Called from the sync in webtransport.rs)
    pub fn press(&self, connection: u64) {
        let mut state = lock(&self.state);
        state.pending = Some(connection);
        self.distribute_locked(&state);
        // notify_one keeps a notification as a permit when no one is waiting,
        // so a press made before the waiter creates notified() is not missed
        self.pressed.notify_one();
    }

    /// Waits for a press. Returns the connection number of the page that pressed, or None after timeout
    pub async fn wait(&self, timeout: Duration) -> Option<u64> {
        let guard = WaitingGuard::new(self);
        self.distribute();
        // Even when woken again (another call took the press, etc.), up to timeout in total
        let deadline = Instant::now() + timeout;
        loop {
            // Create notified() before checking pending (the notify_one permit is picked up when
            // the created notified() starts waiting, so nothing is missed)
            let pressed = self.pressed.notified();
            // A lock() guard created in an if let condition lives until the end of the then block, so to avoid
            // deadlocking by locking the same Mutex again, take pending and release the lock before deciding
            let pending = {
                let mut state = lock(&self.state);
                let pending = state.pending.take();
                if pending.is_some() {
                    // No grace period after picking up a press (the AI is working, so the pulsing stops right away)
                    state.grace_until = None;
                }
                pending
            };
            if let Some(connection) = pending {
                // Dropping the guard takes 1 off waiting and sends the state. The page stops pulsing
                std::mem::drop(guard);
                return Some(connection);
            }
            tokio::select! {
                _ = pressed => {}
                _ = tokio::time::sleep_until(deadline) => {
                    // Start the grace period before dropping the guard (so that waiting does not
                    // flicker to false for a moment when the guard sends the state)
                    self.start_grace();
                    std::mem::drop(guard);
                    return None;
                }
            }
        }
    }

    /// The receiver for the pages
    pub fn subscribe(&self) -> watch::Receiver<AiStatus> {
        self.status.subscribe()
    }

    /// Returned without a press, so treat the AI as waiting during the grace period.
    /// When it ends, send the state again (waiting becomes false)
    fn start_grace(&self) {
        let until = {
            let mut state = lock(&self.state);
            let until = Instant::now() + self.grace;
            state.grace_until = Some(until);
            self.distribute_locked(&state);
            until
        };
        let state = Arc::clone(&self.state);
        let status = Arc::clone(&self.status);
        tokio::spawn(async move {
            tokio::time::sleep_until(until).await;
            let value = current_status(&lock(&state));
            let _ = status.send_if_modified(|last| {
                let changed = *last != value;
                *last = value;
                changed
            });
        });
    }

    /// Computes the state and sends it to the pages (only when the value changed)
    fn distribute(&self) {
        let state = lock(&self.state);
        self.distribute_locked(&state);
    }

    fn distribute_locked(&self, state: &State) {
        let status = current_status(state);
        let _ = self.status.send_if_modified(|last| {
            let changed = *last != status;
            *last = status;
            changed
        });
    }
}

/// The state is computed in one place: waiting if a call is waiting or the grace period has not ended
fn current_status(state: &State) -> AiStatus {
    AiStatus {
        waiting: state.waiting > 0
            || state
                .grace_until
                .is_some_and(|until| until > Instant::now()),
        queued: state.pending.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Waits until the state becomes expected (fails after 1 second)
    async fn wait_for_status(inbox: &AiInbox, expected: AiStatus) {
        let mut status = inbox.subscribe();
        tokio::time::timeout(Duration::from_secs(1), async {
            while *status.borrow_and_update() != expected {
                let _ = status.changed().await;
            }
        })
        .await
        .expect("状態が期待どおりにならない");
    }

    #[tokio::test]
    async fn press_without_waiter_is_queued_and_wait_picks_it_up() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        assert_eq!(*inbox.subscribe().borrow_and_update(), AiStatus::default());

        inbox.press(2);
        wait_for_status(
            &inbox,
            AiStatus {
                waiting: false,
                queued: true,
            },
        )
        .await;

        // Returns the pressed number right away even though no one was waiting
        assert_eq!(
            tokio::time::timeout(
                Duration::from_millis(50),
                inbox.wait(Duration::from_secs(10))
            )
            .await
            .unwrap(),
            Some(2)
        );
        wait_for_status(&inbox, AiStatus::default()).await;
    }

    #[tokio::test]
    async fn press_while_waiting_is_delivered() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        let waiting = inbox.clone();
        let waiter = tokio::spawn(async move { waiting.wait(Duration::from_secs(10)).await });
        wait_for_status(
            &inbox,
            AiStatus {
                waiting: true,
                queued: false,
            },
        )
        .await;

        inbox.press(5);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap(),
            Some(5)
        );
    }

    #[tokio::test]
    async fn presses_are_coalesced_into_one() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        inbox.press(1);
        inbox.press(2);

        assert_eq!(inbox.wait(Duration::from_secs(1)).await, Some(2));
        // There is no second press, so the next wait times out with None
        assert_eq!(inbox.wait(Duration::from_millis(20)).await, None);
    }

    #[tokio::test]
    async fn one_press_wakes_only_one_of_the_waits() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        let first_wait = inbox.clone();
        let second_wait = inbox.clone();
        let first = tokio::spawn(async move { first_wait.wait(Duration::from_secs(10)).await });
        let second =
            tokio::spawn(async move { second_wait.wait(Duration::from_millis(200)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        inbox.press(9);

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), first)
                .await
                .unwrap()
                .unwrap(),
            Some(9)
        );
        // Only one receives the press; the other times out with None
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .unwrap()
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn waiting_is_true_while_waiting_and_false_right_after_delivery() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        let waiting = inbox.clone();
        let waiter = tokio::spawn(async move { waiting.wait(Duration::from_secs(10)).await });
        wait_for_status(
            &inbox,
            AiStatus {
                waiting: true,
                queued: false,
            },
        )
        .await;

        inbox.press(3);
        // After returning with a press, false right away without a grace period
        wait_for_status(&inbox, AiStatus::default()).await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap(),
            Some(3)
        );
    }

    #[tokio::test]
    async fn grace_keeps_waiting_true_until_it_expires() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        assert_eq!(inbox.wait(Duration::from_millis(10)).await, None);

        // Stays true during the grace period
        wait_for_status(
            &inbox,
            AiStatus {
                waiting: true,
                queued: false,
            },
        )
        .await;
        // False once the grace period ends
        wait_for_status(&inbox, AiStatus::default()).await;
    }

    #[tokio::test]
    async fn dropping_wait_returns_the_waiting_count() {
        let inbox = AiInbox::new(Duration::from_millis(50));
        let waiting = inbox.clone();
        let dropped = tokio::spawn(async move { waiting.wait(Duration::from_secs(10)).await });
        wait_for_status(
            &inbox,
            AiStatus {
                waiting: true,
                queued: false,
            },
        )
        .await;

        dropped.abort();
        wait_for_status(&inbox, AiStatus::default()).await;
    }
}
