//! Platform-neutral appearance state and event contracts.
//!
//! This crate deliberately has no GPUI, D-Bus, portal, or platform-FFI
//! dependency. Platform adapters implement [`AppearanceSource`], while apps and
//! shell surfaces consume complete snapshots and reduce events through
//! [`AppearanceState`].

use async_channel::Sender;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorScheme {
    #[default]
    NoPreference,
    PreferDark,
    PreferLight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResolvedColorScheme {
    #[default]
    Light,
    Dark,
}

impl ColorScheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoPreference => "Automatic",
            Self::PreferDark => "Dark",
            Self::PreferLight => "Light",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Contrast {
    #[default]
    Normal,
    Higher,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPreference {
    #[default]
    Full,
    Reduced,
}

/// An sRGB accent color normalized to the inclusive 0–1 range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccentColor {
    red: f64,
    green: f64,
    blue: f64,
}

impl AccentColor {
    pub fn new(red: f64, green: f64, blue: f64) -> Option<Self> {
        [red, green, blue]
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            .then_some(Self { red, green, blue })
    }

    pub fn red(self) -> f64 {
        self.red
    }

    pub fn green(self) -> f64 {
        self.green
    }

    pub fn blue(self) -> f64 {
        self.blue
    }

    pub fn components(self) -> (f64, f64, f64) {
        (self.red, self.green, self.blue)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub color_scheme: bool,
    pub accent_color: bool,
    pub contrast: bool,
    pub reduced_motion: bool,
}

impl Capabilities {
    pub fn any(self) -> bool {
        self.color_scheme || self.accent_color || self.contrast || self.reduced_motion
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub color_scheme: ColorScheme,
    pub accent_color: Option<AccentColor>,
    pub contrast: Contrast,
    pub motion: MotionPreference,
    pub capabilities: Capabilities,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedAppearance {
    pub color_scheme: ResolvedColorScheme,
    pub accent_color: AccentColor,
    pub contrast: Contrast,
    pub motion: MotionPreference,
}

impl Snapshot {
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::default()
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: String,
    detail: String,
}

impl Error {
    pub fn new(operation: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Asynchronous appearance source used by apps and shell surfaces.
pub trait AppearanceSource: Send + Sync {
    fn snapshot(&self) -> SourceFuture<'_, Snapshot>;

    /// Forward source events until the consumer closes `events` or a terminal
    /// source error occurs. Reconnectable adapters should stay in this future
    /// and publish [`Event::Unavailable`] between successful snapshots.
    fn watch(&self, events: Sender<Event>) -> SourceFuture<'_, ()>;
}

/// Deterministic source for application and component tests.
#[derive(Clone)]
pub struct FakeAppearanceSource {
    state: Arc<Mutex<Snapshot>>,
    events: async_channel::Receiver<Event>,
}

#[derive(Clone)]
pub struct FakeAppearanceController {
    state: Arc<Mutex<Snapshot>>,
    events: async_channel::Sender<Event>,
}

impl FakeAppearanceSource {
    pub fn new(snapshot: Snapshot) -> (Self, FakeAppearanceController) {
        let state = Arc::new(Mutex::new(snapshot));
        let (events_tx, events_rx) = async_channel::unbounded();
        (
            Self {
                state: Arc::clone(&state),
                events: events_rx,
            },
            FakeAppearanceController {
                state,
                events: events_tx,
            },
        )
    }

    pub fn current(&self) -> Snapshot {
        self.state.lock().expect("fake state lock poisoned").clone()
    }
}

impl FakeAppearanceController {
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        *self.state.lock().expect("fake state lock poisoned") = snapshot.clone();
        let _ = self.events.try_send(Event::Snapshot(snapshot));
    }

    pub fn set_unavailable(&self, error: Error) {
        let _ = self.events.try_send(Event::Unavailable(error));
    }
}

impl AppearanceSource for FakeAppearanceSource {
    fn snapshot(&self) -> SourceFuture<'_, Snapshot> {
        Box::pin(async { Ok(self.current()) })
    }

    fn watch(&self, events: Sender<Event>) -> SourceFuture<'_, ()> {
        Box::pin(async move {
            while let Ok(event) = self.events.recv().await {
                if events.send(event).await.is_err() {
                    return Ok(());
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark_snapshot() -> Snapshot {
        Snapshot {
            available: true,
            color_scheme: ColorScheme::PreferDark,
            accent_color: AccentColor::new(0.0, 0.48, 1.0),
            contrast: Contrast::Higher,
            motion: MotionPreference::Reduced,
            capabilities: Capabilities {
                color_scheme: true,
                accent_color: true,
                contrast: true,
                reduced_motion: true,
            },
            detail: None,
        }
    }

    #[test]
    fn accent_color_rejects_non_finite_and_out_of_range_components() {
        assert!(AccentColor::new(0.0, 0.5, 1.0).is_some());
        assert!(AccentColor::new(-0.1, 0.5, 1.0).is_none());
        assert!(AccentColor::new(0.0, 1.1, 1.0).is_none());
        assert!(AccentColor::new(0.0, f64::NAN, 1.0).is_none());
    }

    #[test]
    fn state_keeps_last_known_good_snapshot_during_source_loss() {
        let snapshot = dark_snapshot();
        let mut state = AppearanceState::default();
        assert!(state.apply(Event::Snapshot(snapshot.clone())));
        assert!(state.apply(Event::Unavailable(Error::new(
            "watch appearance settings",
            "portal stopped"
        ))));
        assert_eq!(state.snapshot, snapshot);
        assert!(state.source_error.is_some());
    }

    #[test]
    fn repeated_events_do_not_request_an_unnecessary_redraw() {
        let snapshot = dark_snapshot();
        let mut state = AppearanceState {
            snapshot: snapshot.clone(),
            source_error: None,
        };
        assert!(!state.apply(Event::Snapshot(snapshot)));
    }

    #[test]
    fn fake_controller_updates_authoritative_state() {
        let (source, controller) = FakeAppearanceSource::new(Snapshot::default());
        let snapshot = dark_snapshot();
        controller.set_snapshot(snapshot.clone());
        assert_eq!(source.current(), snapshot);
    }
}
