//! Domain events and a simple synchronous event bus (observer pattern).

use crate::{Language, Variable};
use std::sync::{Arc, Mutex};

/// Something that happened in the domain. UI views subscribe to these.
#[derive(Debug, Clone, PartialEq)]
pub enum DomainEvent {
    /// A variable was created or changed.
    VariableChanged(Variable),
    /// A variable was removed.
    VariableRemoved {
        /// Owning language.
        language: Language,
        /// Variable name.
        name: String,
    },
    /// All variables of a language were cleared (e.g. kernel restart).
    WorkspaceCleared(Option<Language>),
    /// A kernel changed status.
    KernelStatus {
        /// Kernel language.
        language: Language,
        /// New status label.
        status: String,
    },
    /// A plugin changed state.
    PluginState {
        /// Plugin id.
        id: String,
        /// New state label.
        state: String,
    },
    /// Debugger state changed.
    DebugState(String),
}

/// Subscriber callback.
pub type Subscriber = Box<dyn Fn(&DomainEvent) + Send + Sync>;

/// Publish/subscribe port. Adapters (UI, logging, plugins) implement or consume it.
pub trait EventBus: Send + Sync {
    /// Publish an event to all subscribers.
    fn publish(&self, event: DomainEvent);
    /// Register a subscriber.
    fn subscribe(&self, subscriber: Subscriber);
}

/// Thread-safe, synchronous in-memory event bus.
#[derive(Default, Clone)]
pub struct InMemoryEventBus {
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
}

impl InMemoryEventBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventBus for InMemoryEventBus {
    fn publish(&self, event: DomainEvent) {
        let subs = self.subscribers.lock().expect("event bus poisoned");
        for s in subs.iter() {
            s(&event);
        }
    }

    fn subscribe(&self, subscriber: Subscriber) {
        self.subscribers
            .lock()
            .expect("event bus poisoned")
            .push(subscriber);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn delivers_to_all_subscribers() {
        let bus = InMemoryEventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let c = count.clone();
            bus.subscribe(Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            }));
        }
        bus.publish(DomainEvent::WorkspaceCleared(None));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
