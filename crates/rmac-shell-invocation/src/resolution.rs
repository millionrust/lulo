use std::fmt;

use crate::{SeatId, SeatInventory};

#[derive(Clone, Eq, PartialEq)]
pub struct Invocation {
    output: rmac_compositor::OutputId,
    seat: SeatId,
    restore_window: Option<rmac_compositor::WindowId>,
}

impl Invocation {
    pub fn output(&self) -> &rmac_compositor::OutputId {
        &self.output
    }

    pub fn seat(&self) -> &SeatId {
        &self.seat
    }

    pub fn restore_window(&self) -> Option<rmac_compositor::WindowId> {
        self.restore_window
    }
}

impl fmt::Debug for Invocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Invocation")
            .field("output", &"<redacted>")
            .field("seat", &"<redacted>")
            .field("restore_window", &self.restore_window.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveError {
    NoFocusedOutput,
    OutputUnavailable,
    NoSeat,
    AmbiguousSeat,
    SeatUnavailable,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoFocusedOutput => "the compositor has no focused output",
            Self::OutputUnavailable => "the invocation output is unavailable",
            Self::NoSeat => "no complete Wayland seat is available",
            Self::AmbiguousSeat => {
                "the shortcut activation does not identify one of multiple seats"
            }
            Self::SeatUnavailable => "the invoking Wayland seat is unavailable",
        })
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a global shortcut only when the compositor has an exact focused
/// output and the Wayland host has exactly one complete seat. Multi-seat
/// sessions fail explicitly because portal shortcut activation carries no seat.
pub fn global_shortcut(
    compositor: &rmac_compositor::Snapshot,
    seats: &SeatInventory,
) -> Result<Invocation, ResolveError> {
    let output = compositor
        .focus
        .output
        .as_ref()
        .ok_or(ResolveError::NoFocusedOutput)?;
    let seat = match seats.seats.as_slice() {
        [] => return Err(ResolveError::NoSeat),
        [seat] => seat.clone(),
        _ => return Err(ResolveError::AmbiguousSeat),
    };
    capture(output, seat, compositor)
}

/// Resolve a pointer invocation whose layer host already knows the exact
/// output and seat. Both identities are revalidated against complete state.
pub fn surface_control(
    output: &rmac_compositor::OutputId,
    seat: &SeatId,
    compositor: &rmac_compositor::Snapshot,
    seats: &SeatInventory,
) -> Result<Invocation, ResolveError> {
    if !seats.exact(seat) {
        return Err(ResolveError::SeatUnavailable);
    }
    capture(output, seat.clone(), compositor)
}

fn capture(
    output: &rmac_compositor::OutputId,
    seat: SeatId,
    compositor: &rmac_compositor::Snapshot,
) -> Result<Invocation, ResolveError> {
    if !compositor
        .outputs
        .iter()
        .any(|candidate| candidate.id == *output && candidate.enabled())
    {
        return Err(ResolveError::OutputUnavailable);
    }
    let restore_window = compositor.focus.window.filter(|focused| {
        compositor
            .windows
            .iter()
            .any(|window| window.id == *focused && window.focused)
    });
    Ok(Invocation {
        output: output.clone(),
        seat,
        restore_window,
    })
}
