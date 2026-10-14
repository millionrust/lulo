//! Keeps windows in the Mac's visible frame (below the menu bar, beside the
//! Dock): new third-party windows, dragged title bars, and windows a Dock or
//! display change leaves underneath (crates/rmac-compositor/src/placement.rs).
//!
//! It runs on the background executor with its own compositor event stream,
//! so the UI thread only forwards Desktop & Dock changes. Every decision is
//! a reaction to an event; nothing polls.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use rmac_compositor::placement::{Placer, Reservation};
use rmac_compositor::{Event, State};

/// How long after windows come back from a revealed desktop the placer
/// keeps its hands off them: their way back is still in flight.
const REVEAL_SETTLE: Duration = Duration::from_millis(1000);

/// The menu bar's reservation (rmac-menubar BAR_HEIGHT).
pub const MENU_BAR_HEIGHT: f64 = 29.0;

pub enum Input {
    Compositor(Event),
    Dock(Reservation),
}

/// Whether the shell has pushed windows aside on purpose (click wallpaper
/// to reveal desktop, F11), shared between the service and the placer.
#[derive(Clone, Default)]
pub struct RevealGuard(Arc<AtomicU64>);

fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    u64::try_from(EPOCH.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl RevealGuard {
    /// Windows are being pushed to the edges.
    pub fn hold(&self) {
        self.0.store(u64::MAX, Ordering::Release);
    }

    /// Windows are on their way back.
    pub fn release(&self) {
        let until = now_ms().saturating_add(REVEAL_SETTLE.as_millis() as u64);
        self.0.store(until, Ordering::Release);
    }

    fn active(&self) -> bool {
        now_ms() < self.0.load(Ordering::Acquire)
    }
}

pub fn channel() -> (Sender<Input>, Receiver<Input>) {
    async_channel::bounded(64)
}

/// The placer's loop: apply each event, run what it asks for in order.
pub async fn run(inputs: Receiver<Input>, reveal: RevealGuard) {
    let mut state = State::default();
    let mut placer = Placer::new(Reservation {
        menu_bar: MENU_BAR_HEIGHT,
        ..Reservation::default()
    });
    while let Ok(input) = inputs.recv().await {
        let actions = match input {
            Input::Compositor(event) => {
                state.apply(event.clone());
                placer.on_event(&event, &state, reveal.active())
            }
            Input::Dock(reservation) => {
                placer.set_reservation(reservation, &state, reveal.active())
            }
        };
        for action in actions {
            if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                eprintln!(
                    "Mission Control could not place a window ({:?}): {error:?}",
                    action.kind()
                );
            }
        }
    }
}

/// Forward the placer's own compositor event stream into its inputs.
pub async fn forward(events: Receiver<Event>, inputs: Sender<Input>) {
    while let Ok(event) = events.recv().await {
        if inputs.send(Input::Compositor(event)).await.is_err() {
            return;
        }
    }
}
