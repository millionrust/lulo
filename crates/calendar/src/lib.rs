//! Calendar presentation and event editing model. EDS work stays off the UI thread.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use rmac_calendar_store::{layout_week, LayoutEvent, LayoutSlot};

pub mod editing;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarColor {
    Blue,
    Teal,
    Orange,
    Green,
    Purple,
    Red,
}

#[derive(Clone, Debug)]
pub struct Calendar {
    pub name: String,
    pub account: String,
    pub color: CalendarColor,
    pub visible: bool,
    pub source_uid: Option<String>,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub location: String,
    pub calendar: usize,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub ical: Option<rmac_calendar_store::Event>,
}

#[derive(Clone, Debug)]
pub struct WeekSnapshot {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub slots: Vec<LayoutSlot>,
}

impl WeekSnapshot {
    pub fn empty() -> Self {
        Self {
            calendars: Vec::new(),
            events: Vec::new(),
            slots: Vec::new(),
        }
    }

    pub fn slots_for(&self, first: NaiveDate) -> Vec<LayoutSlot> {
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|event| !event.all_day)
            .map(|event| LayoutEvent {
                id: event.id.clone(),
                start: event.start,
                end: event.end,
            })
            .collect();
        layout_week(&events, first, Tz::UTC).unwrap_or_default()
    }
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

pub fn fixture_week(first_day: NaiveDate) -> WeekSnapshot {
    let calendars = vec![
        Calendar {
            name: "Work".into(),
            account: "Google".into(),
            color: CalendarColor::Blue,
            visible: true,
            source_uid: None,
            writable: false,
        },
        Calendar {
            name: "Team".into(),
            account: "Google".into(),
            color: CalendarColor::Teal,
            visible: true,
            source_uid: None,
            writable: false,
        },
        Calendar {
            name: "Family".into(),
            account: "Google".into(),
            color: CalendarColor::Orange,
            visible: true,
            source_uid: None,
            writable: false,
        },
        Calendar {
            name: "Home".into(),
            account: "iCloud".into(),
            color: CalendarColor::Green,
            visible: true,
            source_uid: None,
            writable: false,
        },
        Calendar {
            name: "Gym".into(),
            account: "iCloud".into(),
            color: CalendarColor::Purple,
            visible: false,
            source_uid: None,
            writable: false,
        },
        Calendar {
            name: "Holidays".into(),
            account: "Other".into(),
            color: CalendarColor::Red,
            visible: true,
            source_uid: None,
            writable: false,
        },
    ];
    let mut events = Vec::new();
    let mut add = |day: i64,
                   from: u32,
                   to: u32,
                   title: &str,
                   location: &str,
                   calendar: usize,
                   all_day: bool| {
        let date = first_day + Duration::days(day);
        // Fixture hours are UTC, making the screenshot reproducible across machines.
        let start = Utc.from_utc_datetime(&date.and_hms_opt(from, 0, 0).unwrap());
        let end = Utc.from_utc_datetime(&date.and_hms_opt(to, 0, 0).unwrap());
        let index = events.len();
        events.push(Event {
            id: format!("fixture-{index}"),
            title: title.into(),
            location: location.into(),
            calendar,
            start,
            end,
            all_day,
            ical: None,
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
}
