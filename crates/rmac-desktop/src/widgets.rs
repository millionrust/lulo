//! Desktop and Notification Centre widgets: which ones exist (only those
//! rmac has a real backend for), where they sit, and the pure parts of
//! what they draw. Sizes follow the macOS widget families (S, see
//! design-lab/desktop.html); the gallery numbers are measured.

use serde::{Deserialize, Serialize};

/// Small widget edge and the medium width (S: Apple's macOS families).
pub const SMALL: f32 = 170.0;
pub const MEDIUM_WIDTH: f32 = 364.0;
/// Gap between widgets placed side by side (S; 2 × 170 + 24 = 364).
pub const GAP: f32 = 24.0;
/// Corner radius (S: the gallery's measured 17 at 112 scaled to 170).
pub const RADIUS: f32 = 24.0;
/// Where the first widget goes on an empty desktop (S).
pub const FIRST_LEFT: f32 = 20.0;
pub const FIRST_TOP: f32 = 53.0;
/// The menu bar the widgets stay below.
pub const MENU_BAR: f32 = 33.0;
pub const MAX_WIDGETS: usize = 32;

/// Widgets rmac can back with real data. Reminders, Photos, Notes and the
/// Calendar "Up Next" list have no rmac data source yet and are omitted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum WidgetKind {
    /// Batteries: this computer's charge (UPower).
    Battery,
    /// Calendar: today's month.
    Calendar,
    /// Clock: an analogue face with the local time.
    Clock,
    /// Weather: the place selected in Weather, from its forecast cache.
    Weather,
}

impl WidgetKind {
    pub const ALL: [Self; 4] = [Self::Battery, Self::Calendar, Self::Clock, Self::Weather];

    /// The gallery's source name.
    pub fn source(self) -> &'static str {
        match self {
            Self::Battery => "Batteries",
            Self::Calendar => "Calendar",
            Self::Clock => "Clock",
            Self::Weather => "Weather",
        }
    }

    /// Sizes rmac draws. Medium and Large need data rmac does not have yet
    /// (a per-widget hourly forecast, other devices' batteries, events).
    pub fn sizes(self) -> &'static [WidgetSize] {
        &[WidgetSize::Small]
    }
}

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub enum WidgetSize {
    #[default]
    Small,
    Medium,
}

impl WidgetSize {
    pub fn size(self) -> (f32, f32) {
        match self {
            Self::Small => (SMALL, SMALL),
            Self::Medium => (MEDIUM_WIDTH, SMALL),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "on")]
pub enum WidgetLocation {
    /// Top-left corner in screen points.
    Desktop {
        left: f32,
        top: f32,
    },
    NotificationCenter,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Widget {
    pub id: u64,
    pub kind: WidgetKind,
    #[serde(default)]
    pub size: WidgetSize,
    pub location: WidgetLocation,
}

impl Widget {
    pub fn is_valid(&self) -> bool {
        self.kind.sizes().contains(&self.size)
            && match self.location {
                WidgetLocation::Desktop { left, top } => left.is_finite() && top.is_finite(),
                WidgetLocation::NotificationCenter => true,
            }
    }

    pub fn frame(&self) -> Option<Frame> {
        match self.location {
            WidgetLocation::Desktop { left, top } => {
                let (width, height) = self.size.size();
                Some(Frame {
                    left,
                    top,
                    width,
                    height,
                })
            }
            WidgetLocation::NotificationCenter => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Frame {
    fn overlaps(&self, other: &Frame) -> bool {
        self.left < other.left + other.width
            && other.left < self.left + self.width
            && self.top < other.top + other.height
            && other.top < self.top + self.height
    }
}

/// Keeps a widget fully on screen and below the menu bar.
pub fn clamp_origin(size: WidgetSize, left: f32, top: f32, screen: (f32, f32)) -> (f32, f32) {
    let (width, height) = size.size();
    let left = if left.is_finite() { left } else { FIRST_LEFT };
    let top = if top.is_finite() { top } else { FIRST_TOP };
    (
        left.clamp(0.0, (screen.0 - width).max(0.0)),
        top.clamp(MENU_BAR, (screen.1 - height).max(MENU_BAR)),
    )
}

/// Where a widget added from the gallery goes: the first spot on a
/// 170 + 24 lattice from the top-left, down then across, that overlaps no
/// desktop widget.
pub fn next_desktop_origin(
    existing: &[Widget],
    size: WidgetSize,
    screen: (f32, f32),
) -> (f32, f32) {
    let frames = existing
        .iter()
        .filter_map(Widget::frame)
        .collect::<Vec<_>>();
    let (width, height) = size.size();
    let step = SMALL + GAP;
    let mut left = FIRST_LEFT;
    while left + width <= screen.0 {
        let mut top = FIRST_TOP;
        while top + height <= screen.1 {
            let frame = Frame {
                left,
                top,
                width,
                height,
            };
            if !frames.iter().any(|other| frame.overlaps(other)) {
                return (left, top);
            }
            top += step;
        }
        left += step;
    }
    (FIRST_LEFT, FIRST_TOP)
}

/// A month laid out Sunday-first, as the Calendar widget draws it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonthGrid {
    /// "SEPTEMBER".
    pub title: &'static str,
    /// Leading blanks then days; always whole weeks.
    pub cells: Vec<Option<u8>>,
    pub today: u8,
}

const MONTHS: [&str; 12] = [
    "JANUARY",
    "FEBRUARY",
    "MARCH",
    "APRIL",
    "MAY",
    "JUNE",
    "JULY",
    "AUGUST",
    "SEPTEMBER",
    "OCTOBER",
    "NOVEMBER",
    "DECEMBER",
];

pub const WEEKDAY_INITIALS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

pub fn days_in_month(year: i32, month: u32) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// 0 = Sunday (Sakamoto's method, proleptic Gregorian).
pub fn weekday(year: i32, month: u32, day: u32) -> u32 {
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let month = month.clamp(1, 12);
    let year = if month < 3 { year - 1 } else { year };
    (year + year.div_euclid(4) - year.div_euclid(100)
        + year.div_euclid(400)
        + OFFSETS[(month - 1) as usize]
        + day as i32)
        .rem_euclid(7) as u32
}

pub fn month_grid(year: i32, month: u32, today: u32) -> MonthGrid {
    let month = month.clamp(1, 12);
    let lead = weekday(year, month, 1) as usize;
    let days = days_in_month(year, month);
    let mut cells = vec![None; lead];
    cells.extend((1..=days).map(Some));
    while cells.len() % 7 != 0 {
        cells.push(None);
    }
    MonthGrid {
        title: MONTHS[(month - 1) as usize],
        cells,
        today: today.min(u32::from(days)) as u8,
    }
}

/// Hour, minute and second hand angles in degrees clockwise from 12.
pub fn clock_hands(hour: u32, minute: u32, second: u32) -> (f32, f32, f32) {
    let second = (second % 60) as f32;
    let minute = (minute % 60) as f32 + second / 60.0;
    let hour = (hour % 12) as f32 + minute / 60.0;
    (hour * 30.0, minute * 6.0, second * 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop(id: u64, left: f32, top: f32) -> Widget {
        Widget {
            id,
            kind: WidgetKind::Clock,
            size: WidgetSize::Small,
            location: WidgetLocation::Desktop { left, top },
        }
    }

    #[test]
    fn september_2026_starts_on_a_tuesday() {
        assert_eq!(weekday(2026, 9, 1), 2);
        assert_eq!(weekday(2026, 9, 23), 3);
        assert_eq!(weekday(2000, 1, 1), 6);
        let grid = month_grid(2026, 9, 23);
        assert_eq!(grid.title, "SEPTEMBER");
        assert_eq!(&grid.cells[..3], &[None, None, Some(1)]);
        assert_eq!(grid.cells.len(), 35);
        assert_eq!(grid.cells.iter().flatten().count(), 30);
        assert_eq!(grid.today, 23);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
    }

    #[test]
    fn clock_hands_move_continuously() {
        assert_eq!(clock_hands(0, 0, 0), (0.0, 0.0, 0.0));
        assert_eq!(clock_hands(15, 30, 0), (105.0, 180.0, 0.0));
        let (hour, minute, second) = clock_hands(9, 16, 30);
        assert!((hour - 278.25).abs() < 0.01);
        assert!((minute - 99.0).abs() < 0.01);
        assert_eq!(second, 180.0);
    }

    #[test]
    fn new_widgets_fill_down_then_across_without_overlap() {
        let screen = (1470.0, 956.0);
        assert_eq!(
            next_desktop_origin(&[], WidgetSize::Small, screen),
            (FIRST_LEFT, FIRST_TOP)
        );
        let placed = [desktop(1, FIRST_LEFT, FIRST_TOP)];
        assert_eq!(
            next_desktop_origin(&placed, WidgetSize::Small, screen),
            (FIRST_LEFT, FIRST_TOP + SMALL + GAP)
        );
        let column = [
            desktop(1, FIRST_LEFT, FIRST_TOP),
            desktop(2, FIRST_LEFT, FIRST_TOP + 194.0),
            desktop(3, FIRST_LEFT, FIRST_TOP + 388.0),
            desktop(4, FIRST_LEFT, FIRST_TOP + 582.0),
        ];
        assert_eq!(
            next_desktop_origin(&column, WidgetSize::Small, screen),
            (FIRST_LEFT + 194.0, FIRST_TOP)
        );
    }

    #[test]
    fn widgets_stay_on_screen_and_only_offer_backed_sizes() {
        assert_eq!(
            clamp_origin(WidgetSize::Medium, 2000.0, 0.0, (1470.0, 956.0)),
            (1470.0 - MEDIUM_WIDTH, MENU_BAR)
        );
        let mut widget = desktop(1, 10.0, 40.0);
        assert!(widget.is_valid());
        widget.size = WidgetSize::Medium;
        assert!(!widget.is_valid());
        widget.size = WidgetSize::Small;
        widget.location = WidgetLocation::Desktop {
            left: f32::NAN,
            top: 40.0,
        };
        assert!(!widget.is_valid());
    }
}
