//! The Clock window: toolbar tabs, World Clock, Alarms, Stopwatch, Timers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::{
    canvas, div, img, point, prelude::FluentBuilder as _, px, rgb, rgba, svg, AppContext as _,
    Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, PathBuilder, Pixels, Render, RenderImage,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea,
};
use rmac_clock::alarms::{Alarm, Days};
use rmac_clock::changes::Change;
use rmac_clock::cities::{self, City};
use rmac_clock::countdown::{self, Entry, Field};
use rmac_clock::format::{self, WallTime};
use rmac_clock::map::{self, Land};
use rmac_clock::metrics as m;
use rmac_clock::solar::{self, Daylight};
use rmac_clock::stopwatch::{self, LapMark, Phase};
use rmac_clock::store::{self, State};
use rmac_clock::tz::{self, Zone};
use rmac_clock::{now_millis, schedule};
use rmac_ui::{mac, InputEvent, InputState, SearchField, TextField, Toggle};

use crate::{
    CloseWindow, LapReset, NewItem, ShowAlarms, ShowStopwatch, ShowTimers, ShowWorldClock,
    StartStop,
};

const LAND_SVG: &str = include_str!("../assets/world-land.svg");
/// Hundredths need a fast refresh while the Stopwatch runs on screen.
const FAST_TICK: Duration = Duration::from_millis(33);
const SLOW_TICK: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    World,
    Alarms,
    Stopwatch,
    Timers,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::World, "World Clock"),
    (Tab::Alarms, "Alarms"),
    (Tab::Stopwatch, "Stopwatch"),
    (Tab::Timers, "Timers"),
];

/// A rendered map and what it was rendered for.
struct MapImage {
    key: (usize, usize, i64),
    image: Arc<RenderImage>,
}

/// The Add/Edit Alarm sheet.
struct AlarmEditor {
    alarm: Alarm,
    is_new: bool,
    /// 0 = hour, 1 = minute.
    field: usize,
    label: Entity<InputState>,
}

pub(crate) struct ClockView {
    pub(crate) focus: FocusHandle,
    tab: Tab,
    state: State,
    state_mtime: Option<SystemTime>,
    zone: Zone,
    zones: HashMap<&'static str, Zone>,
    land: Option<Arc<Land>>,
    map: Option<MapImage>,
    map_pending: Option<(usize, usize, i64)>,
    garbage: Vec<Arc<RenderImage>>,
    picker: Option<Entity<InputState>>,
    editor: Option<AlarmEditor>,
    entry: Entry,
    entry_field: Field,
    timer_setup: bool,
    /// The saved state could not be read; edits are not saved over it.
    read_failed: bool,
    error: Option<SharedString>,
}

impl ClockView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        let (state, error) = match store::load() {
            Ok(state) => (state, None),
            Err(error) => (
                State::default(),
                Some(SharedString::from(format!(
                    "Clock could not read its saved alarms ({error})."
                ))),
            ),
        };
        let mut view = Self {
            focus: cx.focus_handle(),
            tab: Tab::World,
            state,
            state_mtime: state_mtime(),
            zone: tz::local_zone(),
            zones: HashMap::new(),
            land: Land::parse(LAND_SVG).map(Arc::new),
            map: None,
            map_pending: None,
            garbage: Vec::new(),
            picker: None,
            editor: None,
            entry: Entry::default(),
            entry_field: Field::Minutes,
            timer_setup: false,
            read_failed: error.is_some(),
            error,
        };
        if view.state.cities.is_none() && !view.read_failed {
            let local = tz::local_zone_name()
                .and_then(|name| cities::for_zone(&name))
                .map(|city| vec![city.name.to_owned()])
                .unwrap_or_default();
            view.change(Change::SetCities(local), cx);
        }
        // Make sure the ring schedule matches the saved state (for example
        // after an update moved the binary).
        view.persist(None, cx);
        view.start_ticker(cx);
        view
    }

    fn start_ticker(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            let fast = this
                .update(cx, |view, _| {
                    view.tab == Tab::Stopwatch && view.state.stopwatch.phase() == Phase::Running
                })
                .unwrap_or(false);
            cx.background_executor()
                .timer(if fast { FAST_TICK } else { SLOW_TICK })
                .await;
            if this
                .update(cx, |view, cx| {
                    view.reload_if_changed();
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    /// Pick up the ring process's edits (a one-time alarm switching off, a
    /// finished timer going away).
    fn reload_if_changed(&mut self) {
        let mtime = state_mtime();
        if mtime.is_some() && mtime != self.state_mtime {
            self.state_mtime = mtime;
            if let Ok(state) = store::load() {
                self.state = state;
            }
        }
    }

    /// Apply a change now and persist it (and the ring schedule) off the UI
    /// thread.
    fn change(&mut self, change: Change, cx: &mut Context<Self>) {
        change.apply(&mut self.state);
        self.persist(Some(change), cx);
        cx.notify();
    }

    fn persist(&mut self, change: Option<Change>, cx: &mut Context<Self>) {
        if self.read_failed && change.is_some() {
            // Never overwrite a state file we could not read.
            return;
        }
        let zone = self.zone.clone();
        let reschedule = change.as_ref().is_none_or(Change::affects_schedule);
        let task = cx.background_executor().spawn(async move {
            let now = now_millis();
            let state = match change {
                Some(change) => store::update(|state| change.apply(state)).map(|(state, ())| state),
                None => store::load(),
            }?;
            if reschedule {
                schedule::apply(&state, now, &|utc| zone.offset_at(utc))?;
            }
            std::io::Result::Ok(())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.state_mtime = state_mtime();
                if let Err(error) = result {
                    view.error = Some(SharedString::from(format!(
                        "Alarms and timers may not ring: {error}."
                    )));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn zone_for(&mut self, name: &'static str) -> Option<&Zone> {
        if !self.zones.contains_key(name) {
            let zone = Zone::load(name).ok()?;
            self.zones.insert(name, zone);
        }
        self.zones.get(name)
    }

    fn cities(&self) -> Vec<&'static City> {
        self.state
            .cities
            .iter()
            .flatten()
            .filter_map(|name| cities::find(name))
            .collect()
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.picker = None;
        cx.notify();
    }

    // ------------------------------------------------------------ actions

    fn new_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tab {
            Tab::World => self.open_picker(window, cx),
            Tab::Alarms => self.open_editor(None, window, cx),
            Tab::Timers => {
                self.timer_setup = true;
                cx.notify();
            }
            Tab::Stopwatch => {}
        }
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            self.picker = None;
            cx.notify();
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        input.update(cx, |state, cx| state.focus(window, cx));
        self.picker = Some(input);
        cx.notify();
    }

    fn open_editor(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = id.and_then(|id| self.state.alarms.iter().find(|alarm| alarm.id == id));
        let is_new = existing.is_none();
        let alarm = existing.cloned().unwrap_or_else(|| {
            let mut state = self.state.clone();
            Alarm {
                id: state.allocate_id(),
                ..Alarm::default()
            }
        });
        let label_text = alarm.label.clone();
        let label = cx.new(|cx| InputState::new(window, cx).placeholder("Alarm"));
        label.update(cx, |state, cx| state.set_value(label_text, window, cx));
        self.editor = Some(AlarmEditor {
            alarm,
            is_new,
            field: 0,
            label,
        });
        cx.notify();
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let mut alarm = editor.alarm;
        alarm.label = editor.label.read(cx).value().trim().to_string();
        alarm.enabled = true;
        alarm.snoozed_until = None;
        self.change(Change::SaveAlarm(alarm), cx);
    }

    fn start_stop(&mut self, cx: &mut Context<Self>) {
        let now = now_millis();
        match self.tab {
            Tab::Stopwatch => match self.state.stopwatch.phase() {
                Phase::Running => self.change(Change::StopwatchStop(now), cx),
                Phase::Idle | Phase::Paused => self.change(Change::StopwatchStart(now), cx),
            },
            Tab::Timers => self.start_timer(cx),
            _ => {}
        }
    }

    fn lap_reset(&mut self, cx: &mut Context<Self>) {
        let now = now_millis();
        match self.state.stopwatch.phase() {
            Phase::Running => self.change(Change::StopwatchLap(now), cx),
            Phase::Paused => self.change(Change::StopwatchReset, cx),
            Phase::Idle => {}
        }
    }

    fn start_timer(&mut self, cx: &mut Context<Self>) {
        if self.entry.is_zero() {
            return;
        }
        let id = self.state.clone().allocate_id();
        self.timer_setup = false;
        self.change(
            Change::StartTimer {
                id,
                duration: self.entry.milliseconds(),
                now: now_millis(),
            },
            cx,
        );
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform || keystroke.modifiers.control {
            return;
        }
        if let Some(editor) = self.editor.as_mut() {
            match keystroke.key.as_str() {
                "escape" => self.editor = None,
                "enter" => return self.save_editor(cx),
                "tab" | "left" | "right" => editor.field = 1 - editor.field,
                "up" => step_alarm(&mut editor.alarm, editor.field, 1),
                "down" => step_alarm(&mut editor.alarm, editor.field, -1),
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.picker.is_some() && keystroke.key == "escape" {
            self.picker = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        match (self.tab, keystroke.key.as_str()) {
            (Tab::Stopwatch, "space") => self.start_stop(cx),
            (Tab::Stopwatch, "l") => self.lap_reset(cx),
            (Tab::Timers, key) if self.showing_entry() => {
                let fields = [Field::Hours, Field::Minutes, Field::Seconds];
                let index = fields
                    .iter()
                    .position(|f| *f == self.entry_field)
                    .unwrap_or(1);
                match key {
                    "enter" | "space" => self.start_timer(cx),
                    "left" => self.entry_field = fields[index.saturating_sub(1)],
                    "right" | "tab" => self.entry_field = fields[(index + 1).min(2)],
                    "up" => self.entry.step(self.entry_field, 1),
                    "down" => self.entry.step(self.entry_field, -1),
                    "escape" if !self.state.timers.is_empty() => self.timer_setup = false,
                    digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                        self.entry
                            .type_digit(self.entry_field, digit.as_bytes()[0] - b'0');
                    }
                    _ => return,
                }
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn showing_entry(&self) -> bool {
        self.timer_setup || self.state.timers.is_empty()
    }

    // ------------------------------------------------------------ toolbar

    fn render_toolbar(
        &self,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (light_x, light_y) = m::TRAFFIC_LIGHT_CENTER;
        let tabs_left = ((width - m::TABS_WIDTH) / 2.0).round();
        let segments = TABS.iter().enumerate().map(|(index, (tab, label))| {
            let tab = *tab;
            let selected = self.tab == tab;
            div()
                .id(SharedString::from(format!("clock-tab-{index}")))
                .absolute()
                .left(px(m::TAB_LEFTS[index]))
                .top(px(1.0))
                .w(px(m::TAB_WIDTHS[index]))
                .h(px(m::TAB_HEIGHT))
                .rounded(px(m::TAB_HEIGHT / 2.0))
                .when(selected, |segment| segment.bg(rgb(m::TAB_SELECTED)))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(m::TAB_LABEL_SIZE))
                .text_color(mac::text())
                .child(*label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_tab(tab, cx)))
        });
        let add = (self.tab != Tab::Stopwatch).then(|| {
            div()
                .id("clock-add")
                .absolute()
                .right(px(m::ADD_RIGHT))
                .top(px(m::TABS_TOP))
                .size(px(m::ADD_DIAMETER))
                .rounded_full()
                .bg(rgb(m::CAPSULE_FILL))
                .border_1()
                .border_color(rgb(m::CAPSULE_RIM))
                .flex()
                .items_center()
                .justify_center()
                .active(|style| style.opacity(0.7))
                .child(
                    svg()
                        .path("icons/clock/plus.svg")
                        .size(px(m::ADD_GLYPH))
                        .text_color(mac::text()),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.new_item(window, cx)))
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(m::TOOLBAR_HEIGHT))
            .child(
                div()
                    .id("clock-drag")
                    .absolute()
                    .size_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_click(|event, window, _| {
                        // Double-clicking the title area zooms, as on macOS.
                        if event.click_count() >= 2 {
                            window.zoom_window();
                        }
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(light_x - mac::traffic_light_hit_width() / 2.0))
                    .top(px(light_y - mac::traffic_light_hit_height() / 2.0))
                    .child(rmac_ui::traffic_lights_active(window.is_window_active())),
            )
            .child(
                div()
                    .absolute()
                    .left(px(tabs_left))
                    .top(px(m::TABS_TOP))
                    .w(px(m::TABS_WIDTH))
                    .h(px(m::TABS_HEIGHT))
                    .rounded(px(m::TABS_HEIGHT / 2.0))
                    .bg(rgb(m::CAPSULE_FILL))
                    .border_1()
                    .border_color(rgb(m::CAPSULE_RIM))
                    .children(segments),
            )
            .children(add)
    }

    // ------------------------------------------------------------ world clock

    fn ensure_map(&mut self, width: f32, scale: f32, now: u64, cx: &mut Context<Self>) {
        let Some(land) = self.land.clone() else {
            return;
        };
        let pixel_width = (width * scale).round().max(1.0) as usize;
        let pixel_height = (map::height_for(width) * scale).round().max(1.0) as usize;
        let minute = (now / 60_000) as i64;
        let key = (pixel_width, pixel_height, minute);
        if self.map.as_ref().is_some_and(|map| map.key == key) || self.map_pending == Some(key) {
            return;
        }
        self.map_pending = Some(key);
        let task = cx.background_executor().spawn(async move {
            let mask = land.mask(width, pixel_width, pixel_height, scale);
            let sun = solar::subsolar((minute * 60) as f64);
            let pixels = map::paint(&mask, &sun, width, pixel_width, pixel_height, scale);
            image::RgbaImage::from_raw(pixel_width as u32, pixel_height as u32, pixels)
                .map(|buffer| Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        });
        cx.spawn(async move |this, cx| {
            let image = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.map_pending == Some(key) {
                    view.map_pending = None;
                }
                if let Some(image) = image {
                    if let Some(old) = view.map.replace(MapImage { key, image }) {
                        view.garbage.push(old.image);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_world(&mut self, width: f32, now: u64, cx: &mut Context<Self>) -> impl IntoElement {
        let map_height = map::height_for(width);
        let utc = (now / 1000) as i64;
        let local_offset = self.zone.offset_at(utc);
        let local_days = WallTime::at(utc, local_offset).days;
        let cities = self.cities();
        let mut pins = Vec::new();
        let mut cards = Vec::new();
        for (index, city) in cities.iter().enumerate() {
            let Some(offset) = self.zone_for(city.zone).map(|zone| zone.offset_at(utc)) else {
                continue;
            };
            let wall = WallTime::at(utc, offset);
            let time = format::time_12h(wall.hour, wall.minute);
            let (x, y) = map::project(width, city.longitude, city.latitude);
            pins.push(pin(city.name, &time, x, y));
            let daylight = solar::daylight(wall.days, city.latitude, city.longitude);
            let sun_line = |label: &str, time: Option<i64>| match time {
                Some(time) => {
                    let wall = WallTime::at(time, offset);
                    format!("{label}: {}", format::time_12h(wall.hour, wall.minute))
                }
                None => format!("{label}: —"),
            };
            let (rise, set) = match daylight {
                Daylight::Times { sunrise, sunset } => (Some(sunrise), Some(sunset)),
                Daylight::PolarDay | Daylight::PolarNight => (None, None),
            };
            let lines = [
                (format!("{}, {time}", city.name), mac::text()),
                (
                    format!(
                        "{}, {}",
                        format::relative_day(wall.days, local_days),
                        format::offset_difference(offset, local_offset)
                    ),
                    rgb(m::CARD_SECONDARY).into(),
                ),
                (sun_line("Sunrise", rise), rgb(m::CARD_SECONDARY).into()),
                (sun_line("Sunset", set), rgb(m::CARD_SECONDARY).into()),
            ];
            let name = city.name.to_owned();
            cards.push(
                div()
                    .id(SharedString::from(format!("clock-card-{index}")))
                    .group("card")
                    .relative()
                    .flex_none()
                    .w(px(m::CARD_WIDTH))
                    .h(px(m::CARD_HEIGHT))
                    .rounded(px(m::CARD_RADIUS))
                    .bg(rgb(m::CARD_FILL))
                    .child(clock_face(
                        m::CARD_WIDTH / 2.0,
                        m::FACE_TOP + m::FACE_DIAMETER / 2.0,
                        m::FACE_DIAMETER,
                        wall,
                    ))
                    .children(lines.into_iter().enumerate().map(|(line, (text, color))| {
                        div()
                            .absolute()
                            .left_0()
                            .w_full()
                            .top(px(m::CARD_LINES_TOP + line as f32 * m::CARD_LINE))
                            .h(px(m::CARD_LINE))
                            .text_size(px(m::CARD_TEXT_SIZE))
                            .line_height(px(m::CARD_LINE))
                            .text_color(color)
                            .flex()
                            .justify_center()
                            .whitespace_nowrap()
                            .child(text)
                    }))
                    .child(
                        div()
                            .id(SharedString::from(format!("clock-card-remove-{index}")))
                            .absolute()
                            .left(px(8.0))
                            .top(px(8.0))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(rgb(0x5A5A5A))
                            .opacity(0.0)
                            .group_hover("card", |style| style.opacity(1.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(mac::text())
                            .child("×")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.change(Change::RemoveCity(name.clone()), cx);
                            })),
                    ),
            );
        }
        let map_image = self.map.as_ref().map(|map| map.image.clone());
        div()
            .id("clock-world")
            .absolute()
            .top(px(m::TOOLBAR_HEIGHT))
            .left_0()
            .w_full()
            .bottom_0()
            .overflow_y_scroll()
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(map_height))
                    .bg(rgb(map::OCEAN))
                    .when_some(map_image, |band, image| {
                        band.child(img(image).absolute().top_0().left_0().w_full().h_full())
                    })
                    .children(pins),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(m::CARD_GAP))
                    .pl(px(m::CARD_LEFT))
                    .pr(px(m::CARD_LEFT))
                    .pt(px(m::CARD_GAP_BELOW_MAP))
                    .pb(px(m::CARD_GAP))
                    .children(cards),
            )
    }

    fn render_picker(&self, width: f32, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let input = self.picker.as_ref()?;
        let query = input.read(cx).value().to_string();
        let current = self.state.cities.clone().unwrap_or_default();
        let results = cities::search(&query)
            .into_iter()
            .filter(|city| !current.iter().any(|name| name == city.name))
            .take(8)
            .enumerate()
            .map(|(index, city)| {
                let name = city.name.to_owned();
                div()
                    .id(SharedString::from(format!("clock-city-{index}")))
                    .h(px(36.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .hover(|style| style.bg(mac::accent()))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(mac::text())
                            .child(city.name),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(m::SECONDARY_TEXT))
                            .child(city.country),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.picker = None;
                        this.change(Change::AddCity(name.clone()), cx);
                    }))
            })
            .collect::<Vec<_>>();
        let panel_width = 280.0;
        Some(
            div()
                .id("clock-picker")
                .absolute()
                .top(px(m::TOOLBAR_HEIGHT - 4.0))
                .left(px(width - panel_width - m::ADD_RIGHT))
                .w(px(panel_width))
                .p(px(6.0))
                .rounded(px(mac::radius_popover()))
                .bg(mac::material_popover())
                .border_1()
                .border_color(mac::separator())
                .shadow_lg()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(SearchField::new(input).small())
                .children(results)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.picker = None;
                    cx.notify();
                })),
        )
    }

    // ------------------------------------------------------------ alarms

    fn render_alarms(&self, now: u64, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div()
            .id("clock-alarms")
            .absolute()
            .top(px(m::TOOLBAR_HEIGHT))
            .left_0()
            .w_full()
            .bottom_0();
        if self.state.alarms.is_empty() {
            return body
                .child(
                    svg()
                        .absolute()
                        .top(px(m::EMPTY_GLYPH_TOP - m::TOOLBAR_HEIGHT))
                        .left(px(0.0))
                        .w_full()
                        .h(px(m::EMPTY_GLYPH))
                        .path("icons/clock/alarm.svg")
                        .text_color(rgb(m::EMPTY_GLYPH_FILL)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(m::EMPTY_LABEL_TOP - m::TOOLBAR_HEIGHT))
                        .w_full()
                        .flex()
                        .justify_center()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(m::EMPTY_LABEL))
                        .child("No Alarms"),
                );
        }
        let utc = (now / 1000) as i64;
        let zone = self.zone.clone();
        let rows = self.state.alarms.iter().map(|alarm| {
            let id = alarm.id;
            let enabled = alarm.enabled;
            let (display, suffix) = hour_12(alarm.hour);
            let mut subtitle = alarm.title().to_owned();
            let repeat = alarm.repeat.label();
            if !repeat.is_empty() {
                subtitle.push_str(", ");
                subtitle.push_str(&repeat);
            }
            if let Some(until) = alarm.snoozed_until.filter(|&until| until > utc) {
                let wall = WallTime::at(until, zone.offset_at(until));
                subtitle.push_str(&format!(
                    " · Snoozed until {}",
                    format::time_12h(wall.hour, wall.minute)
                ));
            }
            let color: Hsla = if enabled {
                mac::text()
            } else {
                rgb(0x8A8A8A).into()
            };
            div()
                .id(SharedString::from(format!("clock-alarm-{id}")))
                .relative()
                .mx(px(m::ALARM_ROW_INSET))
                .h(px(m::ALARM_ROW_HEIGHT))
                .border_b_1()
                .border_color(rgb(m::ALARM_SEPARATOR))
                .child(
                    div()
                        .absolute()
                        .left(px(4.0))
                        .top(px(8.0))
                        .flex()
                        .items_baseline()
                        .gap(px(4.0))
                        .text_color(color)
                        .font_features(mac::tabular_font_features())
                        .child(
                            div()
                                .text_size(px(m::ALARM_TIME_SIZE))
                                .line_height(px(56.0))
                                .font_weight(FontWeight::LIGHT)
                                .child(format!("{display}:{:02}", alarm.minute)),
                        )
                        .child(div().text_size(px(26.0)).child(suffix)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(6.0))
                        .top(px(62.0))
                        .text_size(px(13.0))
                        .text_color(rgb(m::SECONDARY_TEXT))
                        .child(subtitle),
                )
                .child(
                    div().absolute().right(px(6.0)).top(px(30.0)).child(
                        Toggle::new(SharedString::from(format!("clock-alarm-toggle-{id}")))
                            .checked(enabled)
                            .on_click({
                                let view = cx.entity().downgrade();
                                move |checked, _, cx| {
                                    let checked = *checked;
                                    let _ = view.update(cx, |this, cx| {
                                        this.change(Change::SetAlarmEnabled(id, checked), cx)
                                    });
                                }
                            }),
                    ),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        this.open_editor(Some(id), window, cx);
                    }
                }))
        });
        body.overflow_y_scroll()
            .pt(px(8.0))
            .children(rows.collect::<Vec<_>>())
    }

    fn render_editor(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let editor = self.editor.as_ref()?;
        let alarm = &editor.alarm;
        let (display, suffix) = hour_12(alarm.hour);
        let field = |index: usize, text: String| {
            div()
                .id(SharedString::from(format!("clock-editor-field-{index}")))
                .px(px(6.0))
                .rounded(px(8.0))
                .when(editor.field == index, |group| group.bg(rgb(m::BUTTON_FILL)))
                .child(text)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        editor.field = index;
                        cx.notify();
                    }
                }))
                .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                    let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
                    if let Some(editor) = this.editor.as_mut() {
                        if delta.abs() >= 1.0 {
                            step_alarm(&mut editor.alarm, index, if delta > 0.0 { 1 } else { -1 });
                            cx.notify();
                        }
                    }
                }))
        };
        let days = (0..7u32).map(|day| {
            let on = alarm.repeat.contains(day);
            div()
                .id(SharedString::from(format!("clock-editor-day-{day}")))
                .size(px(30.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(mac::text())
                .bg(if on {
                    mac::accent()
                } else {
                    rgb(m::BUTTON_FILL).into()
                })
                .child(Days::NAMES[day as usize][..1].to_owned())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        editor.alarm.repeat = editor.alarm.repeat.toggled(day);
                        cx.notify();
                    }
                }))
        });
        let snooze = alarm.snooze;
        let button = |id: &'static str, label: &'static str, fill: Hsla| {
            div()
                .id(id)
                .h(px(28.0))
                .px(px(16.0))
                .rounded(px(14.0))
                .bg(fill)
                .flex()
                .items_center()
                .text_size(px(13.0))
                .text_color(mac::text())
                .active(|style| style.opacity(0.7))
                .child(label)
        };
        let panel_width = 340.0;
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(width))
                .h(px(height))
                .bg(rgba(0x0000_0059))
                .child(
                    div()
                        .id("clock-editor")
                        .absolute()
                        .left(px((width - panel_width) / 2.0))
                        .top(px(m::TOOLBAR_HEIGHT + 24.0))
                        .w(px(panel_width))
                        .p(px(20.0))
                        .rounded(px(mac::radius_large_surface()))
                        .bg(mac::sheet())
                        .border_1()
                        .border_color(mac::separator())
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .gap(px(16.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(mac::text())
                                .child(if editor.is_new {
                                    "Add Alarm"
                                } else {
                                    "Edit Alarm"
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(2.0))
                                .text_size(px(48.0))
                                .font_weight(FontWeight::LIGHT)
                                .font_features(mac::tabular_font_features())
                                .text_color(mac::text())
                                .child(field(0, display.to_string()))
                                .child(":")
                                .child(field(1, format!("{:02}", alarm.minute)))
                                .child(
                                    div()
                                        .id("clock-editor-ampm")
                                        .ml(px(8.0))
                                        .px(px(8.0))
                                        .rounded(px(8.0))
                                        .bg(rgb(m::BUTTON_FILL))
                                        .text_size(px(20.0))
                                        .child(suffix)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            if let Some(editor) = this.editor.as_mut() {
                                                editor.alarm.hour = (editor.alarm.hour + 12) % 24;
                                                cx.notify();
                                            }
                                        })),
                                ),
                        )
                        .child(labelled("Repeat", div().flex().gap(px(6.0)).children(days)))
                        .child(labelled(
                            "Label",
                            TextField::new(&editor.label).small().w(px(200.0)),
                        ))
                        .child(labelled(
                            "Snooze",
                            Toggle::new("clock-editor-snooze")
                                .checked(snooze)
                                .on_click({
                                    let view = cx.entity().downgrade();
                                    move |checked, _, cx| {
                                        let checked = *checked;
                                        let _ = view.update(cx, |this, cx| {
                                            if let Some(editor) = this.editor.as_mut() {
                                                editor.alarm.snooze = checked;
                                                cx.notify();
                                            }
                                        });
                                    }
                                }),
                        ))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.0))
                                .when(!editor.is_new, |row| {
                                    let id = alarm.id;
                                    row.child(
                                        button(
                                            "clock-editor-delete",
                                            "Delete",
                                            rgb(m::STOP_FILL).into(),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                                this.editor = None;
                                                this.change(Change::RemoveAlarm(id), cx);
                                            }),
                                        ),
                                    )
                                })
                                .child(div().flex_1())
                                .child(
                                    button(
                                        "clock-editor-cancel",
                                        "Cancel",
                                        rgb(m::BUTTON_FILL).into(),
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, _, cx| {
                                            this.editor = None;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    button("clock-editor-save", "Save", mac::accent()).on_click(
                                        cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.save_editor(cx)
                                        }),
                                    ),
                                ),
                        ),
                ),
        )
    }

    // ------------------------------------------------------------ stopwatch

    fn render_stopwatch(
        &self,
        width: f32,
        height: f32,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let watch = &self.state.stopwatch;
        let phase = watch.phase();
        let table_left = ((width - m::LAP_TABLE_WIDTH) / 2.0).round();
        let rows = watch.rows(now).into_iter().enumerate().map(|(index, row)| {
            let color: Hsla = match row.mark {
                LapMark::Plain => mac::text(),
                LapMark::Fastest => rgb(m::LAP_FASTEST).into(),
                LapMark::Slowest => rgb(m::LAP_SLOWEST).into(),
            };
            let top = m::LAP_RULE_TOP + 1.0 + index as f32 * m::LAP_ROW_HEIGHT;
            div()
                .absolute()
                .left(px(table_left))
                .top(px(top))
                .w(px(m::LAP_TABLE_WIDTH))
                .h(px(m::LAP_ROW_HEIGHT))
                .border_b_1()
                .border_color(rgb(m::ALARM_SEPARATOR))
                .text_size(px(m::LAP_TEXT_SIZE))
                .line_height(px(m::LAP_ROW_HEIGHT))
                .text_color(color)
                .child(
                    div()
                        .absolute()
                        .left(px(1.0))
                        .child(format!("Lap {}", row.number)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(m::SPLIT_COLUMN_LEFT))
                        .w(px(m::SPLIT_COLUMN_WIDTH))
                        .flex()
                        .justify_center()
                        .child(stopwatch::format(row.split)),
                )
                .child(
                    div()
                        .absolute()
                        .right(px(1.5))
                        .child(stopwatch::format(row.total)),
                )
        });
        let buttons_top = height - m::STOPWATCH_BUTTONS_BOTTOM - m::BUTTON_HEIGHT;
        let (left_label, left_enabled) = match phase {
            Phase::Idle => ("Lap", false),
            Phase::Running => ("Lap", true),
            Phase::Paused => ("Reset", true),
        };
        let (right_label, right_fill) = match phase {
            Phase::Running => ("Stop", m::STOP_FILL),
            Phase::Idle | Phase::Paused => ("Start", m::START_FILL),
        };
        let header = |text: &'static str| {
            div()
                .text_size(px(m::LAP_HEADER_SIZE))
                .line_height(px(13.0))
                .text_color(rgb(m::LAP_HEADER))
                .child(text)
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(width))
            .h(px(height))
            .font_features(mac::tabular_font_features())
            .child(digits(
                m::STOPWATCH_DIGITS_TOP,
                stopwatch::format(watch.elapsed(now)),
            ))
            .child(
                div()
                    .absolute()
                    .left(px(table_left))
                    .top(px(m::LAP_HEADER_TOP))
                    .w(px(m::LAP_TABLE_WIDTH))
                    .child(div().absolute().left(px(1.0)).child(header("Lap No.")))
                    .child(
                        div()
                            .absolute()
                            .left(px(m::SPLIT_COLUMN_LEFT))
                            .w(px(m::SPLIT_COLUMN_WIDTH))
                            .flex()
                            .justify_center()
                            .child(header("Split")),
                    )
                    .child(div().absolute().right(px(1.5)).child(header("Total"))),
            )
            .child(
                div()
                    .absolute()
                    .left(px(table_left))
                    .top(px(m::LAP_RULE_TOP))
                    .w(px(m::LAP_TABLE_WIDTH))
                    .h(px(0.5))
                    .bg(rgb(m::LAP_RULE)),
            )
            .child(
                div()
                    .id("clock-laps")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px((buttons_top - 12.0).max(0.0)))
                    .overflow_hidden()
                    .children(rows),
            )
            .child(button_pair(
                width,
                buttons_top,
                ("clock-lap", left_label, left_enabled, m::BUTTON_FILL),
                ("clock-start", right_label, true, right_fill),
                cx.listener(|this, _: &ClickEvent, _, cx| this.lap_reset(cx)),
                cx.listener(|this, _: &ClickEvent, _, cx| this.start_stop(cx)),
            ))
    }

    // ------------------------------------------------------------ timers

    fn render_timers(
        &self,
        width: f32,
        height: f32,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let body = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(width))
            .h(px(height))
            .font_features(mac::tabular_font_features());
        if self.showing_entry() {
            let scale = width / m::WINDOW.0;
            let labels = ["hr", "min", "sec"]
                .into_iter()
                .enumerate()
                .map(|(index, label)| {
                    div()
                        .absolute()
                        .left(px(m::TIMER_LABEL_CENTERS[index] * scale - 20.0))
                        .top(px(m::TIMER_LABEL_TOP))
                        .w(px(40.0))
                        .flex()
                        .justify_center()
                        .text_size(px(m::TIMER_LABEL_SIZE))
                        .text_color(rgb(m::SECONDARY_TEXT))
                        .child(label)
                });
            let fields = [
                (Field::Hours, self.entry.hours),
                (Field::Minutes, self.entry.minutes),
                (Field::Seconds, self.entry.seconds),
            ];
            let groups = fields
                .into_iter()
                .enumerate()
                .flat_map(|(index, (field, value))| {
                    let selected = self.entry_field == field;
                    let group = div()
                        .id(SharedString::from(format!("clock-entry-{index}")))
                        .rounded(px(12.0))
                        .when(selected, |group| group.bg(rgb(0x2A2A2A)))
                        .child(format!("{value:02}"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.entry_field = field;
                            cx.notify();
                        }))
                        .on_scroll_wheel(cx.listener(
                            move |this, event: &ScrollWheelEvent, _, cx| {
                                let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
                                if delta.abs() >= 1.0 {
                                    this.entry_field = field;
                                    this.entry.step(field, if delta > 0.0 { 1 } else { -1 });
                                    cx.notify();
                                }
                            },
                        ))
                        .into_any_element();
                    let colon = (index < 2).then(|| div().child(":").into_any_element());
                    std::iter::once(group).chain(colon)
                });
            let cancellable = !self.state.timers.is_empty();
            return body
                .children(labels)
                .child(
                    div()
                        .absolute()
                        .top(px(m::TIMER_DIGITS_TOP))
                        .w_full()
                        .h(px(m::DIGITS_LINE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(m::DIGITS_SIZE))
                        .line_height(px(m::DIGITS_LINE))
                        .font_weight(FontWeight::THIN)
                        .text_color(mac::text())
                        .children(groups),
                )
                .child(button_pair(
                    width,
                    m::TIMER_BUTTONS_TOP,
                    ("clock-timer-cancel", "Cancel", cancellable, m::BUTTON_FILL),
                    (
                        "clock-timer-start",
                        "Start",
                        !self.entry.is_zero(),
                        m::START_FILL,
                    ),
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        if !this.state.timers.is_empty() {
                            this.timer_setup = false;
                            cx.notify();
                        }
                    }),
                    cx.listener(|this, _: &ClickEvent, _, cx| this.start_timer(cx)),
                ));
        }
        // Running and paused timers, newest first, as rings.
        let zone = self.zone.clone();
        let count = self.state.timers.len().max(1) as f32;
        let diameter = (m::RING_DIAMETER / count.sqrt()).max(160.0);
        let cards = self.state.timers.iter().rev().map(|timer| {
            let id = timer.id;
            let remaining = timer.remaining(now);
            let fraction = timer.fraction_left(now);
            let running = timer.is_running();
            let end_text = timer.ends_at().map(|ends_at| {
                let seconds = ends_at.div_ceil(1000) as i64;
                let wall = WallTime::at(seconds, zone.offset_at(seconds));
                format::time_12h(wall.hour, wall.minute)
            });
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(18.0))
                .child(
                    div()
                        .relative()
                        .size(px(diameter))
                        .child(ring(diameter, fraction))
                        .child(
                            div()
                                .absolute()
                                .size_full()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(px(
                                            m::RING_DIGITS_SIZE * diameter / m::RING_DIAMETER
                                        ))
                                        .font_weight(FontWeight::THIN)
                                        .text_color(mac::text())
                                        .child(countdown::remaining_text(remaining)),
                                )
                                .when_some(end_text, |column, end| {
                                    column.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.0))
                                            .text_size(px(13.0))
                                            .text_color(rgb(m::SECONDARY_TEXT))
                                            .child(
                                                svg()
                                                    .path("icons/clock/bell.svg")
                                                    .size(px(12.0))
                                                    .text_color(rgb(m::SECONDARY_TEXT)),
                                            )
                                            .child(end),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(m::BUTTON_GAP))
                        .child(
                            capsule(
                                SharedString::from(format!("clock-timer-cancel-{id}")),
                                "Cancel",
                                true,
                                m::BUTTON_FILL,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.change(Change::CancelTimer(id), cx)
                                },
                            )),
                        )
                        .child(
                            capsule(
                                SharedString::from(format!("clock-timer-pause-{id}")),
                                if running { "Pause" } else { "Resume" },
                                true,
                                if running {
                                    m::PAUSE_FILL
                                } else {
                                    m::START_FILL
                                },
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    let now = now_millis();
                                    let change = if running {
                                        Change::PauseTimer { id, now }
                                    } else {
                                        Change::ResumeTimer { id, now }
                                    };
                                    this.change(change, cx);
                                },
                            )),
                        ),
                )
        });
        body.child(
            div()
                .id("clock-timers")
                .absolute()
                .top(px(m::TOOLBAR_HEIGHT))
                .left_0()
                .w_full()
                .bottom_0()
                .overflow_y_scroll()
                .flex()
                .flex_wrap()
                .justify_center()
                .content_center()
                .gap(px(40.0))
                .p(px(24.0))
                .children(cards.collect::<Vec<_>>()),
        )
    }
}

fn state_mtime() -> Option<SystemTime> {
    std::fs::metadata(store::state_path()?)
        .ok()?
        .modified()
        .ok()
}

/// (1–12, "AM"/"PM") for a 0–23 hour.
fn hour_12(hour: u8) -> (u8, &'static str) {
    match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    }
}

fn step_alarm(alarm: &mut Alarm, field: usize, delta: i32) {
    if field == 0 {
        alarm.hour = (i32::from(alarm.hour) + delta).rem_euclid(24) as u8;
    } else {
        alarm.minute = (i32::from(alarm.minute) + delta).rem_euclid(60) as u8;
    }
}

fn labelled(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(13.0))
                .text_color(mac::text_secondary())
                .child(label),
        )
        .child(control)
}

fn digits(top: f32, text: String) -> impl IntoElement {
    div()
        .absolute()
        .top(px(top))
        .left_0()
        .w_full()
        .h(px(m::DIGITS_LINE))
        .flex()
        .justify_center()
        .text_size(px(m::DIGITS_SIZE))
        .line_height(px(m::DIGITS_LINE))
        .font_weight(FontWeight::THIN)
        .text_color(mac::text())
        .child(text)
}

fn capsule(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    enabled: bool,
    fill: u32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(m::BUTTON_WIDTH))
        .h(px(m::BUTTON_HEIGHT))
        .rounded(px(m::BUTTON_HEIGHT / 2.0))
        .bg(rgb(if enabled {
            fill
        } else {
            m::BUTTON_DISABLED_FILL
        }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(m::BUTTON_TEXT_SIZE))
        .text_color(if enabled {
            mac::text()
        } else {
            rgb(m::BUTTON_DISABLED_TEXT).into()
        })
        .when(enabled, |button| button.active(|style| style.opacity(0.75)))
        .child(label)
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

/// The measured pair of 150 × 28 capsules, centred 20 apart.
fn button_pair(
    width: f32,
    top: f32,
    left: (&'static str, &'static str, bool, u32),
    right: (&'static str, &'static str, bool, u32),
    on_left: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_right: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let start = ((width - 2.0 * m::BUTTON_WIDTH - m::BUTTON_GAP) / 2.0).round();
    let (on_left, on_right): (ClickHandler, ClickHandler) = (Box::new(on_left), Box::new(on_right));
    div()
        .absolute()
        .top(px(top))
        .left(px(start))
        .flex()
        .gap(px(m::BUTTON_GAP))
        .child(
            capsule(left.0, left.1, left.2, left.3).when(left.2, |button| {
                button.on_click(move |event, window, cx| on_left(event, window, cx))
            }),
        )
        .child(
            capsule(right.0, right.1, right.2, right.3).when(right.2, |button| {
                button.on_click(move |event, window, cx| on_right(event, window, cx))
            }),
        )
}

fn pin(name: &'static str, time: &str, x: f32, y: f32) -> impl IntoElement {
    div()
        .absolute()
        .left(px(x - m::PIN_DIAMETER / 2.0))
        .top(px(y - m::PIN_DIAMETER / 2.0))
        .child(
            div()
                .size(px(m::PIN_DIAMETER))
                .rounded_full()
                .bg(rgb(m::PIN_FILL)),
        )
        .child(
            div()
                .absolute()
                .left(px(m::PIN_DIAMETER + 3.5))
                .top(px(-4.5))
                .text_size(px(m::PIN_NAME_SIZE))
                .line_height(px(15.0))
                .font_weight(FontWeight::BOLD)
                .text_color(mac::text())
                .whitespace_nowrap()
                .child(name),
        )
        .child(
            div()
                .absolute()
                .left(px(m::PIN_DIAMETER + 3.5))
                .top(px(10.5))
                .text_size(px(m::PIN_TIME_SIZE))
                .line_height(px(12.0))
                .text_color(mac::text())
                .whitespace_nowrap()
                .child(time.to_owned()),
        )
}

/// An analogue face: white disc, numerals, black hands and the orange
/// second hand.
fn clock_face(center_x: f32, center_y: f32, diameter: f32, time: WallTime) -> impl IntoElement {
    let radius = diameter / 2.0;
    let numerals = (1..=12).map(move |hour| {
        let angle = hour as f32 * std::f32::consts::PI / 6.0;
        let distance = radius - 15.0;
        div()
            .absolute()
            .left(px(center_x + distance * angle.sin() - 10.0))
            .top(px(center_y - distance * angle.cos() - 8.0))
            .w(px(20.0))
            .h(px(16.0))
            .flex()
            .justify_center()
            .text_size(px(m::NUMERAL_SIZE))
            .line_height(px(16.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(0x000000))
            .child(hour.to_string())
    });
    let (hour, minute, second) = format::hand_angles(time);
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(
            div()
                .absolute()
                .left(px(center_x - radius))
                .top(px(center_y - radius))
                .size(px(diameter))
                .rounded_full()
                .bg(rgb(0xFFFFFF)),
        )
        .children(numerals)
        .child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, (), window, _| {
                    let origin = bounds.origin;
                    let center = point(origin.x + px(center_x), origin.y + px(center_y));
                    let hand = |angle: f32,
                                length: f32,
                                tail: f32,
                                width: f32,
                                color: Hsla,
                                window: &mut Window| {
                        let radians = angle.to_radians();
                        let (sin, cos) = radians.sin_cos();
                        let mut path = PathBuilder::stroke(px(width));
                        path.move_to(point(center.x - px(tail * sin), center.y + px(tail * cos)));
                        path.line_to(point(
                            center.x + px(length * sin),
                            center.y - px(length * cos),
                        ));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    };
                    let black: Hsla = rgb(0x000000).into();
                    let orange: Hsla = rgb(m::SECOND_HAND).into();
                    hand(hour, radius * 0.5, 0.0, 3.0, black, window);
                    hand(minute, radius * 0.8, 0.0, 2.5, black, window);
                    hand(second, radius * 0.88, 12.0, 1.0, orange, window);
                    let mut dot = PathBuilder::fill();
                    let steps = 16;
                    for step in 0..=steps {
                        let t = step as f32 / steps as f32 * std::f32::consts::TAU;
                        let at = point(center.x + px(3.0 * t.cos()), center.y + px(3.0 * t.sin()));
                        if step == 0 {
                            dot.move_to(at);
                        } else {
                            dot.line_to(at);
                        }
                    }
                    dot.close();
                    if let Ok(dot) = dot.build() {
                        window.paint_path(dot, orange);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

/// A countdown ring: grey track and the orange part still left.
fn ring(diameter: f32, fraction: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, _| {
            let radius = diameter / 2.0 - m::RING_WIDTH / 2.0;
            let center = point(
                bounds.origin.x + px(diameter / 2.0),
                bounds.origin.y + px(diameter / 2.0),
            );
            let arc = |from: f32, to: f32, color: Hsla, window: &mut Window| {
                if to - from <= 0.0005 {
                    return;
                }
                let steps = ((to - from) * 180.0).ceil().max(2.0) as usize;
                let mut path = PathBuilder::stroke(px(m::RING_WIDTH));
                for step in 0..=steps {
                    let t = from + (to - from) * step as f32 / steps as f32;
                    let angle = t * std::f32::consts::TAU;
                    let at = point(
                        center.x + px(radius * angle.sin()),
                        center.y - px(radius * angle.cos()),
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
            arc(0.0, 1.0, rgb(m::RING_TRACK).into(), window);
            // The coloured arc shrinks clockwise from 12 o'clock.
            arc(
                1.0 - fraction.clamp(0.0, 1.0),
                1.0,
                rgb(m::RING_FILL).into(),
                window,
            );
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Render for ClockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.garbage.drain(..) {
            cx.drop_image(image, Some(window));
        }
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height));
        let now = now_millis();
        if self.tab == Tab::World {
            self.ensure_map(width, window.scale_factor(), now, cx);
        }
        let content = match self.tab {
            Tab::World => self.render_world(width, now, cx).into_any_element(),
            Tab::Alarms => self.render_alarms(now, cx).into_any_element(),
            Tab::Stopwatch => self
                .render_stopwatch(width, height, now, cx)
                .into_any_element(),
            Tab::Timers => self
                .render_timers(width, height, now, cx)
                .into_any_element(),
        };
        let picker = self.render_picker(width, cx);
        let editor = self.render_editor(width, height, cx);
        let error = self.error.clone().map(|message| {
            div()
                .absolute()
                .bottom(px(12.0))
                .left(px(16.0))
                .right(px(16.0))
                .flex()
                .justify_center()
                .text_size(px(12.0))
                .text_color(mac::danger())
                .child(message)
        });
        div()
            .id("clock")
            .track_focus(&self.focus)
            .key_context("Clock")
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key_down(event, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowWorldClock, _, cx| this.set_tab(Tab::World, cx)))
            .on_action(cx.listener(|this, _: &ShowAlarms, _, cx| this.set_tab(Tab::Alarms, cx)))
            .on_action(
                cx.listener(|this, _: &ShowStopwatch, _, cx| this.set_tab(Tab::Stopwatch, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowTimers, _, cx| this.set_tab(Tab::Timers, cx)))
            .on_action(cx.listener(|this, _: &NewItem, window, cx| this.new_item(window, cx)))
            .on_action(cx.listener(|this, _: &StartStop, _, cx| this.start_stop(cx)))
            .on_action(cx.listener(|this, _: &LapReset, _, cx| this.lap_reset(cx)))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(m::WINDOW_FILL))
            .child(content)
            .child(self.render_toolbar(width, window, cx))
            .children(picker)
            .children(error)
            .children(editor)
    }
}
