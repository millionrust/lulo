//! Pure Rust iCalendar event parsing, bounded recurrence expansion, and timed grid layout.
//!
//! Expansion is synchronous and should run on a worker thread, never the UI thread.

mod ical;
mod layout;
mod recurrence;

pub use ical::{Calendar, CalendarError, Event, TimeValue, Zone};
pub use layout::{layout_day, LayoutEvent, LayoutSlot};
pub use recurrence::{expand, Occurrence};

#[cfg(test)]
mod tests;
