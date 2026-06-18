//! Auto-lock timer for vaults.
//!
//! A vault can lock itself after a period of inactivity so that a forgotten,
//! unlocked vault does not stay readable indefinitely. This module provides the
//! timer state machine ([`AutoLockTimer`]) and an async firing loop
//! ([`run_auto_lock`]) that emits a [`LockEvent`] when the timer expires; the
//! GUI handles the event by calling [`crate::Vault::lock`], which zeroizes the
//! key.
//!
//! # Security
//! This module holds **no key material**. It only decides *when* a lock should
//! happen; the actual key wipe is done by `Vault::lock`. Reducing the unlocked
//! window shrinks the chance of key material being read from a left-open vault.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

/// Why a vault lock happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// The auto-lock inactivity timer fired.
    TimedOut,
    /// The user locked the vault explicitly.
    Manual,
}

/// Emitted when a vault should lock. The consumer must perform the actual lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEvent {
    /// Name of the vault to lock.
    pub vault: String,
    /// Why the lock is being requested.
    pub reason: LockReason,
}

/// Inactivity auto-lock timer.
///
/// Pure state: it does not lock anything itself. Call [`touch`] on every vault
/// interaction to reset the countdown. A `duration` of `None` disables
/// auto-lock (manual override).
///
/// [`touch`]: AutoLockTimer::touch
#[derive(Debug)]
pub struct AutoLockTimer {
    duration: Option<Duration>,
    last_activity: Instant,
}

impl AutoLockTimer {
    /// Create a timer with the given inactivity `duration`. `None` disables
    /// auto-lock.
    #[must_use]
    pub fn new(duration: Option<Duration>) -> Self {
        Self {
            duration,
            last_activity: Instant::now(),
        }
    }

    /// Create a timer from a per-vault `auto_lock_minutes` value (0 or `None`
    /// disables it).
    #[must_use]
    pub fn from_minutes(minutes: Option<u32>) -> Self {
        let duration = match minutes {
            Some(m) if m > 0 => Some(Duration::from_secs(u64::from(m) * 60)),
            _ => None,
        };
        Self::new(duration)
    }

    /// Reset the inactivity countdown. Call on any vault interaction.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Returns `true` if auto-lock is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.duration.is_some()
    }

    /// Disable auto-lock (manual override).
    pub fn disable(&mut self) {
        self.duration = None;
    }

    /// Enable or change the auto-lock duration and reset the countdown.
    pub fn set_duration(&mut self, duration: Option<Duration>) {
        self.duration = duration;
        self.touch();
    }

    /// Whether the timer has expired as of instant `now`.
    #[must_use]
    pub fn expired_at(&self, now: Instant) -> bool {
        match self.duration {
            Some(d) => now.saturating_duration_since(self.last_activity) >= d,
            None => false,
        }
    }

    /// Whether the timer has expired as of the current instant.
    #[must_use]
    pub fn expired(&self) -> bool {
        self.expired_at(Instant::now())
    }

    /// Time remaining before lock as of instant `now`, or `None` when disabled.
    #[must_use]
    pub fn remaining_at(&self, now: Instant) -> Option<Duration> {
        self.duration
            .map(|d| d.saturating_sub(now.saturating_duration_since(self.last_activity)))
    }

    /// Time remaining before lock as of the current instant.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        self.remaining_at(Instant::now())
    }
}

/// Poll interval for the async firing loop.
const POLL: Duration = Duration::from_millis(200);

/// Drive a shared [`AutoLockTimer`]: when it expires, send a [`LockEvent`] and
/// return. Touching the shared timer from elsewhere postpones the lock.
///
/// The loop also returns (without sending) if the receiver is dropped, so a
/// closed channel cleanly stops the task.
pub async fn run_auto_lock(timer: Arc<Mutex<AutoLockTimer>>, vault: String, tx: Sender<LockEvent>) {
    loop {
        // Decide how long to wait based on the current remaining time.
        let wait = {
            let guard = timer.lock().await;
            match guard.remaining() {
                None => POLL, // disabled: re-check periodically
                Some(r) if r.is_zero() => Duration::ZERO,
                Some(r) => r.min(POLL),
            }
        };

        if wait.is_zero() {
            // Re-check under lock to avoid racing a concurrent touch().
            let fire = {
                let guard = timer.lock().await;
                guard.expired()
            };
            if fire {
                let _ = tx
                    .send(LockEvent {
                        vault,
                        reason: LockReason::TimedOut,
                    })
                    .await;
                return;
            }
        }

        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(wait.max(Duration::from_millis(1))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_never_expires() {
        let t = AutoLockTimer::new(None);
        assert!(!t.is_enabled());
        assert!(!t.expired());
        assert_eq!(t.remaining(), None);
    }

    #[test]
    fn from_minutes_zero_disables() {
        assert!(!AutoLockTimer::from_minutes(Some(0)).is_enabled());
        assert!(!AutoLockTimer::from_minutes(None).is_enabled());
        assert!(AutoLockTimer::from_minutes(Some(5)).is_enabled());
    }

    #[test]
    fn expires_after_duration() {
        let t = AutoLockTimer::new(Some(Duration::from_secs(60)));
        let base = t.last_activity;
        assert!(!t.expired_at(base + Duration::from_secs(59)));
        assert!(t.expired_at(base + Duration::from_secs(60)));
        assert!(t.expired_at(base + Duration::from_secs(120)));
    }

    #[test]
    fn touch_resets_countdown() {
        let mut t = AutoLockTimer::new(Some(Duration::from_secs(60)));
        std::thread::sleep(Duration::from_millis(5));
        t.touch();
        let now = t.last_activity; // touch set last_activity to ~now
        assert!(!t.expired_at(now + Duration::from_secs(59)));
        assert!(t.expired_at(now + Duration::from_secs(60)));
    }

    #[test]
    fn remaining_counts_down() {
        let t = AutoLockTimer::new(Some(Duration::from_secs(60)));
        let base = t.last_activity;
        assert_eq!(
            t.remaining_at(base + Duration::from_secs(20)),
            Some(Duration::from_secs(40))
        );
        assert_eq!(
            t.remaining_at(base + Duration::from_secs(90)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn disable_overrides() {
        let mut t = AutoLockTimer::new(Some(Duration::from_secs(1)));
        t.disable();
        assert!(!t.is_enabled());
        assert!(!t.expired_at(t.last_activity + Duration::from_secs(100)));
    }

    #[tokio::test]
    async fn async_loop_fires_after_timeout() {
        let timer = Arc::new(Mutex::new(AutoLockTimer::new(Some(Duration::from_millis(
            150,
        )))));
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let handle = tokio::spawn(run_auto_lock(timer, "Personal".into(), tx));

        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("did not fire in time")
            .expect("channel closed");
        assert_eq!(event.vault, "Personal");
        assert_eq!(event.reason, LockReason::TimedOut);
        handle.await.expect("task panicked");
    }

    #[tokio::test]
    async fn async_loop_touch_postpones_lock() {
        let timer = Arc::new(Mutex::new(AutoLockTimer::new(Some(Duration::from_millis(
            300,
        )))));
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let handle = tokio::spawn(run_auto_lock(timer.clone(), "V".into(), tx));

        // Keep touching for ~600ms; the lock must not fire while active.
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            timer.lock().await.touch();
        }
        // No event yet.
        assert!(rx.try_recv().is_err());

        // Stop touching; it should fire within the timeout now.
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("did not fire after activity stopped")
            .expect("channel closed");
        assert_eq!(event.reason, LockReason::TimedOut);
        handle.await.expect("task panicked");
    }
}
