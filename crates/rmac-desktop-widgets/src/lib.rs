//! Widget faces for the desktop, the Edit Widgets gallery and Notification
//! Centre. Each face is drawn at 170-point "small" size times `scale` (the
//! gallery shows them at 112, a measured 0.66). Numbers come from
//! design-lab/desktop.html: the gallery tiles were measured on the Mac; the
//! 170-point layout is those numbers × 1.52 (S).

use chrono::{Datelike as _, Local, Timelike as _};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, rgba, svg, AnyElement, Bounds, FontWeight, Hsla,
    PathBuilder, Pixels, SharedString, Window,
};
use rmac_desktop::widgets::{self, WidgetKind, WidgetSize};
use rmac_weather::widget::{Unavailable, WidgetWeather};

/// The measured gallery tile fill (#1A2459) as a translucent dark glass
/// over the wallpaper (S: solved from the tile and the gallery behind it).
pub const GLASS: u32 = 0x17132EB0;
/// The tile's light rim.
pub const RIM: u32 = 0xFFFFFF40;
/// Measured battery green in the gallery.
pub const BATTERY_GREEN: u32 = 0x70D871;
/// The Clock app's second hand.
pub const SECOND_HAND: u32 = 0xF09748;
pub const CALENDAR_RED: u32 = 0xFF453A;
/// The gallery draws small widgets 112 points wide.
pub const GALLERY_SCALE: f32 = 112.0 / widgets::SMALL;

/// This computer's battery, as the Batteries widget shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

impl Battery {
    pub fn from_power(battery: &rmac_power::Battery) -> Self {
        Self {
            percent: battery.percentage.min(100),
            charging: matches!(
                battery.state,
                rmac_power::BatteryState::Charging | rmac_power::BatteryState::FullyCharged
            ) || !battery.on_battery,
        }
    }
}

/// What the faces draw; each field is `None` until it has been read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetData {
    pub weather: Option<Result<WidgetWeather, Unavailable>>,
    pub battery: Option<Battery>,
    /// The power service answered and there is no battery.
    pub no_battery: bool,
}

impl WidgetData {
    /// Batteries is offered only on a computer with a battery.
    pub fn offers(&self, kind: WidgetKind) -> bool {
        kind != WidgetKind::Battery || !self.no_battery
    }
}

/// Blocking: reads the battery through the power service.
pub fn read_battery() -> (Option<Battery>, bool) {
    match rmac_power::snapshot() {
        Ok(snapshot) => match snapshot.battery {
            Some(battery) => (Some(Battery::from_power(&battery)), false),
            None => (None, true),
        },
        Err(_) => (None, false),
    }
}

/// Blocking: the Weather widget's content, refreshing a stale cache.
pub fn read_weather(refresh: bool) -> Result<WidgetWeather, Unavailable> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    rmac_weather::widget::load(now, refresh)
}

macro_rules! weather_glyphs {
    ($($name:literal),* $(,)?) => {
        /// The faces' glyphs, served under `widgets/weather/`.
        pub fn asset(path: &str) -> Option<&'static [u8]> {
            $(
                if path == concat!("widgets/weather/", $name, ".svg") {
                    return Some(include_bytes!(concat!(
                        "../../weather/assets/icons/weather/",
                        $name,
                        ".svg"
                    )));
                }
            )*
            None
        }
    };
}

weather_glyphs!(
    "bolt",
    "cloud-front",
    "cloud-top",
    "cloud",
    "fog",
    "location",
    "moon-small",
    "moon",
    "rain",
    "snow",
    "sun-small",
    "sun",
);

fn glyph(name: &str, size: f32, color: u32) -> gpui::Svg {
    svg()
        .size(px(size))
        .flex_none()
        .path(SharedString::from(format!("widgets/weather/{name}.svg")))
        .text_color(rgb(color))
}

/// A widget face `WidgetSize::size() × scale` points large.
pub fn face(kind: WidgetKind, size: WidgetSize, scale: f32, data: &WidgetData) -> AnyElement {
    let (width, height) = size.size();
    let tile = div()
        .relative()
        .flex_none()
        .w(px(width * scale))
        .h(px(height * scale))
        .rounded(px(widgets::RADIUS * scale))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(RIM));
    match kind {
        WidgetKind::Clock => tile.bg(rgba(GLASS)).child(clock(scale)).into_any_element(),
        WidgetKind::Calendar => tile
            .bg(rgba(GLASS))
            .child(calendar(scale))
            .into_any_element(),
        WidgetKind::Battery => tile
            .bg(rgba(GLASS))
            .child(battery(scale, data.battery))
            .into_any_element(),
        WidgetKind::Weather => weather(tile, scale, data.weather.as_ref()),
    }
}

fn clock(scale: f32) -> impl IntoElement {
    let now = Local::now();
    let (hour, minute, second) = widgets::clock_hands(now.hour(), now.minute(), now.second());
    // Measured: a white face inset 6 in the 112 gallery tile.
    let inset = 9.0 * scale;
    let diameter = widgets::SMALL * scale - 2.0 * inset;
    let radius = diameter / 2.0;
    div()
        .absolute()
        .left(px(inset))
        .top(px(inset))
        .size(px(diameter))
        .rounded_full()
        .bg(rgb(0xFFFFFF))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
                    let centre = point(bounds.origin.x + px(radius), bounds.origin.y + px(radius));
                    let hand = |angle: f32,
                                length: f32,
                                tail: f32,
                                width: f32,
                                color: Hsla,
                                window: &mut Window| {
                        let (sin, cos) = angle.to_radians().sin_cos();
                        let mut path = PathBuilder::stroke(px(width));
                        path.move_to(point(centre.x - px(tail * sin), centre.y + px(tail * cos)));
                        path.line_to(point(
                            centre.x + px(length * sin),
                            centre.y - px(length * cos),
                        ));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    };
                    let black: Hsla = rgb(0x000000).into();
                    let orange: Hsla = rgb(SECOND_HAND).into();
                    hand(hour, radius * 0.5, 0.0, 4.5 * scale, black, window);
                    hand(minute, radius * 0.8, 0.0, 4.5 * scale, black, window);
                    hand(
                        second,
                        radius * 0.88,
                        10.0 * scale,
                        1.2 * scale,
                        orange,
                        window,
                    );
                    let mut hub = PathBuilder::fill();
                    let steps = 16;
                    let hub_radius = 3.5 * scale;
                    for step in 0..=steps {
                        let t = step as f32 / steps as f32 * std::f32::consts::TAU;
                        let at = point(
                            centre.x + px(hub_radius * t.cos()),
                            centre.y + px(hub_radius * t.sin()),
                        );
                        if step == 0 {
                            hub.move_to(at);
                        } else {
                            hub.line_to(at);
                        }
                    }
                    hub.close();
                    if let Ok(hub) = hub.build() {
                        window.paint_path(hub, orange);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

fn calendar(scale: f32) -> impl IntoElement {
    let today = Local::now().date_naive();
    let grid = widgets::month_grid(today.year(), today.month(), today.day());
    let cell = 20.5 * scale;
    let day_cell = |value: Option<u8>| {
        let is_today = value == Some(grid.today);
        div()
            .w(px(cell))
            .h(px(17.0 * scale))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(17.0 * scale))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(is_today, |day| day.bg(rgb(CALENDAR_RED)))
                    .child(value.map(|day| day.to_string()).unwrap_or_default()),
            )
    };
    let header = widgets::WEEKDAY_INITIALS.iter().map(|initial| {
        div()
            .w(px(cell))
            .h(px(17.0 * scale))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(9.0 * scale))
            .font_weight(FontWeight::BOLD)
            .text_color(rgba(0xFFFFFF8C))
            .child(*initial)
    });
    let weeks = grid.cells.chunks(7).map(|week| {
        div()
            .flex()
            .children(week.iter().map(|value| day_cell(*value)))
    });
    div()
        .absolute()
        .inset_0()
        .pt(px(14.0 * scale))
        .px(px(13.0 * scale))
        .flex()
        .flex_col()
        .gap(px(3.0 * scale))
        .text_color(rgb(0xFFFFFF))
        .text_size(px(10.0 * scale))
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            div()
                .text_size(px(11.0 * scale))
                .line_height(px(13.0 * scale))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(CALENDAR_RED))
                .child(grid.title),
        )
        .child(div().flex().children(header))
        .children(weeks)
}

fn battery(scale: f32, battery: Option<Battery>) -> impl IntoElement {
    // Measured in the 112 gallery tile: ring centre (30, 30), outer radius
    // 22, stroke 3; "52%" ≈ 29 pt at x 10, 11 above the bottom.
    let ring = 67.0 * scale;
    let stroke = 4.5 * scale;
    let fraction = battery.map_or(0.0, |battery| f32::from(battery.percent) / 100.0);
    let charging = battery.is_some_and(|battery| battery.charging);
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .left(px(13.0 * scale))
                .top(px(12.0 * scale))
                .size(px(ring))
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
                            let radius = ring / 2.0 - stroke / 2.0;
                            let centre = point(
                                bounds.origin.x + px(ring / 2.0),
                                bounds.origin.y + px(ring / 2.0),
                            );
                            let arc = |to: f32, color: Hsla, window: &mut Window| {
                                if to <= 0.0005 {
                                    return;
                                }
                                let steps = (to * 180.0).ceil().max(2.0) as usize;
                                let mut path = PathBuilder::stroke(px(stroke));
                                for step in 0..=steps {
                                    let angle =
                                        to * step as f32 / steps as f32 * std::f32::consts::TAU;
                                    let at = point(
                                        centre.x + px(radius * angle.sin()),
                                        centre.y - px(radius * angle.cos()),
                                    );
                                    if step == 0 {
                                        path.move_to(at);
                                    } else {
                                        path.line_to(at);
                                    }
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, color);
                                }
                            };
                            arc(1.0, rgba(0xFFFFFF33).into(), window);
                            arc(fraction.clamp(0.0, 1.0), rgb(BATTERY_GREEN).into(), window);
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                // The laptop glyph.
                .child(
                    div()
                        .absolute()
                        .left(px(21.0 * scale))
                        .top(px(25.0 * scale))
                        .w(px(25.0 * scale))
                        .h(px(15.0 * scale))
                        .rounded(px(2.0 * scale))
                        .border(px(2.0 * scale))
                        .border_color(rgb(0xFFFFFF)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(17.0 * scale))
                        .top(px(41.0 * scale))
                        .w(px(33.0 * scale))
                        .h(px(2.5 * scale))
                        .rounded(px(1.25 * scale))
                        .bg(rgb(0xFFFFFF)),
                )
                .when(charging, |ring_box| {
                    ring_box.child(
                        div()
                            .absolute()
                            .left(px(ring / 2.0 - 6.0 * scale))
                            .top(px(-3.0 * scale))
                            .child(glyph("bolt", 12.0 * scale, 0xFFFFFF)),
                    )
                }),
        )
        .child(
            div()
                .absolute()
                .left(px(15.0 * scale))
                .bottom(px(14.0 * scale))
                .text_size(px(44.0 * scale))
                .line_height(px(44.0 * scale))
                .text_color(rgb(0xEDEDF2))
                .child(
                    battery
                        .map_or_else(|| "—".to_owned(), |battery| format!("{}%", battery.percent)),
                ),
        )
}

fn weather(
    tile: gpui::Div,
    scale: f32,
    weather: Option<&Result<WidgetWeather, Unavailable>>,
) -> AnyElement {
    let Some(Ok(weather)) = weather else {
        let message = match weather {
            Some(Err(Unavailable::NoPlace)) => "Open Weather to choose a place.",
            Some(Err(Unavailable::NoForecast)) => "Weather is unavailable.",
            _ => "",
        };
        return tile
            .bg(rgba(GLASS))
            .p(px(16.0 * scale))
            .flex()
            .flex_col()
            .justify_end()
            .text_size(px(12.0 * scale))
            .line_height(px(15.0 * scale))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgba(0xFFFFFFB3))
            .child(message)
            .into_any_element();
    };
    let (top, bottom) = weather.backdrop;
    let parts = rmac_weather::summary::sky_parts(weather.sky, weather.day);
    let icon = 16.0 * scale;
    tile.bg(gpui::linear_gradient(
        180.0,
        gpui::linear_color_stop(rgb(top), 0.0),
        gpui::linear_color_stop(rgb(bottom), 1.0),
    ))
    .text_color(rgb(0xFFFFFF))
    .child(
        div()
            .absolute()
            .left(px(16.0 * scale))
            .top(px(15.0 * scale))
            .right(px(12.0 * scale))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(14.0 * scale))
                    .line_height(px(17.0 * scale))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(weather.place.clone()),
            )
            .child(
                div()
                    .text_size(px(44.0 * scale))
                    .line_height(px(50.0 * scale))
                    .font_weight(FontWeight::LIGHT)
                    .child(weather.temperature.clone()),
            ),
    )
    .child(
        div()
            .absolute()
            .left(px(16.0 * scale))
            .right(px(12.0 * scale))
            .bottom(px(15.0 * scale))
            .flex()
            .flex_col()
            .text_size(px(12.0 * scale))
            .line_height(px(15.0 * scale))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                div()
                    .relative()
                    .size(px(icon))
                    .mb(px(2.0 * scale))
                    .children(parts.iter().map(|(name, color, dx, dy, part)| {
                        glyph(name, icon * part, *color)
                            .absolute()
                            .left(px(dx * icon))
                            .top(px(dy * icon))
                    })),
            )
            .child(div().truncate().child(weather.description))
            .child(div().child(weather.range.clone())),
    )
    .into_any_element()
}
