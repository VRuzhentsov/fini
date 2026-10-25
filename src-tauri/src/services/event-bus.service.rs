//! The event bus (ADR-0010 D8): one `tokio::sync::broadcast` of `Event`.
//! A service publishes when its state changes; the services that need the
//! change, and the UI (D9), subscribe. The publisher does not know its
//! subscribers.

use tokio::sync::broadcast;

use crate::models::Event;

/// How many events a slow subscriber can fall behind before it gets
/// `Lagged`. A subscriber that gets `Lagged` reads the current state again.
const CAPACITY: usize = 256;

pub struct EventBusService {
    sender: broadcast::Sender<Event>,
}

impl Default for EventBusService {
    fn default() -> Self {
        Self { sender: broadcast::channel(CAPACITY).0 }
    }
}

impl EventBusService {
    /// Tells every subscriber about `event`. With no subscriber, the event
    /// goes nowhere, which is correct: nobody waits for it.
    pub fn publish(&self, event: Event) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}
