//! Aggregate redacted Caps Lock state for one or more keyboard seats.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Default)]
pub(crate) struct CapsLockState {
    seats: BTreeMap<u32, SeatState>,
    active: bool,
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

    pub(crate) fn remove_seat(&mut self, seat: u32) -> Result<Option<bool>, Error> {
        self.seats.remove(&seat).ok_or(Error::SeatMissing)?;
        Ok(self.reconcile())
    }

    pub(crate) fn set_focused(&mut self, seat: u32, focused: bool) -> Result<Option<bool>, Error> {
        let state = self.seats.get_mut(&seat).ok_or(Error::SeatMissing)?;
        state.focused = focused;
        Ok(self.reconcile())
    }

    pub(crate) fn set_locked(&mut self, seat: u32, locked: bool) -> Result<Option<bool>, Error> {
        let state = self.seats.get_mut(&seat).ok_or(Error::SeatMissing)?;
        state.locked = locked;
        Ok(self.reconcile())
    }

    fn reconcile(&mut self) -> Option<bool> {
        let active = self
            .seats
            .values()
            .any(|state| state.focused && state.locked);
        if active == self.active {
            None
        } else {
            self.active = active;
            Some(active)
        }
    }
}

impl fmt::Debug for CapsLockState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapsLockState")
            .field("active", &self.active)
            .field("seats", &self.seats.len())
            .finish()
    }
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
        assert_eq!(state.set_locked(1, true), Ok(None));
        assert_eq!(state.set_focused(1, true), Ok(Some(true)));
        assert_eq!(state.set_focused(1, true), Ok(None));
        assert_eq!(state.set_focused(1, false), Ok(Some(false)));
    }

    #[test]
    fn another_focused_locked_seat_prevents_a_false_transition() {
        let mut state = CapsLockState::default();
        state.add_seat(1).unwrap();
        state.add_seat(2).unwrap();
        state.set_locked(1, true).unwrap();
        state.set_locked(2, true).unwrap();
        assert_eq!(state.set_focused(1, true), Ok(Some(true)));
        assert_eq!(state.set_focused(2, true), Ok(None));
        assert_eq!(state.remove_seat(1), Ok(None));
        assert_eq!(state.set_locked(2, false), Ok(Some(false)));
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
            "CapsLockState { active: false, seats: 1 }"
        );
    }
}
