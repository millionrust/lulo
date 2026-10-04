//! Calendar's read-only presentation model. CAL-2 can replace `fixture_week`
//! with an EDS snapshot without changing navigation or Week layout.

pub mod store;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rmac_calendar_store::{layout_week, LayoutEvent, LayoutSlot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Day,
    Week,
    Month,
    Year,
}

impl View {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Year];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Year => "Year",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarColor {
    Blue,
    Teal,
    Orange,
    Green,
    Purple,
    Red,
}

impl CalendarColor {
    pub const ALL: [Self; 6] = [
        Self::Blue,
        Self::Teal,
        Self::Orange,
        Self::Green,
        Self::Purple,
        Self::Red,
    ];
}

#[derive(Clone, Debug)]
pub struct Calendar {
    pub name: String,
    /// Sidebar section heading: an account name ("Google", "iCloud"), "On My
    /// Mac" for a local calendar, or "Subscribed" for an ICS subscription.
    pub account: String,
    pub color: CalendarColor,
    pub visible: bool,
    /// Soft-deleted calendars stay in place (never removed from the list)
    /// so `Event::calendar` indices into the fixture stay valid; they are
    /// skipped in the sidebar and forced invisible. See [`CalendarView`]'s
    /// own `calendars`/`removed` state (CAL-6), which is the mutable,
    /// session-long copy this fixture seeds once.
    pub removed: bool,
    /// `Some(url)` for an ICS subscription (`http://`, `https://` or
    /// `webcal://`); `None` for a local or account calendar. CAL-6 adds the
    /// subscription and validates the URL; fetching it is CAL-2/CAL-9 work.
    pub subscription_url: Option<String>,
}

impl Calendar {
    fn new(name: &str, account: &str, color: CalendarColor, visible: bool) -> Self {
        Self {
            name: name.to_owned(),
            account: account.to_owned(),
            color,
            visible,
            removed: false,
            subscription_url: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: String,
    pub title: &'static str,
    pub location: &'static str,
    pub calendar: usize,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
}

#[derive(Clone, Debug)]
pub struct WeekSnapshot {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub slots: Vec<LayoutSlot>,
}

#[derive(Clone, Debug)]
pub struct Navigator {
    pub selected: NaiveDate,
    pub view: View,
}

impl Navigator {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            selected: today,
            view: View::Week,
        }
    }

    pub fn today(&mut self, today: NaiveDate) {
        self.selected = today;
    }

    pub fn step(&mut self, direction: i64) {
        self.selected = match self.view {
            View::Day => self.selected.checked_add_signed(Duration::days(direction)),
            View::Week => self
                .selected
                .checked_add_signed(Duration::days(direction * 7)),
            View::Month => shift_month(self.selected, direction),
            View::Year => shift_month(self.selected, direction * 12),
        }
        .unwrap_or(self.selected);
    }

    pub fn week_start(&self) -> NaiveDate {
        self.selected - Duration::days(self.selected.weekday().num_days_from_monday() as i64)
    }

    pub fn move_day(&mut self, days: i64) {
        if let Some(date) = self.selected.checked_add_signed(Duration::days(days)) {
            self.selected = date;
        }
    }
}

fn shift_month(date: NaiveDate, delta: i64) -> Option<NaiveDate> {
    let month_index = i64::from(date.year()) * 12 + i64::from(date.month0()) + delta;
    let year = i32::try_from(month_index.div_euclid(12)).ok()?;
    let month = u32::try_from(month_index.rem_euclid(12) + 1).ok()?;
    let last_day = (NaiveDate::from_ymd_opt(year, month, 1)? + Duration::days(32)).with_day(1)?
        - Duration::days(1);
    NaiveDate::from_ymd_opt(year, month, date.day().min(last_day.day()))
}

pub fn current_date() -> NaiveDate {
    Local::now().date_naive()
}

/// The fixture's seed calendars, in the stable order `Event::calendar`
/// indexes into. CAL-6's `CalendarView` seeds its own mutable calendar list
/// (names, colours, visibility, soft-delete) from this once at startup, and
/// then never calls it again — every later render reuses the fixture's
/// events against that same, possibly user-edited, list.
pub fn seed_calendars() -> Vec<Calendar> {
    vec![
        Calendar::new("Work", "Google", CalendarColor::Blue, true),
        Calendar::new("Team", "Google", CalendarColor::Teal, true),
        Calendar::new("Family", "Google", CalendarColor::Orange, true),
        Calendar::new("Home", "iCloud", CalendarColor::Green, true),
        Calendar::new("Gym", "iCloud", CalendarColor::Purple, false),
        Calendar::new("Holidays", "Other", CalendarColor::Red, true),
    ]
}

pub fn fixture_week(first_day: NaiveDate) -> WeekSnapshot {
    let calendars = seed_calendars();
    let mut events = Vec::new();
    let mut add = |day: i64, from: u32, to: u32, title, location, calendar, all_day| {
        let date = first_day + Duration::days(day);
        // Fixture hours are UTC, making the screenshot reproducible across machines.
        let start = Utc.from_utc_datetime(&date.and_hms_opt(from, 0, 0).unwrap());
        let end = Utc.from_utc_datetime(&date.and_hms_opt(to, 0, 0).unwrap());
        let index = events.len();
        events.push(Event {
            id: format!("fixture-{index}"),
            title,
            location,
            calendar,
            start,
            end,
            all_day,
        });
        index
    };
    for day in 0..5 {
        add(day, 9, 10, "Stand-up", "Room 4", 0, false);
    }
    add(0, 13, 14, "Design review", "Studio", 0, false);
    add(1, 10, 12, "Calendar sync spec", "Video call", 1, false);
    add(2, 12, 13, "Lunch with Ana", "Café Lulo", 3, false);
    add(2, 12, 14, "Vendor call", "", 0, false);
    add(3, 15, 17, "Release triage", "Room 2", 1, false);
    add(4, 16, 17, "1:1 with Sam", "", 0, false);
    add(5, 10, 12, "Farmers market", "Plaza", 2, false);
    add(5, 14, 15, "Haircut", "", 3, false);
    add(6, 11, 13, "Family lunch", "Grandma's", 2, false);
    let conference = add(1, 0, 23, "Conference", "", 1, true);
    let birthday = add(4, 0, 23, "Ana's birthday", "", 2, true);
    events[conference].end = Utc.from_utc_datetime(
        &(first_day + Duration::days(4))
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    );
    events[birthday].end = Utc.from_utc_datetime(
        &(first_day + Duration::days(6))
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    );
    let layout_events: Vec<_> = events
        .iter()
        .filter(|event| !event.all_day)
        .map(|event| LayoutEvent {
            id: event.id.clone(),
            start: event.start,
            end: event.end,
        })
        .collect();
    let slots =
        layout_week(&layout_events, first_day, Tz::UTC).expect("fixture contains valid week dates");
    WeekSnapshot {
        calendars,
        events,
        slots,
    }
}

pub fn month_grid_start(date: NaiveDate) -> NaiveDate {
    let first = date.with_day(1).expect("date has a month");
    first - Duration::days(first.weekday().num_days_from_monday() as i64)
}

pub fn is_weekend(day: NaiveDate) -> bool {
    matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
}

pub fn event_occurs_on(event: &Event, day: NaiveDate) -> bool {
    let start = event.start.date_naive();
    let end = event.end.date_naive();
    if event.all_day {
        start <= day && day < end
    } else {
        start <= day
            && (day < end || (day == end && event.end.time() != chrono::NaiveTime::MIN))
            && event.end > event.start
    }
}

pub fn events_on_day<'a>(
    snapshot: &'a WeekSnapshot,
    visible: &[bool],
    day: NaiveDate,
) -> Vec<&'a Event> {
    snapshot
        .events
        .iter()
        .filter(|event| {
            visible.get(event.calendar).copied().unwrap_or(false) && event_occurs_on(event, day)
        })
        .collect()
}

/// CAL-6: calendar list management (new/rename/delete, ICS subscriptions)
/// and search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarNameError {
    Empty,
    TooLong,
}

/// macOS allows any non-empty calendar name (trimmed of leading/trailing
/// whitespace); Lulo additionally caps it for the sidebar layout.
pub const MAX_CALENDAR_NAME_LEN: usize = 120;

pub fn validate_calendar_name(name: &str) -> Result<String, CalendarNameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CalendarNameError::Empty);
    }
    if trimmed.chars().count() > MAX_CALENDAR_NAME_LEN {
        return Err(CalendarNameError::TooLong);
    }
    Ok(trimmed.to_owned())
}

/// "New Calendar", "New Calendar 2", "New Calendar 3", … — the same
/// uniquification Files uses for "untitled folder".
pub fn unique_calendar_name(existing: &[Calendar], base: &str) -> String {
    let taken = |candidate: &str| {
        existing
            .iter()
            .any(|calendar| !calendar.removed && calendar.name.eq_ignore_ascii_case(candidate))
    };
    if !taken(base) {
        return base.to_owned();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base} {suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionUrlError {
    Empty,
    UnsupportedScheme,
    MissingHost,
}

/// Validates an ICS subscription URL (`File ▸ New Calendar Subscription…`):
/// `http://`, `https://` or `webcal://` with a non-empty host. Returns the
/// trimmed URL unchanged — `webcal://` is translated to `https://` only when
/// the adapter actually fetches it (CAL-2/CAL-9), not here.
pub fn validate_subscription_url(raw: &str) -> Result<String, SubscriptionUrlError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SubscriptionUrlError::Empty);
    }
    let rest = ["webcal://", "https://", "http://"]
        .into_iter()
        .find_map(|scheme| {
            trimmed
                .get(..scheme.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(scheme))
                .map(|_| &trimmed[scheme.len()..])
        })
        .ok_or(SubscriptionUrlError::UnsupportedScheme)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(SubscriptionUrlError::MissingHost);
    }
    Ok(trimmed.to_owned())
}

/// A reasonable default name for a subscription, from the last non-empty
/// path segment (minus a `.ics` extension), falling back to "Subscription".
pub fn subscription_default_name(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let last_segment = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let stem = last_segment.strip_suffix(".ics").unwrap_or(last_segment);
    let name = stem.replace(['-', '_'], " ").trim().to_owned();
    if name.is_empty() {
        "Subscription".to_owned()
    } else {
        name
    }
}

/// One match for the toolbar search field.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub event_id: String,
    pub calendar: usize,
    pub start: DateTime<Utc>,
}

/// Finds events by title or location across calendars that are both visible
/// and not soft-deleted, newest first by start time. Matching on notes and
/// invitees (ADR 0022 §2) waits on those fields landing on `Event` with
/// CAL-5/CAL-8.
pub fn search_events(snapshot: &WeekSnapshot, visible: &[bool], query: &str) -> Vec<SearchResult> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    let mut results: Vec<&Event> = snapshot
        .events
        .iter()
        .filter(|event| {
            visible.get(event.calendar).copied().unwrap_or(false)
                && !snapshot
                    .calendars
                    .get(event.calendar)
                    .is_some_and(|calendar| calendar.removed)
                && (event.title.to_lowercase().contains(&query_lower)
                    || event.location.to_lowercase().contains(&query_lower))
        })
        .collect();
    results.sort_by_key(|event| event.start);
    results
        .into_iter()
        .map(|event| SearchResult {
            event_id: event.id.clone(),
            calendar: event.calendar,
            start: event.start,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn week_navigation_and_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let mut nav = Navigator::new(today);
        assert_eq!(
            nav.week_start(),
            NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
        );
        nav.step(1);
        assert_eq!(
            nav.week_start(),
            NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
        );
        nav.today(today);
        assert_eq!(nav.selected, today);
    }

    #[test]
    fn month_navigation_clamps_short_months() {
        let mut nav = Navigator::new(NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        nav.view = View::Month;
        nav.step(1);
        assert_eq!(nav.selected, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    #[test]
    fn fixture_has_overlapping_week_events_and_all_day_strip() {
        let snapshot = fixture_week(NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        assert_eq!(snapshot.calendars.len(), 6);
        assert_eq!(snapshot.events.len(), 16);
        assert_eq!(
            snapshot.events.iter().filter(|event| event.all_day).count(),
            2
        );
        assert_eq!(
            (snapshot.events[14].end - snapshot.events[14].start).num_days(),
            3
        );
        assert!(snapshot.slots.iter().any(|slot| slot.columns == 2));
    }

    #[test]
    fn month_day_events_respect_visibility_and_exclusive_all_day_end() {
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let snapshot = fixture_week(first);
        let mut visible = [true; 6];
        assert_eq!(
            events_on_day(&snapshot, &visible, first + Duration::days(2)).len(),
            4
        );
        visible[1] = false;
        assert_eq!(
            events_on_day(&snapshot, &visible, first + Duration::days(2)).len(),
            3
        );
        assert!(!event_occurs_on(
            &snapshot.events[14],
            first + Duration::days(4)
        ));
    }

    #[test]
    fn month_arrow_navigation_crosses_week_and_month_boundary() {
        let mut nav = Navigator::new(NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        nav.view = View::Month;
        nav.move_day(1);
        assert_eq!(nav.selected, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        nav.move_day(7);
        assert_eq!(nav.selected, NaiveDate::from_ymd_opt(2026, 11, 8).unwrap());
        nav.move_day(-7);
        assert_eq!(nav.selected, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
    }

    #[test]
    fn calendar_name_is_trimmed_and_bounded() {
        assert_eq!(validate_calendar_name("  Work  ").unwrap(), "Work");
        assert_eq!(validate_calendar_name(""), Err(CalendarNameError::Empty));
        assert_eq!(validate_calendar_name("   "), Err(CalendarNameError::Empty));
        let long = "x".repeat(MAX_CALENDAR_NAME_LEN + 1);
        assert_eq!(
            validate_calendar_name(&long),
            Err(CalendarNameError::TooLong)
        );
    }

    #[test]
    fn new_calendar_name_is_uniquified_like_a_new_folder() {
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let mut snapshot = fixture_week(first);
        assert_eq!(
            unique_calendar_name(&snapshot.calendars, "New Calendar"),
            "New Calendar"
        );
        snapshot.calendars.push(Calendar::new(
            "New Calendar",
            "On My Mac",
            CalendarColor::Blue,
            true,
        ));
        assert_eq!(
            unique_calendar_name(&snapshot.calendars, "New Calendar"),
            "New Calendar 2"
        );
        snapshot.calendars.push(Calendar::new(
            "New Calendar 2",
            "On My Mac",
            CalendarColor::Blue,
            true,
        ));
        assert_eq!(
            unique_calendar_name(&snapshot.calendars, "New Calendar"),
            "New Calendar 3"
        );
        // A soft-deleted calendar's name is free to reuse.
        snapshot.calendars.last_mut().unwrap().removed = true;
        assert_eq!(
            unique_calendar_name(&snapshot.calendars, "New Calendar 2"),
            "New Calendar 2"
        );
    }

    #[test]
    fn subscription_url_requires_a_known_scheme_and_host() {
        assert_eq!(
            validate_subscription_url("webcal://calendar.example.com/team.ics").unwrap(),
            "webcal://calendar.example.com/team.ics"
        );
        assert!(validate_subscription_url("https://example.com/a.ics").is_ok());
        assert!(validate_subscription_url("HTTPS://Example.com/a.ics").is_ok());
        assert_eq!(
            validate_subscription_url(""),
            Err(SubscriptionUrlError::Empty)
        );
        assert_eq!(
            validate_subscription_url("ftp://example.com/a.ics"),
            Err(SubscriptionUrlError::UnsupportedScheme)
        );
        assert_eq!(
            validate_subscription_url("https:///a.ics"),
            Err(SubscriptionUrlError::MissingHost)
        );
    }

    #[test]
    fn subscription_default_name_comes_from_the_last_path_segment() {
        assert_eq!(
            subscription_default_name("https://example.com/team-releases.ics"),
            "team releases"
        );
        assert_eq!(
            subscription_default_name("webcal://example.com/cal/"),
            "cal"
        );
        // No path beyond the host: the host itself is a reasonable name.
        assert_eq!(
            subscription_default_name("https://example.com/"),
            "example.com"
        );
        // Nothing at all to build a name from falls back to "Subscription".
        assert_eq!(subscription_default_name(""), "Subscription");
    }

    #[test]
    fn search_matches_title_or_location_in_visible_calendars_only() {
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let snapshot = fixture_week(first);
        let mut visible = [true; 6];
        let results = search_events(&snapshot, &visible, "lunch");
        assert_eq!(results.len(), 2);
        assert!(results
            .windows(2)
            .all(|pair| pair[0].start <= pair[1].start));
        // Matches on location, not just title, while every calendar is shown.
        assert_eq!(search_events(&snapshot, &visible, "Café Lulo").len(), 1);
        // "Lunch with Ana" (the Café Lulo event) is on calendar 3; hiding it
        // drops that match from both searches.
        visible[3] = false;
        let results = search_events(&snapshot, &visible, "lunch");
        assert_eq!(results.len(), 1);
        assert_eq!(search_events(&snapshot, &visible, "  ").len(), 0);
        assert_eq!(search_events(&snapshot, &visible, "Café Lulo").len(), 0);
    }

    #[test]
    fn search_skips_soft_deleted_calendars() {
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let mut snapshot = fixture_week(first);
        snapshot.calendars[0].removed = true;
        let visible = [true; 6];
        assert_eq!(search_events(&snapshot, &visible, "Stand-up").len(), 0);
    }
}
