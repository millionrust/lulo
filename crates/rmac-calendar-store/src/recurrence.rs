use crate::ical::{Calendar, CalendarError, Event, TimeValue, Zone};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use rrule::{RRule, RRuleSet, Unvalidated};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub uid: String,
    /// Original start of this instance, even if a detached VEVENT moved it.
    pub recurrence_id: DateTime<Utc>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub summary: String,
    pub all_day: bool,
}

/// Expand occurrences intersecting the half-open UTC window. At most `limit` results are
/// returned; exceeding it is an error so callers cannot silently miss events.
pub fn expand(
    calendar: &Calendar,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    floating_zone: Tz,
    limit: usize,
) -> Result<Vec<Occurrence>, CalendarError> {
    if window_end <= window_start || limit == 0 || limit > 10_000 {
        return Err(CalendarError("invalid expansion window or limit".into()));
    }
    let mut masters: HashMap<&str, &Event> = HashMap::new();
    let mut overrides: HashMap<&str, Vec<&Event>> = HashMap::new();
    for event in &calendar.events {
        if event.recurrence_id.is_some() {
            overrides.entry(&event.uid).or_default().push(event);
        } else if masters.insert(&event.uid, event).is_some() {
            return Err(CalendarError(format!(
                "duplicate master UID: {}",
                event.uid
            )));
        }
    }
    let mut result = Vec::new();
    for (uid, master) in masters {
        let detached = overrides.remove(uid).unwrap_or_default();
        let mut replaced = HashSet::new();
        for event in detached {
            let id = event
                .recurrence_id
                .ok_or_else(|| CalendarError("missing RECURRENCE-ID".into()))?
                .resolve(floating_zone)?;
            if !replaced.insert(id) {
                return Err(CalendarError(format!("duplicate RECURRENCE-ID for {uid}")));
            }
            if !event.cancelled {
                append_occurrence(
                    &mut result,
                    event,
                    id,
                    event.start.resolve(floating_zone)?,
                    window_start,
                    window_end,
                    floating_zone,
                    limit,
                )?;
            }
        }
        if master.cancelled {
            continue;
        }
        let first = master.start.resolve(floating_zone)?;
        let duration = nominal_duration(master, floating_zone)?;
        let lookback = if master.start.zone == Zone::Date {
            duration + Duration::days(2)
        } else {
            duration
        };
        let search_start = window_start
            .checked_sub_signed(lookback)
            .ok_or_else(|| CalendarError("window start underflow".into()))?;
        let mut starts = Vec::new();
        if master.rrules.is_empty() {
            starts.push(first);
        } else {
            let zone = match master.start.zone {
                Zone::Utc => chrono_tz::UTC,
                Zone::Iana(zone) => zone,
                Zone::Floating | Zone::Date => floating_zone,
            };
            let dtstart = first.with_timezone(&rrule::Tz::from(zone));
            let mut set = RRuleSet::new(dtstart);
            for value in &master.rrules {
                let raw: RRule<Unvalidated> = value
                    .parse()
                    .map_err(|error| CalendarError(format!("invalid RRULE: {error}")))?;
                let rule = raw
                    .validate(dtstart)
                    .map_err(|error| CalendarError(format!("invalid RRULE: {error}")))?;
                set = set.rrule(rule);
            }
            // The library's limit bounds expansion of untrusted, potentially unending rules.
            let lower = search_start
                .checked_sub_signed(Duration::seconds(1))
                .ok_or_else(|| CalendarError("window start underflow".into()))?;
            let expanded = set
                .after(lower.with_timezone(&rrule::Tz::from(zone)))
                .before(window_end.with_timezone(&rrule::Tz::from(zone)))
                .all(10_001);
            if expanded.limited || expanded.dates.len() > 10_000 {
                return Err(CalendarError(
                    "recurrence expansion exceeds 10000 candidates".into(),
                ));
            }
            starts.extend(
                expanded
                    .dates
                    .into_iter()
                    .map(|date| date.with_timezone(&Utc)),
            );
        }
        for date in &master.rdates {
            starts.push(date.resolve(floating_zone)?);
        }
        let excluded: HashSet<_> = master
            .exdates
            .iter()
            .map(|date| date.resolve(floating_zone))
            .collect::<Result<_, _>>()?;
        starts.sort_unstable();
        starts.dedup();
        for start in starts {
            if !excluded.contains(&start) && !replaced.contains(&start) {
                append_occurrence(
                    &mut result,
                    master,
                    start,
                    start,
                    window_start,
                    window_end,
                    floating_zone,
                    limit,
                )?;
            }
        }
    }
    if !overrides.is_empty() {
        return Err(CalendarError("detached VEVENT has no master".into()));
    }
    result.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.uid.cmp(&b.uid))
            .then_with(|| a.recurrence_id.cmp(&b.recurrence_id))
    });
    Ok(result)
}

fn nominal_duration(event: &Event, floating_zone: Tz) -> Result<Duration, CalendarError> {
    let duration = if event.start.zone == Zone::Date && event.end.zone == Zone::Date {
        event.end.local - event.start.local
    } else {
        event.end.resolve(floating_zone)? - event.start.resolve(floating_zone)?
    };
    if duration < Duration::zero() {
        return Err(CalendarError("negative event duration".into()));
    }
    Ok(duration)
}

#[allow(clippy::too_many_arguments)]
fn append_occurrence(
    output: &mut Vec<Occurrence>,
    event: &Event,
    id: DateTime<Utc>,
    start: DateTime<Utc>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    floating_zone: Tz,
    limit: usize,
) -> Result<(), CalendarError> {
    let duration = nominal_duration(event, floating_zone)?;
    let end = if event.start.zone == Zone::Date && event.end.zone == Zone::Date {
        let zone = match event.start.zone {
            Zone::Utc => chrono_tz::UTC,
            Zone::Iana(zone) => zone,
            Zone::Floating | Zone::Date => floating_zone,
        };
        let local_end = start
            .with_timezone(&zone)
            .naive_local()
            .checked_add_signed(duration)
            .ok_or_else(|| CalendarError("event end overflows".into()))?;
        TimeValue {
            local: local_end,
            zone: event.end.zone,
        }
        .resolve(floating_zone)?
    } else {
        start
            .checked_add_signed(duration)
            .ok_or_else(|| CalendarError("event end overflows".into()))?
    };
    if start < window_end && (end > window_start || end == start && start >= window_start) {
        if output.len() >= limit {
            return Err(CalendarError("occurrence result limit exceeded".into()));
        }
        output.push(Occurrence {
            uid: event.uid.clone(),
            recurrence_id: id,
            start,
            end,
            summary: event.summary.clone(),
            all_day: event.start.zone == Zone::Date,
        });
    }
    Ok(())
}
