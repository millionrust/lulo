//! Every edit the Clock window makes, as data. The window applies a change
//! to its copy of the state at once and replays the same change inside the
//! store's locked read-modify-write, so the ring process's own updates
//! (a one-time alarm switching off, a finished timer disappearing) are
//! never overwritten.

use crate::alarms::Alarm;
use crate::countdown::Countdown;
use crate::store::{State, MAX_ALARMS, MAX_CITIES, MAX_TIMERS};

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Replace the World Clock list (first launch seeds it with the local
    /// city).
    SetCities(Vec<String>),
    AddCity(String),
    RemoveCity(String),
    /// Add or replace an alarm by id.
    SaveAlarm(Alarm),
    RemoveAlarm(u64),
    SetAlarmEnabled(u64, bool),
    StartTimer {
        id: u64,
        duration: u64,
        now: u64,
    },
    PauseTimer {
        id: u64,
        now: u64,
    },
    ResumeTimer {
        id: u64,
        now: u64,
    },
    CancelTimer(u64),
    StopwatchStart(u64),
    StopwatchStop(u64),
    StopwatchLap(u64),
    StopwatchReset,
}

impl Change {
    pub fn apply(&self, state: &mut State) {
        match self {
            Self::SetCities(cities) => {
                let mut cities = cities.clone();
                cities.dedup();
                cities.truncate(MAX_CITIES);
                state.cities = Some(cities);
            }
            Self::AddCity(name) => {
                let cities = state.cities.get_or_insert_with(Vec::new);
                if !cities.contains(name) && cities.len() < MAX_CITIES {
                    cities.push(name.clone());
                }
            }
            Self::RemoveCity(name) => {
                if let Some(cities) = state.cities.as_mut() {
                    cities.retain(|city| city != name);
                }
            }
            Self::SaveAlarm(alarm) => {
                if let Some(existing) = state.alarms.iter_mut().find(|item| item.id == alarm.id) {
                    *existing = alarm.clone();
                } else if state.alarms.len() < MAX_ALARMS {
                    state.alarms.push(alarm.clone());
                }
                state.next_id = state.next_id.max(alarm.id);
                state
                    .alarms
                    .sort_by_key(|alarm| (alarm.hour, alarm.minute, alarm.id));
            }
            Self::RemoveAlarm(id) => state.alarms.retain(|alarm| alarm.id != *id),
            Self::SetAlarmEnabled(id, enabled) => {
                if let Some(alarm) = state.alarms.iter_mut().find(|alarm| alarm.id == *id) {
                    alarm.enabled = *enabled;
                    alarm.snoozed_until = None;
                }
            }
            Self::StartTimer { id, duration, now } => {
                if *duration > 0
                    && state.timers.len() < MAX_TIMERS
                    && state.timers.iter().all(|timer| timer.id != *id)
                {
                    state.timers.push(Countdown::start(*id, *duration, *now));
                    state.next_id = state.next_id.max(*id);
                }
            }
            Self::PauseTimer { id, now } => {
                if let Some(timer) = state.timers.iter_mut().find(|timer| timer.id == *id) {
                    timer.pause(*now);
                }
            }
            Self::ResumeTimer { id, now } => {
                if let Some(timer) = state.timers.iter_mut().find(|timer| timer.id == *id) {
                    timer.resume(*now);
                }
            }
            Self::CancelTimer(id) => state.timers.retain(|timer| timer.id != *id),
            Self::StopwatchStart(now) => state.stopwatch.start(*now),
            Self::StopwatchStop(now) => state.stopwatch.stop(*now),
            Self::StopwatchLap(now) => state.stopwatch.lap(*now),
            Self::StopwatchReset => state.stopwatch.reset(),
        }
    }

    /// Whether the ring schedule may have changed.
    pub fn affects_schedule(&self) -> bool {
        matches!(
            self,
            Self::SaveAlarm(_)
                | Self::RemoveAlarm(_)
                | Self::SetAlarmEnabled(..)
                | Self::StartTimer { .. }
                | Self::PauseTimer { .. }
                | Self::ResumeTimer { .. }
                | Self::CancelTimer(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cities_add_once_and_remove() {
        let mut state = State::default();
        Change::AddCity("Tokyo".into()).apply(&mut state);
        Change::AddCity("Tokyo".into()).apply(&mut state);
        Change::AddCity("Paris".into()).apply(&mut state);
        assert_eq!(
            state.cities.as_deref(),
            Some(&["Tokyo".into(), "Paris".into()][..])
        );
        Change::RemoveCity("Tokyo".into()).apply(&mut state);
        assert_eq!(state.cities.as_deref(), Some(&["Paris".into()][..]));
    }

    #[test]
    fn alarms_save_sort_toggle_and_remove() {
        let mut state = State::default();
        let alarm = |id, hour| Alarm {
            id,
            hour,
            ..Alarm::default()
        };
        Change::SaveAlarm(alarm(1, 9)).apply(&mut state);
        Change::SaveAlarm(alarm(2, 6)).apply(&mut state);
        assert_eq!(
            state
                .alarms
                .iter()
                .map(|alarm| alarm.id)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        Change::SaveAlarm(alarm(1, 5)).apply(&mut state); // edit moves it first
        assert_eq!(state.alarms[0].id, 1);
        assert_eq!(state.alarms.len(), 2);
        state.alarms[0].snoozed_until = Some(10);
        Change::SetAlarmEnabled(1, false).apply(&mut state);
        assert!(!state.alarms[0].enabled);
        assert_eq!(state.alarms[0].snoozed_until, None);
        Change::RemoveAlarm(1).apply(&mut state);
        assert_eq!(state.alarms.len(), 1);
        assert_eq!(state.allocate_id(), 3);
    }

    #[test]
    fn timers_start_pause_resume_cancel() {
        let mut state = State::default();
        Change::StartTimer {
            id: 4,
            duration: 0,
            now: 0,
        }
        .apply(&mut state);
        assert!(state.timers.is_empty()); // zero-length timers never start
        Change::StartTimer {
            id: 4,
            duration: 60_000,
            now: 1_000,
        }
        .apply(&mut state);
        Change::PauseTimer { id: 4, now: 31_000 }.apply(&mut state);
        assert_eq!(state.timers[0].remaining(90_000), 30_000);
        Change::ResumeTimer { id: 4, now: 40_000 }.apply(&mut state);
        assert_eq!(state.timers[0].ends_at(), Some(70_000));
        Change::CancelTimer(4).apply(&mut state);
        assert!(state.timers.is_empty());
    }

    #[test]
    fn stopwatch_changes_and_schedule_relevance() {
        let mut state = State::default();
        Change::StopwatchStart(0).apply(&mut state);
        Change::StopwatchLap(1_000).apply(&mut state);
        Change::StopwatchStop(2_000).apply(&mut state);
        assert_eq!(state.stopwatch.elapsed(9_000), 2_000);
        assert_eq!(state.stopwatch.laps, [1_000]);
        Change::StopwatchReset.apply(&mut state);
        assert_eq!(state.stopwatch.elapsed(9_000), 0);
        assert!(!Change::StopwatchReset.affects_schedule());
        assert!(Change::CancelTimer(1).affects_schedule());
    }
}
