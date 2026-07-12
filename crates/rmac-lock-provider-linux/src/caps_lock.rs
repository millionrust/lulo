//! Aggregate redacted keyboard focus and Caps Lock state across seats.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Default)]
pub(crate) struct CapsLockState {
    seats: BTreeMap<u32, SeatState>,
    caps_lock_active: bool,
    keyboard_focused: bool,
}

#[derive(Clone, Copy, Default)]
struct SeatState {
    focused: bool,
    locked: bool,
}

impl CapsLockState {
    pub(crate) fn add_seat(&mut self, seat: u32) -> Result<(), Error> {
        if self.seats.insert(seat, SeatState::default()).is_some() {
            return Err(Error::DuplicateSeat);
        }
        Ok(())
    }

    pub(crate) fn remove_seat(&mut self, seat: u32) -> Result<InputChange, Error> {
        self.seats.remove(&seat).ok_or(Error::SeatMissing)?;
        Ok(self.reconcile())
    }

    pub(crate) fn set_focused(&mut self, seat: u32, focused: bool) -> Result<InputChange, Error> {
        let state = self.seats.get_mut(&seat).ok_or(Error::SeatMissing)?;
        state.focused = focused;
        Ok(self.reconcile())
    }

    pub(crate) fn set_locked(&mut self, seat: u32, locked: bool) -> Result<InputChange, Error> {
        let state = self.seats.get_mut(&seat).ok_or(Error::SeatMissing)?;
        state.locked = locked;
        Ok(self.reconcile())
    }

    fn reconcile(&mut self) -> InputChange {
        let caps_lock_active = self
            .seats
            .values()
            .any(|state| state.focused && state.locked);
        let keyboard_focused = self.seats.values().any(|state| state.focused);
        let change = InputChange {
            caps_lock: (caps_lock_active != self.caps_lock_active).then_some(caps_lock_active),
            keyboard_focus: (keyboard_focused != self.keyboard_focused).then_some(keyboard_focused),
        };
        self.caps_lock_active = caps_lock_active;
        self.keyboard_focused = keyboard_focused;
        change
    }
}

impl fmt::Debug for CapsLockState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapsLockState")
            .field("caps_lock_active", &self.caps_lock_active)
            .field("keyboard_focused", &self.keyboard_focused)
            .field("seats", &self.seats.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InputChange {
    pub(crate) caps_lock: Option<bool>,
    pub(crate) keyboard_focus: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    DuplicateSeat,
    SeatMissing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_focused_locked_seat_activates_the_indicator() {
        let mut state = CapsLockState::default();
        state.add_seat(1).unwrap();
        assert_eq!(state.set_locked(1, true), Ok(InputChange::default()));
        assert_eq!(
            state.set_focused(1, true),
            Ok(InputChange {
                caps_lock: Some(true),
                keyboard_focus: Some(true),
            })
        );
        assert_eq!(state.set_focused(1, true), Ok(InputChange::default()));
        assert_eq!(
            state.set_focused(1, false),
            Ok(InputChange {
                caps_lock: Some(false),
                keyboard_focus: Some(false),
            })
        );
    }

    #[test]
    fn another_focused_locked_seat_prevents_a_false_transition() {
        let mut state = CapsLockState::default();
        state.add_seat(1).unwrap();
        state.add_seat(2).unwrap();
        state.set_locked(1, true).unwrap();
        state.set_locked(2, true).unwrap();
        assert_eq!(
            state.set_focused(1, true),
            Ok(InputChange {
                caps_lock: Some(true),
                keyboard_focus: Some(true),
            })
        );
        assert_eq!(state.set_focused(2, true), Ok(InputChange::default()));
        assert_eq!(state.remove_seat(1), Ok(InputChange::default()));
        assert_eq!(
            state.set_locked(2, false),
            Ok(InputChange {
                caps_lock: Some(false),
                keyboard_focus: None,
            })
        );
    }

    #[test]
    fn rejects_unknown_or_duplicate_seats() {
        let mut state = CapsLockState::default();
        state.add_seat(7).unwrap();
        assert_eq!(state.add_seat(7), Err(Error::DuplicateSeat));
        assert_eq!(state.set_focused(8, true), Err(Error::SeatMissing));
        assert_eq!(state.set_locked(8, true), Err(Error::SeatMissing));
        assert_eq!(state.remove_seat(8), Err(Error::SeatMissing));
        assert_eq!(
            format!("{state:?}"),
            "CapsLockState { caps_lock_active: false, keyboard_focused: false, seats: 1 }"
        );
    }
}
