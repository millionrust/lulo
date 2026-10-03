use crate::CalendarError;
use chrono::{DateTime, Days, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutEvent {
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Seconds use real elapsed time from local midnight, preserving the extra or missing DST hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutSlot {
    pub id: String,
    pub day: NaiveDate,
    pub start_second: i64,
    pub end_second: i64,
    pub column: usize,
    pub columns: usize,
}

/// Deterministic day column placement. A week view calls this once per visible day.
/// Intervals touching at an endpoint do not overlap.
pub fn layout_day(
    events: &[LayoutEvent],
    day: NaiveDate,
    zone: Tz,
) -> Result<Vec<LayoutSlot>, CalendarError> {
    let next = day
        .checked_add_days(Days::new(1))
        .ok_or_else(|| CalendarError("day overflows".into()))?;
    let day_start = local_midnight(day, zone)?;
    let day_end = local_midnight(next, zone)?;
    let mut visible: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if event.end <= event.start || event.end <= day_start || event.start >= day_end {
                None
            } else {
                Some((event, event.start.max(day_start), event.end.min(day_end)))
            }
        })
        .collect();
    visible.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });

    let mut result = Vec::with_capacity(visible.len());
    let mut group_start = 0;
    let mut group_end = day_start;
    let mut active: Vec<(usize, DateTime<Utc>)> = Vec::new();
    let mut group_width = 0;
    for (event, start, end) in visible {
        if !active.is_empty() && start >= group_end {
            for slot in &mut result[group_start..] {
                slot.columns = group_width;
            }
            group_start = result.len();
            active.clear();
            group_width = 0;
        }
        active.retain(|(_, active_end)| *active_end > start);
        let column = (0..)
            .find(|column| active.iter().all(|(used, _)| used != column))
            .ok_or_else(|| CalendarError("no layout column available".into()))?;
        active.push((column, end));
        group_width = group_width.max(column + 1);
        group_end = group_end.max(end);
        result.push(LayoutSlot {
            id: event.id.clone(),
            day,
            start_second: (start - day_start).num_seconds(),
            end_second: (end - day_start).num_seconds(),
            column,
            columns: 0,
        });
    }
    for slot in &mut result[group_start..] {
        slot.columns = group_width;
    }
    Ok(result)
}

/// Lay out seven consecutive local day columns, splitting events at local midnight.
pub fn layout_week(
    events: &[LayoutEvent],
    first_day: NaiveDate,
    zone: Tz,
) -> Result<Vec<LayoutSlot>, CalendarError> {
    let mut slots = Vec::new();
    for offset in 0..7 {
        let day = first_day
            .checked_add_days(Days::new(offset))
            .ok_or_else(|| CalendarError("week overflows".into()))?;
        slots.extend(layout_day(events, day, zone)?);
    }
    Ok(slots)
}

fn local_midnight(day: NaiveDate, zone: Tz) -> Result<DateTime<Utc>, CalendarError> {
    let local = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| CalendarError("invalid midnight".into()))?;
    match zone.from_local_datetime(&local) {
        LocalResult::Single(time) | LocalResult::Ambiguous(time, _) => Ok(time.with_timezone(&Utc)),
        LocalResult::None => Err(CalendarError(format!(
            "midnight does not exist on {day} in {zone}"
        ))),
    }
}
