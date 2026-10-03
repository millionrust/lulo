//! Pure Rust iCalendar event parsing, bounded recurrence expansion, and timed grid layout.
//!
//! Expansion is synchronous and should run on a worker thread, never the UI thread.
//! TZID values use the IANA database. Custom VTIMEZONE definitions and iCalendar
//! DURATION/PERIOD/RANGE forms are not yet modelled; unsupported recurrence forms
//! return errors rather than producing a misleading schedule.

mod ical;
mod layout;
mod recurrence;

pub use ical::{Calendar, CalendarError, Event, TimeValue, Zone};
pub use layout::{layout_day, layout_week, LayoutEvent, LayoutSlot};
pub use recurrence::{expand, Occurrence};

#[cfg(test)]
mod tests;
