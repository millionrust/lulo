//! Launch arguments that open Calendar at a specific place (CAL-8): the
//! CAL-7 reminders agent's notification default action, and the
//! Notification Centre Calendar widget's "Up Next" rows and mini month.
//!
//! Parsing is pure and argv-shaped (not a `calendar:` URI scheme -- nothing
//! else resolves MIME/URI handlers to Calendar yet) so both callers can
//! build a plain `Command` without a URL-encoding round trip.

use chrono::{DateTime, NaiveDate, Utc};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeepLink {
    /// `--event <calendar-uid> <event-uid> <occurrence-start RFC 3339>`:
    /// jumps to and selects that exact occurrence in Day view (the same
    /// landing `jump_to_search_result` gives a search hit).
    Event {
        calendar_uid: String,
        event_uid: String,
        occurrence_start: DateTime<Utc>,
    },
    /// `--date <YYYY-MM-DD>`: just navigates to that day (the mini month
    /// and the Small widget have no single event to point at).
    Date(NaiveDate),
}

/// `args` is the process's own arguments without the executable name
/// (`std::env::args().skip(1)`).
pub fn parse(args: &[String]) -> Option<DeepLink> {
    match args {
        [flag, calendar_uid, event_uid, occurrence_start]
            if flag == "--event" && !calendar_uid.is_empty() && !event_uid.is_empty() =>
        {
            let occurrence_start = DateTime::parse_from_rfc3339(occurrence_start)
                .ok()?
                .with_timezone(&Utc);
            Some(DeepLink::Event {
                calendar_uid: calendar_uid.clone(),
                event_uid: event_uid.clone(),
                occurrence_start,
            })
        }
        [flag, date] if flag == "--date" => NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .map(DeepLink::Date),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_an_event_link() {
        let link = parse(&args(&[
            "--event",
            "work-cal",
            "meeting-1",
            "2026-10-10T10:00:00Z",
        ]))
        .unwrap();
        assert_eq!(
            link,
            DeepLink::Event {
                calendar_uid: "work-cal".into(),
                event_uid: "meeting-1".into(),
                occurrence_start: "2026-10-10T10:00:00Z".parse().unwrap(),
            }
        );
    }

    #[test]
    fn parses_a_date_link() {
        assert_eq!(
            parse(&args(&["--date", "2026-10-10"])),
            Some(DeepLink::Date(
                NaiveDate::from_ymd_opt(2026, 10, 10).unwrap()
            ))
        );
    }

    #[test]
    fn rejects_malformed_or_unknown_arguments() {
        assert_eq!(parse(&args(&[])), None);
        assert_eq!(parse(&args(&["--event", "a", "b"])), None);
        assert_eq!(parse(&args(&["--event", "a", "b", "not-a-date"])), None);
        assert_eq!(parse(&args(&["--date", "not-a-date"])), None);
        assert_eq!(parse(&args(&["--bogus", "x"])), None);
        assert_eq!(
            parse(&args(&["--event", "", "b", "2026-10-10T10:00:00Z"])),
            None
        );
    }
}
