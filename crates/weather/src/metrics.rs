//! Weather window geometry (dark appearance). The Mac only showed its
//! location alert (target/evidence/mac-2026-09-23/weather.png), so these
//! are drawn from macOS 26 Weather as known and are all `S`; see
//! design-lab/apps-clock-weather-player.html.

pub const WINDOW: (f32, f32) = (980.0, 720.0);
pub const MIN_WINDOW: (f32, f32) = (720.0, 520.0);
pub const TRAFFIC_LIGHT_CENTER: (f32, f32) = (26.0, 26.0);
pub const TOOLBAR_HEIGHT: f32 = 52.0;

pub const SIDEBAR_WIDTH: f32 = 290.0;
pub const SIDEBAR_FILL: u32 = 0x232325;
pub const SEARCH_HEIGHT: f32 = 28.0;
pub const SEARCH_INSET: f32 = 14.0;
pub const CARD_HEIGHT: f32 = 88.0;
pub const CARD_GAP: f32 = 8.0;
pub const CARD_INSET: f32 = 10.0;
pub const CARD_RADIUS: f32 = 12.0;

pub const CITY_SIZE: f32 = 34.0;
pub const TEMPERATURE_SIZE: f32 = 96.0;
pub const CONDITION_SIZE: f32 = 20.0;
pub const HEADER_TOP: f32 = 58.0;

pub const PANEL_INSET: f32 = 30.0;
pub const PANEL_RADIUS: f32 = 16.0;
/// rgba: black at 14 %.
pub const PANEL_FILL: u32 = 0x0000_0024;
pub const PANEL_RULE: u32 = 0xFFFF_FF40;
pub const CAPTION_SIZE: f32 = 12.0;
pub const HOUR_WIDTH: f32 = 60.0;
pub const HOUR_ICON: f32 = 26.0;
pub const DAY_ROW: f32 = 44.0;
pub const DAY_ICON: f32 = 24.0;
pub const RANGE_WIDTH: f32 = 104.0;
pub const TILE_HEIGHT: f32 = 110.0;
