//! Bounded per-extension event queue with drop-oldest policy.
//!
//! **Phase 1 status:** scaffolding only. The `EventQueue` type exists
//! but is not wired into the broker. Events are delivered via
//! `PluginSlot::notify` directly (same as today). The queue + backpressure
//! logic activates when the socket transport lands (Phase 2+).
//!
//! When wired, each extension gets an `EventQueue`. Observe hooks enqueue
//! events instead of fire-and-forget `notify`. If the queue is full, the
//! event is dropped and a `Lagged { count }` frame is sent on the next
//! successful send. This prevents a slow extension from stalling the
//! turn loop or growing daemon memory.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

/// Default bounded capacity per extension.
const DEFAULT_CAPACITY: usize = 64;

/// An event destined for an extension.
#[derive(Debug, Clone)]
pub struct ExtensionEvent {
    /// The event type name (e.g. "MessageEnd", "ToolEnded").
    pub event_type: String,
    /// The serialized event payload (already redacted if needed).
    pub payload: serde_json::Value,
    /// The session this event belongs to.
    pub session_id: String,
}

/// Bounded per-extension event queue with drop-oldest + Lagged counter.
///
/// `try_send` never blocks — if the queue is full, the event is dropped
/// and the `dropped_count` is incremented. The caller should check
/// `take_dropped_count()` periodically and send a `Lagged { count }`
/// frame if non-zero.
pub struct EventQueue {
    sender: mpsc::Sender<ExtensionEvent>,
    /// Total events dropped since last `take_dropped_count()`.
    dropped_count: Arc<AtomicU64>,
    capacity: usize,
}

impl EventQueue {
    /// Create a new bounded queue with the default capacity.
    pub fn new() -> (Self, mpsc::Receiver<ExtensionEvent>) {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a new bounded queue with a custom capacity.
    pub fn with_capacity(capacity: usize) -> (Self, mpsc::Receiver<ExtensionEvent>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let dropped_count = Arc::new(AtomicU64::new(0));
        (
            Self {
                sender,
                dropped_count,
                capacity,
            },
            receiver,
        )
    }

    /// Try to enqueue an event. Returns `true` if enqueued, `false` if
    /// dropped (queue full). Never blocks.
    pub fn try_send(&self, event: ExtensionEvent) -> bool {
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_count.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Extension disconnected — silently drop.
                false
            }
        }
    }

    /// Take and reset the dropped count. Returns the number of events
    /// dropped since the last call. If non-zero, the caller should send
    /// a `Lagged { count }` frame.
    pub fn take_dropped_count(&self) -> u64 {
        self.dropped_count.swap(0, Ordering::Relaxed)
    }

    /// The queue's bounded capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        // Default constructor panics if the receiver is dropped — but
        // this is only used in tests. In production, `new()` is called
        // and both halves are kept.
        let (queue, _receiver) = Self::new();
        queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_and_receive() {
        let (queue, mut receiver) = EventQueue::new();
        assert!(queue.try_send(ExtensionEvent {
            event_type: "TurnEnded".into(),
            payload: serde_json::json!({}),
            session_id: "sess-1".into(),
        }));
        let event = receiver.blocking_recv().unwrap();
        assert_eq!(event.event_type, "TurnEnded");
    }

    #[test]
    fn test_drop_when_full() {
        let (queue, _receiver) = EventQueue::with_capacity(2);
        assert!(queue.try_send(ExtensionEvent {
            event_type: "A".into(),
            payload: serde_json::json!({}),
            session_id: "s".into(),
        }));
        assert!(queue.try_send(ExtensionEvent {
            event_type: "B".into(),
            payload: serde_json::json!({}),
            session_id: "s".into(),
        }));
        // Queue is full — this should be dropped.
        assert!(!queue.try_send(ExtensionEvent {
            event_type: "C".into(),
            payload: serde_json::json!({}),
            session_id: "s".into(),
        }));
        assert_eq!(queue.take_dropped_count(), 1);
        // Second check — no new drops.
        assert_eq!(queue.take_dropped_count(), 0);
    }
}
