use crate::{Error, Snapshot};

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Initial state, a host change, or the first state after reconnection.
    Snapshot(Snapshot),
    /// The source disappeared. Consumers retain their last known-good visual
    /// state while presenting this service status separately.
    Unavailable(Error),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppearanceState {
    pub snapshot: Snapshot,
    pub source_error: Option<Error>,
}

impl AppearanceState {
    /// Apply one adapter event. Returns whether visible state changed.
    pub fn apply(&mut self, event: Event) -> bool {
        let previous = self.clone();
        match event {
            Event::Snapshot(snapshot) => {
                self.snapshot = snapshot;
                self.source_error = None;
            }
            Event::Unavailable(error) => {
                self.source_error = Some(error);
            }
        }
        *self != previous
    }
}
