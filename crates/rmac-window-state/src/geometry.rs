//! Window geometry validation and display fitting.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

const MAX_DIMENSION: f64 = 32_768.0;
const MAX_COORDINATE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub mode: WindowMode,
}

impl WindowState {
    pub fn checked(x: f64, y: f64, width: f64, height: f64, mode: WindowMode) -> Option<Self> {
        let state = Self {
            x,
            y,
            width,
            height,
            mode,
        };
        state.is_valid().then_some(state)
    }

    /// Fit persisted geometry onto a connected display. The first display is
    /// the preferred fallback (normally the primary display). Existing
    /// placement wins when it still intersects a display; otherwise the window
    /// is centered on the preferred display. The result is wholly on-screen.
    pub fn fit_to_displays(
        self,
        displays: &[DisplayBounds],
        minimum_width: f64,
        minimum_height: f64,
    ) -> Option<Self> {
        if !self.is_valid() || !valid_dimension(minimum_width) || !valid_dimension(minimum_height) {
            return None;
        }
        let valid_displays = displays
            .iter()
            .copied()
            .filter(DisplayBounds::is_valid)
            .collect::<Vec<_>>();
        let preferred = *valid_displays.first()?;
        let target = valid_displays
            .iter()
            .copied()
            .max_by(|left, right| {
                overlap_area(self, *left)
                    .partial_cmp(&overlap_area(self, *right))
                    .unwrap_or(Ordering::Equal)
            })
            .filter(|display| overlap_area(self, *display) > 0.0)
            .unwrap_or(preferred);

        let width = self
            .width
            .clamp(minimum_width.min(target.width), target.width);
        let height = self
            .height
            .clamp(minimum_height.min(target.height), target.height);
        let had_visible_placement = overlap_area(self, target) > 0.0;
        let (x, y) = if had_visible_placement {
            (
                self.x.clamp(target.x, target.right() - width),
                self.y.clamp(target.y, target.bottom() - height),
            )
        } else {
            (
                target.x + (target.width - width) / 2.0,
                target.y + (target.height - height) / 2.0,
            )
        };
        Self::checked(x, y, width, height, self.mode)
    }

    pub(super) fn is_valid(self) -> bool {
        valid_coordinate(self.x)
            && valid_coordinate(self.y)
            && valid_dimension(self.width)
            && valid_dimension(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl DisplayBounds {
    pub fn checked(x: f64, y: f64, width: f64, height: f64) -> Option<Self> {
        let bounds = Self {
            x,
            y,
            width,
            height,
        };
        bounds.is_valid().then_some(bounds)
    }

    fn is_valid(&self) -> bool {
        valid_coordinate(self.x)
            && valid_coordinate(self.y)
            && valid_dimension(self.width)
            && valid_dimension(self.height)
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

fn overlap_area(window: WindowState, display: DisplayBounds) -> f64 {
    let width = (window.x + window.width).min(display.right()) - window.x.max(display.x);
    let height = (window.y + window.height).min(display.bottom()) - window.y.max(display.y);
    width.max(0.0) * height.max(0.0)
}

fn valid_coordinate(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_COORDINATE
}

fn valid_dimension(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_DIMENSION
}
