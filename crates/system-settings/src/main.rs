//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · Apple Account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive: General/Appearance/Sound/Wi-Fi/Bluetooth carry real controls
//! (Switch toggles, Sliders, segmented pickers) whose state is held in the view.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Where it is safe and read-only, panes reflect real macOS state
//! (appearance, computer name, macOS version, chip, memory).

mod storage;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Command;

use gpui::{
    actions, div, prelude::FluentBuilder as _, px, svg, AnyElement, App, AppContext as _,
    AssetSource, Context, Div, ElementId, Entity, FocusHandle, Hsla, InteractiveElement as _,
    IntoElement, KeyBinding, MouseButton, ParentElement, Render, Result, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Svg, Window,
};
use gpui_component::slider::{Slider, SliderState};
use gpui_component::switch::Switch;
use gpui_component::StyledExt as _;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct AppAssets;

struct CombinedAssets;
impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(f) = AppAssets::get(path) {
            return Ok(Some(f.data));
        }
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = AppAssets::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(p.to_string()))
            .collect();
        if let Ok(mut o) = gpui_component_assets::Assets.list(path) {
            v.append(&mut o);
        }
        Ok(v)
    }
}

actions!(system_settings, [GoBack]);

fn hsl(h: u32) -> Hsla {
    gpui::rgb(h).into()
}
fn sidebar_bg() -> Hsla {
    hsl(0xe7e7ea)
}
fn pane_bg() -> Hsla {
    hsl(0xf2f2f4)
}
fn card_bg() -> Hsla {
    hsl(0xffffff)
}
fn accent() -> Hsla {
    hsl(0x0a84ff)
}
fn label() -> Hsla {
    hsl(0x1d1d1f)
}
fn secondary() -> Hsla {
    hsl(0x86868b)
}
fn sep() -> Hsla {
    hsl(0xe5e5e5)
}
fn white() -> Hsla {
    gpui::white()
}

fn glyph(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(color)
        .flex_none()
}

/// A colored rounded-square icon tile (SF-symbol-on-color, like Settings).
fn tile(path: &'static str, bg: Hsla, size: f32) -> impl IntoElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.28))
        .bg(bg)
        .child(glyph(path, size * 0.62, white()))
}

#[derive(Clone)]
struct Row {
    icon: &'static str,
    color: Hsla,
    label: SharedString,
}

#[derive(Clone)]
struct Category {
    name: SharedString,
    icon: &'static str,
    color: Hsla,
    desc: SharedString,
    cards: Vec<Vec<Row>>,
}

// ---- interactive state enums --------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Appearance {
    Light,
    Dark,
    Auto,
}

/// A navigation subpage pushed onto the back stack from a row chevron.
#[derive(Clone)]
enum SubPage {
    About,
    SoftwareUpdate,
    Storage,
    /// A generic placeholder detail page identified by its row label.
    Placeholder {
        icon: &'static str,
        color: Hsla,
        title: SharedString,
    },
}

/// Real, read-only macOS facts gathered after the first frame.
#[derive(Default)]
struct SysInfo {
    computer_name: String,
    os: String,
    chip: String,
    memory: String,
    model: String,
    serial: String,
}

/// A real Bluetooth device parsed from `system_profiler SPBluetoothDataType`.
#[derive(Clone)]
struct BtDevice {
    name: String,
    connected: bool,
    kind: String,
}

/// Live battery readings (read once after launch via `pmset` + `ioreg`).
struct BatteryInfo {
    present: bool,
    percent: String,
    status: String,
    source: String,
    time_remaining: Option<String>,
    cycle_count: Option<String>,
    health_percent: Option<String>,
    condition: String,
}

/// A connected display (read once at launch via `system_profiler`).
struct DisplayInfo {
    name: String,
    resolution: String,
    detail: Option<String>,
    is_main: bool,
}

/// An audio device and whether it's the system default.
struct AudioDevice {
    name: String,
    is_default: bool,
}

/// Audio output/input devices (read once after launch via `system_profiler`).
#[derive(Default)]
struct AudioInfo {
    outputs: Vec<AudioDevice>,
    inputs: Vec<AudioDevice>,
}

/// Boot-volume storage usage (read once after launch via `df`).
#[derive(Default)]
struct StorageInfo {
    volume: String,
    total: u64,
    used: u64,
    avail: u64,
}

/// Primary network connection (read once after launch).
#[derive(Default)]
struct NetworkInfo {
    service: String,
    interface: String,
    connected: bool,
    ip: Option<String>,
    router: Option<String>,
    dns: Option<String>,
    mac: Option<String>,
}

struct Settings {
    system_data_loading: bool,
    account: SharedString,
    sysinfo: SysInfo,
    battery: Option<BatteryInfo>,
    displays: Vec<DisplayInfo>,
    gpu: String,
    network: NetworkInfo,
    storage: StorageInfo,
    audio: AudioInfo,
    sections: Vec<Vec<Category>>,
    selected: (usize, usize),
    nav: Vec<SubPage>,
    search: Entity<gpui_component::input::InputState>,
    focus: FocusHandle,
    focused_once: bool,
    dragging: bool,
    persistence_error: Option<SharedString>,

    // Wi-Fi
    wifi_on: bool,
    ask_to_join: bool,
    joined: Option<usize>,
    /// Real current Wi-Fi network name (from `networksetup`), or `None` if not
    /// associated (e.g. on Ethernet).
    wifi_current: Option<String>,

    // Bluetooth
    bluetooth_on: bool,
    bt_discoverable: bool,
    /// Real paired/known Bluetooth devices (from `system_profiler`).
    bt_devices: Vec<BtDevice>,

    // Appearance
    appearance: Appearance,
    accent_idx: usize,
    show_color_in_menu: bool,
    large_sidebar: bool,

    // Sound
    output_volume: Entity<SliderState>,
    alert_volume: Entity<SliderState>,
    balance: Entity<SliderState>,
    mute: bool,
    play_on_startup: bool,
    play_ui_sounds: bool,
    alert_idx: usize,

    // General
    handoff: bool,
    airdrop_idx: usize,
    airplay_receiver: bool,
}

/// Read-only system data that is slow enough to keep off the first-frame path.
struct SystemSnapshot {
    account: String,
    sysinfo: SysInfo,
    battery: Option<BatteryInfo>,
    displays: Vec<DisplayInfo>,
    gpu: String,
    network: NetworkInfo,
    storage: StorageInfo,
    audio: AudioInfo,
    wifi_current: Option<String>,
    bt_devices: Vec<BtDevice>,
}

const ACCENTS: &[(&str, u32)] = &[
    ("Blue", 0x0a84ff),
    ("Purple", 0xaf52de),
    ("Pink", 0xff2d55),
    ("Red", 0xff3b30),
    ("Orange", 0xff9500),
    ("Yellow", 0xffcc00),
    ("Green", 0x34c759),
    ("Graphite", 0x8e8e93),
];

const ALERT_SOUNDS: &[&str] = &[
    "Boop",
    "Breeze",
    "Bubble",
    "Crystal",
    "Funk",
    "Heroine",
    "Submarine",
];

// ---- on-disk persistence -------------------------------------------------
//
// The interactive settings state is serialized to a small flat JSON config
// file so toggles/sliders/segmented choices survive a quit. We hand-roll a
// tiny flat-JSON writer/reader (all values are numbers or 0/1 booleans) to
// avoid pulling extra dependencies into the workspace lockfile.

/// A snapshot of all persisted interactive state.
#[derive(Clone, Debug, PartialEq)]
struct Persisted {
    wifi_on: bool,
    ask_to_join: bool,
    joined: Option<usize>,
    bluetooth_on: bool,
    bt_discoverable: bool,
    appearance: u8, // 0 = Light, 1 = Dark, 2 = Auto
    accent_idx: usize,
    show_color_in_menu: bool,
    large_sidebar: bool,
    output_volume: f32,
    alert_volume: f32,
    balance: f32,
    mute: bool,
    play_on_startup: bool,
    play_ui_sounds: bool,
    alert_idx: usize,
    handoff: bool,
    airdrop_idx: usize,
    airplay_receiver: bool,
}

impl Default for Persisted {
    fn default() -> Self {
        Self {
            wifi_on: true,
            ask_to_join: true,
            joined: Some(0),
            bluetooth_on: true,
            bt_discoverable: true,
            // Default appearance follows the live system setting at first launch.
            appearance: if appearance_is_dark() { 1 } else { 0 },
            accent_idx: 0,
            show_color_in_menu: true,
            large_sidebar: false,
            output_volume: 72.0,
            alert_volume: 55.0,
            balance: 50.0,
            mute: false,
            play_on_startup: true,
            play_ui_sounds: true,
            alert_idx: 0,
            handoff: true,
            airdrop_idx: 1,
            airplay_receiver: false,
        }
    }
}

fn config_path() -> Result<PathBuf, storage::Failure> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            Path::new("settings.json"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/rmac-system-settings");
    #[cfg(not(target_os = "macos"))]
    let dir = {
        match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            Some(x) if x.is_absolute() => x.join("rmac-system-settings"),
            _ => home.join(".config/rmac-system-settings"),
        }
    };
    Ok(dir.join("settings.json"))
}

impl Persisted {
    fn load() -> Result<Self, storage::Failure> {
        let path = config_path()?;
        match storage::load_optional(&storage::RealStorage, &path)? {
            Some(content) => Self::parse(&content).map_err(|detail| {
                storage::Failure::message(storage::Operation::LoadSettings, &path, detail)
            }),
            None => Ok(Self::default()),
        }
    }

    fn save(&self) -> Result<(), storage::Failure> {
        let path = config_path()?;
        storage::save(&storage::RealStorage, &path, self.to_json())
    }

    fn to_json(&self) -> String {
        let b = |v: bool| if v { 1 } else { 0 };
        format!(
            concat!(
                "{{\n",
                "  \"wifi_on\": {},\n",
                "  \"ask_to_join\": {},\n",
                "  \"joined\": {},\n",
                "  \"bluetooth_on\": {},\n",
                "  \"bt_discoverable\": {},\n",
                "  \"appearance\": {},\n",
                "  \"accent_idx\": {},\n",
                "  \"show_color_in_menu\": {},\n",
                "  \"large_sidebar\": {},\n",
                "  \"output_volume\": {},\n",
                "  \"alert_volume\": {},\n",
                "  \"balance\": {},\n",
                "  \"mute\": {},\n",
                "  \"play_on_startup\": {},\n",
                "  \"play_ui_sounds\": {},\n",
                "  \"alert_idx\": {},\n",
                "  \"handoff\": {},\n",
                "  \"airdrop_idx\": {},\n",
                "  \"airplay_receiver\": {}\n",
                "}}\n",
            ),
            b(self.wifi_on),
            b(self.ask_to_join),
            self.joined.map(|j| j as i64).unwrap_or(-1),
            b(self.bluetooth_on),
            b(self.bt_discoverable),
            self.appearance,
            self.accent_idx,
            b(self.show_color_in_menu),
            b(self.large_sidebar),
            self.output_volume,
            self.alert_volume,
            self.balance,
            b(self.mute),
            b(self.play_on_startup),
            b(self.play_ui_sounds),
            self.alert_idx,
            b(self.handoff),
            self.airdrop_idx,
            b(self.airplay_receiver),
        )
    }

    /// Parse a flat JSON object of numeric values. Unknown or missing keys keep
    /// their defaults; malformed recognized values reject the existing file.
    fn parse(content: &str) -> Result<Self, String> {
        let mut p = Self::default();
        let trimmed = content.trim();
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return Err("settings file is not a complete JSON object".into());
        }
        let body = &trimmed[1..trimmed.len() - 1];
        for part in body.split(',') {
            let part = part
                .trim()
                .trim_matches(|c: char| c.is_whitespace() || c == '\n');
            if part.is_empty() {
                continue;
            }
            let (raw_key, raw) = part
                .split_once(':')
                .ok_or_else(|| "settings entry is missing ':'".to_string())?;
            let key = raw_key.trim().trim_matches('"');
            let recognized = matches!(
                key,
                "wifi_on"
                    | "ask_to_join"
                    | "joined"
                    | "bluetooth_on"
                    | "bt_discoverable"
                    | "appearance"
                    | "accent_idx"
                    | "show_color_in_menu"
                    | "large_sidebar"
                    | "output_volume"
                    | "alert_volume"
                    | "balance"
                    | "mute"
                    | "play_on_startup"
                    | "play_ui_sounds"
                    | "alert_idx"
                    | "handoff"
                    | "airdrop_idx"
                    | "airplay_receiver"
            );
            if !recognized {
                continue;
            }
            let raw = raw.trim().trim_matches('"');
            let num: f64 = raw
                .parse()
                .map_err(|_| format!("setting '{key}' is not numeric"))?;
            if !num.is_finite() {
                return Err(format!("setting '{key}' is not finite"));
            }
            let truthy = num != 0.0;
            match key {
                "wifi_on" => p.wifi_on = truthy,
                "ask_to_join" => p.ask_to_join = truthy,
                "joined" => p.joined = if num < 0.0 { None } else { Some(num as usize) },
                "bluetooth_on" => p.bluetooth_on = truthy,
                "bt_discoverable" => p.bt_discoverable = truthy,
                "appearance" => p.appearance = (num as u8).min(2),
                "accent_idx" => p.accent_idx = num as usize,
                "show_color_in_menu" => p.show_color_in_menu = truthy,
                "large_sidebar" => p.large_sidebar = truthy,
                "output_volume" => p.output_volume = num as f32,
                "alert_volume" => p.alert_volume = num as f32,
                "balance" => p.balance = num as f32,
                "mute" => p.mute = truthy,
                "play_on_startup" => p.play_on_startup = truthy,
                "play_ui_sounds" => p.play_ui_sounds = truthy,
                "alert_idx" => p.alert_idx = num as usize,
                "handoff" => p.handoff = truthy,
                "airdrop_idx" => p.airdrop_idx = num as usize,
                "airplay_receiver" => p.airplay_receiver = truthy,
                _ => {}
            }
        }
        // Clamp index-like fields so a corrupt file can't panic on lookup.
        if p.accent_idx >= ACCENTS.len() {
            p.accent_idx = 0;
        }
        if p.alert_idx >= ALERT_SOUNDS.len() {
            p.alert_idx = 0;
        }
        if p.airdrop_idx > 2 {
            p.airdrop_idx = 1;
        }
        p.output_volume = p.output_volume.clamp(0.0, 100.0);
        p.alert_volume = p.alert_volume.clamp(0.0, 100.0);
        p.balance = p.balance.clamp(0.0, 100.0);
        Ok(p)
    }

    fn appearance_enum(&self) -> Appearance {
        match self.appearance {
            1 => Appearance::Dark,
            2 => Appearance::Auto,
            _ => Appearance::Light,
        }
    }
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| gpui_component::input::InputState::new(window, cx).placeholder("Search"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        // A missing config is a normal first launch. Existing-but-unreadable or
        // malformed state falls back safely and remains visible to the user.
        let (saved, persistence_error) = match Persisted::load() {
            Ok(saved) => (saved, None),
            Err(failure) => (Persisted::default(), Some(failure.to_string().into())),
        };

        // Sliders persist their value; observing them writes the config on change.
        let mk_slider = |cx: &mut Context<Self>, val: f32| {
            let s = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(100.0)
                    .step(1.0)
                    .default_value(val)
            });
            cx.observe(&s, |this, _, cx| {
                this.persist(cx);
                cx.notify();
            })
            .detach();
            s
        };
        let output_volume = mk_slider(cx, saved.output_volume);
        let alert_volume = mk_slider(cx, saved.alert_volume);
        let balance = mk_slider(cx, saved.balance);

        // Hardware discovery launches multiple platform commands, including
        // system_profiler. Keep it off the first-frame path and redraw once the
        // complete read-only snapshot is available.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async { gather_system_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.apply_system_snapshot(snapshot);
                cx.notify();
            });
        })
        .detach();

        Self {
            system_data_loading: true,
            account: std::env::var("USER")
                .unwrap_or_else(|_| "User".into())
                .into(),
            sysinfo: SysInfo::default(),
            battery: None,
            displays: Vec::new(),
            gpu: String::new(),
            network: NetworkInfo::default(),
            storage: StorageInfo::default(),
            audio: AudioInfo::default(),
            sections: categories(),
            selected: (1, 0), // General
            nav: Vec::new(),
            search,
            focus: cx.focus_handle(),
            focused_once: false,
            dragging: false,
            persistence_error,

            wifi_on: saved.wifi_on,
            ask_to_join: saved.ask_to_join,
            joined: saved.joined,
            wifi_current: None,

            bluetooth_on: saved.bluetooth_on,
            bt_discoverable: saved.bt_discoverable,
            bt_devices: Vec::new(),

            appearance: saved.appearance_enum(),
            accent_idx: saved.accent_idx,
            show_color_in_menu: saved.show_color_in_menu,
            large_sidebar: saved.large_sidebar,

            output_volume,
            alert_volume,
            balance,
            mute: saved.mute,
            play_on_startup: saved.play_on_startup,
            play_ui_sounds: saved.play_ui_sounds,
            alert_idx: saved.alert_idx,

            handoff: saved.handoff,
            airdrop_idx: saved.airdrop_idx,
            airplay_receiver: saved.airplay_receiver,
        }
    }

    fn apply_system_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.account = snapshot.account.into();
        self.sysinfo = snapshot.sysinfo;
        self.battery = snapshot.battery;
        self.displays = snapshot.displays;
        self.gpu = snapshot.gpu;
        self.network = snapshot.network;
        self.storage = snapshot.storage;
        self.audio = snapshot.audio;
        self.wifi_current = snapshot.wifi_current;
        self.bt_devices = snapshot.bt_devices;
        self.system_data_loading = false;
    }

    /// Capture the current interactive state and write it to disk.
    fn persist(&mut self, cx: &App) {
        let snapshot = Persisted {
            wifi_on: self.wifi_on,
            ask_to_join: self.ask_to_join,
            joined: self.joined,
            bluetooth_on: self.bluetooth_on,
            bt_discoverable: self.bt_discoverable,
            appearance: match self.appearance {
                Appearance::Light => 0,
                Appearance::Dark => 1,
                Appearance::Auto => 2,
            },
            accent_idx: self.accent_idx,
            show_color_in_menu: self.show_color_in_menu,
            large_sidebar: self.large_sidebar,
            output_volume: self.output_volume.read(cx).value().start(),
            alert_volume: self.alert_volume.read(cx).value().start(),
            balance: self.balance.read(cx).value().start(),
            mute: self.mute,
            play_on_startup: self.play_on_startup,
            play_ui_sounds: self.play_ui_sounds,
            alert_idx: self.alert_idx,
            handoff: self.handoff,
            airdrop_idx: self.airdrop_idx,
            airplay_receiver: self.airplay_receiver,
        };
        self.persistence_error = snapshot
            .save()
            .err()
            .map(|failure| failure.to_string().into());
    }

    fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        self.nav.pop();
        cx.notify();
    }

    fn push(&mut self, sub: SubPage, cx: &mut Context<Self>) {
        self.nav.push(sub);
        cx.notify();
    }

    // ---- chrome -------------------------------------------------------

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_back = !self.nav.is_empty();
        let back = div()
            .id("nav-back")
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .when(can_back, |el: Stateful<Div>| {
                el.hover(|h| h.bg(hsl(0x00000010)))
                    .cursor_pointer()
                    .on_click(cx.listener(|t, _, _, cx| t.go_back(cx)))
            })
            .child(glyph(
                "icons/chevron-left.svg",
                17.0,
                if can_back { accent() } else { hsl(0xc4c4c8) },
            ));

        div()
            .id("topbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = false),
            )
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .w(px(SIDEBAR_W))
                    .h_full()
                    .bg(sidebar_bg())
                    .flex()
                    .items_center()
                    .pl(px(13.0))
                    .child(rmac_ui::traffic_lights()),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(pane_bg())
                    .flex()
                    .items_center()
                    .pl_3()
                    .gap_1()
                    .child(back)
                    .child(glyph("icons/chevron-right.svg", 17.0, hsl(0xc4c4c8))),
            )
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let search = div()
            .mx_2()
            .mt_1()
            .mb_2()
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(hsl(0xdcdce0))
            .child(glyph("icons/search.svg", 13.0, secondary()))
            .child(
                div()
                    .flex_1()
                    .child(gpui_component::input::Input::new(&self.search).appearance(false)),
            );
        let q = self.search.read(cx).value().to_lowercase();

        let account = div()
            .flex()
            .items_center()
            .gap_2p5()
            .mx_2()
            .mb_2()
            .px_2()
            .py_1p5()
            .rounded(px(8.0))
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(hsl(0xc7c7cc))
                    .child(glyph("icons/user.svg", 22.0, white())),
            )
            .child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(self.account.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(secondary())
                            .child("Apple Account"),
                    ),
            );

        let mut col = div()
            .id("sidebar-scroll")
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_1()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep())
            .overflow_y_scroll()
            .child(search)
            .child(account);

        let mut first_section = true;
        for (si, section) in self.sections.iter().enumerate() {
            let matching: Vec<(usize, &Category)> = section
                .iter()
                .enumerate()
                .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q))
                .collect();
            if matching.is_empty() {
                continue;
            }
            if !first_section {
                col = col.child(div().h(px(14.0)));
            }
            first_section = false;
            for (ci, cat) in matching {
                let selected = self.selected == (si, ci);
                col = col.child(
                    div()
                        .id(SharedString::from(format!("cat-{si}-{ci}")))
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .h(px(30.0))
                        .mx_2()
                        .px_2()
                        .rounded(px(6.0))
                        .when(selected, |el: Stateful<Div>| el.bg(accent()))
                        .when(!selected, |el: Stateful<Div>| {
                            el.hover(|h| h.bg(hsl(0x00000008)))
                        })
                        .child(tile(cat.icon, cat.color, 20.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(if selected { white() } else { label() })
                                .child(cat.name.clone()),
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.selected = (si, ci);
                            t.nav.clear();
                            cx.notify();
                        })),
                );
            }
        }
        col
    }

    // ---- detail dispatch ---------------------------------------------

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content: Div = if let Some(sub) = self.nav.last().cloned() {
            self.render_subpage(&sub, cx)
        } else {
            match self.current().name.as_ref() {
                "Wi-Fi" => self.render_wifi(cx),
                "Bluetooth" => self.render_bluetooth(cx),
                "General" => self.render_general(cx),
                "Appearance" => self.render_appearance(cx),
                "Sound" => self.render_sound(cx),
                "Battery" => self.render_battery(),
                "Displays" => self.render_displays(),
                "Network" => self.render_network(),
                _ => self.render_generic(cx),
            }
        };

        div()
            .id("detail-scroll")
            .flex_1()
            .h_full()
            .bg(pane_bg())
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(560.0))
                    .mx_auto()
                    .px_5()
                    .pb_8()
                    .when(self.system_data_loading, |el| {
                        el.child(note_card("Loading system information…"))
                    })
                    .child(content),
            )
    }

    fn render_hero(&self) -> Div {
        let cat = self.current();
        div()
            .v_flex()
            .items_center()
            .gap_2()
            .pt_6()
            .pb_5()
            .child(tile(cat.icon, cat.color, 64.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(cat.name.clone()),
            )
            .child(
                div()
                    .max_w(px(440.0))
                    .text_center()
                    .text_size(px(13.0))
                    .text_color(secondary())
                    .child(cat.desc.clone()),
            )
    }

    fn pane(&self, cards: Vec<Div>) -> Div {
        div().v_flex().child(self.render_hero()).children(cards)
    }

    // ---- generic (read-only mockup) panes ----------------------------

    fn render_generic(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let cat = self.current();
        let cards: Vec<Div> = cat
            .cards
            .iter()
            .map(|rows| {
                let rows: Vec<AnyElement> = rows
                    .iter()
                    .map(|r| {
                        nav_row(
                            view.clone(),
                            r.icon,
                            r.color,
                            r.label.clone(),
                            None,
                            SubPage::Placeholder {
                                icon: r.icon,
                                color: r.color,
                                title: r.label.clone(),
                            },
                        )
                    })
                    .collect();
                card(rows)
            })
            .collect();
        self.pane(cards)
    }

    // ---- Wi-Fi --------------------------------------------------------

    fn render_wifi(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let on = self.wifi_on;

        let toggle = card(vec![switch_row(
            "icons/wifi.svg",
            accent(),
            "Wi-Fi".into(),
            None,
            self.wifi_on,
            cx,
            |s, v| s.wifi_on = v,
        )]);

        let _ = view;
        let mut cards = vec![
            note_card(
                "The Wi-Fi toggle is local to this app and joining isn't supported. \
                 The current network below is read live from the system.",
            ),
            toggle,
        ];

        if on {
            // Real current network, read from `networksetup`.
            let status_row = match &self.wifi_current {
                Some(ssid) => value_row(
                    "icons/wifi.svg",
                    accent(),
                    ssid.clone().into(),
                    "Connected".into(),
                ),
                None => value_row(
                    "icons/wifi.svg",
                    secondary(),
                    "Not connected".into(),
                    "No Wi-Fi network".into(),
                ),
            };
            cards.push(section_header("Network"));
            cards.push(card(vec![status_row]));

            cards.push(card(vec![switch_row(
                "icons/wifi.svg",
                secondary(),
                "Ask to join networks".into(),
                Some("Known networks are joined automatically.".into()),
                self.ask_to_join,
                cx,
                |s, v| s.ask_to_join = v,
            )]));
        }

        self.pane(cards)
    }

    // ---- Bluetooth ----------------------------------------------------

    fn render_bluetooth(&self, cx: &Context<Self>) -> Div {
        let on = self.bluetooth_on;
        let toggle = card(vec![switch_row(
            "icons/bluetooth.svg",
            accent(),
            "Bluetooth".into(),
            None,
            self.bluetooth_on,
            cx,
            |s, v| s.bluetooth_on = v,
        )]);
        let mut cards = vec![
            note_card(
                "The Bluetooth on/off and discoverable toggles are local to this app \
                 (they don't change your Mac's real Bluetooth state). The device list \
                 below is read live from the system.",
            ),
            toggle,
        ];
        if on {
            cards.push(card(vec![switch_row(
                "icons/bluetooth.svg",
                secondary(),
                "Discoverable".into(),
                Some("This Mac can be discovered by nearby devices.".into()),
                self.bt_discoverable,
                cx,
                |s, v| s.bt_discoverable = v,
            )]));

            let connected: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|d| d.connected)
                .map(|d| {
                    value_row(
                        "icons/bluetooth.svg",
                        accent(),
                        d.name.clone().into(),
                        if d.kind.is_empty() {
                            "Connected".into()
                        } else {
                            d.kind.clone().into()
                        },
                    )
                })
                .collect();
            if !connected.is_empty() {
                cards.push(section_header("Connected"));
                cards.push(card(connected));
            }

            let others: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|d| !d.connected)
                .map(|d| {
                    value_row(
                        "icons/bluetooth.svg",
                        secondary(),
                        d.name.clone().into(),
                        if d.kind.is_empty() {
                            "Not Connected".into()
                        } else {
                            d.kind.clone().into()
                        },
                    )
                })
                .collect();
            if !others.is_empty() {
                cards.push(section_header("Devices"));
                cards.push(card(others));
            }
            if self.bt_devices.is_empty() {
                cards.push(note_card("No paired Bluetooth devices found."));
            }
        }
        self.pane(cards)
    }

    // ---- General ------------------------------------------------------

    fn render_general(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let cards = vec![
            card(vec![
                nav_row(
                    view.clone(),
                    "icons/info.svg",
                    hsl(0x8e8e93),
                    "About".into(),
                    Some(self.sysinfo.model.clone().into()),
                    SubPage::About,
                ),
                nav_row(
                    view.clone(),
                    "icons/refresh-cw.svg",
                    hsl(0x8e8e93),
                    "Software Update".into(),
                    Some(self.sysinfo.os.clone().into()),
                    SubPage::SoftwareUpdate,
                ),
                nav_row(
                    view.clone(),
                    "icons/database.svg",
                    hsl(0x8e8e93),
                    "Storage".into(),
                    None,
                    SubPage::Storage,
                ),
            ]),
            card(vec![switch_row(
                "icons/folder-symlink.svg",
                accent(),
                "Allow Handoff between this Mac and your devices".into(),
                None,
                self.handoff,
                cx,
                |s, v| s.handoff = v,
            )]),
            {
                let mut c = div()
                    .v_flex()
                    .mb_3()
                    .rounded(px(10.0))
                    .bg(card_bg())
                    .border_1()
                    .border_color(sep());
                c = c.child(label_row("AirDrop", None));
                c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
                c = c.child(
                    segmented(
                        view.clone(),
                        "airdrop-seg",
                        &["No One", "Contacts Only", "Everyone"],
                        self.airdrop_idx,
                        |s, i| s.airdrop_idx = i,
                    )
                    .p_3(),
                );
                c
            },
            card(vec![switch_row(
                "icons/app-window.svg",
                accent(),
                "AirPlay Receiver".into(),
                Some("Allow this Mac to receive AirPlay content.".into()),
                self.airplay_receiver,
                cx,
                |s, v| s.airplay_receiver = v,
            )]),
        ];
        self.pane(cards)
    }

    // ---- Appearance ---------------------------------------------------

    fn render_appearance(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();

        let appearance_card = {
            let opt = |id: &'static str, name: &'static str, ap: Appearance, swatch: Hsla| {
                let selected = self.appearance == ap;
                let v = view.clone();
                div()
                    .id(ElementId::from(id))
                    .v_flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(64.0))
                            .h(px(40.0))
                            .rounded(px(6.0))
                            .bg(swatch)
                            .border_2()
                            .border_color(if selected { accent() } else { sep() }),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if selected { accent() } else { label() })
                            .child(name),
                    )
                    .on_click(move |_, _, cx| {
                        v.update(cx, |s, cx| {
                            s.appearance = ap;
                            s.persist(cx);
                            cx.notify();
                        });
                    })
            };
            div()
                .flex()
                .gap_5()
                .justify_center()
                .p_4()
                .rounded(px(10.0))
                .mb_3()
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(opt("ap-light", "Light", Appearance::Light, hsl(0xf5f5f7)))
                .child(opt("ap-dark", "Dark", Appearance::Dark, hsl(0x2c2c2e)))
                .child(opt("ap-auto", "Auto", Appearance::Auto, hsl(0x8e8e93)))
        };

        // Accent color swatches
        let accent_card = {
            let swatches: Vec<AnyElement> = ACCENTS
                .iter()
                .enumerate()
                .map(|(i, (_name, hex))| {
                    let selected = self.accent_idx == i;
                    let v = view.clone();
                    div()
                        .id(ElementId::from(SharedString::from(format!("accent-{i}"))))
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        .bg(hsl(*hex))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| {
                            el.border_2().border_color(white()).shadow_sm()
                        })
                        .when(selected, |el| {
                            el.child(glyph("icons/check.svg", 12.0, white()))
                        })
                        .on_click(move |_, _, cx| {
                            v.update(cx, |s, cx| {
                                s.accent_idx = i;
                                s.persist(cx);
                                cx.notify();
                            });
                        })
                        .into_any_element()
                })
                .collect();
            div()
                .v_flex()
                .mb_3()
                .rounded(px(10.0))
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(label_row("Accent color", None))
                .child(div().h(px(1.0)).bg(sep()).mx_3())
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .flex_wrap()
                        .p_3()
                        .children(swatches),
                )
        };

        let toggles = card(vec![
            switch_row(
                "icons/palette.svg",
                accent(),
                "Show color in menu bar".into(),
                None,
                self.show_color_in_menu,
                cx,
                |s, v| s.show_color_in_menu = v,
            ),
            switch_row(
                "icons/panel-top.svg",
                accent(),
                "Larger sidebar icons".into(),
                None,
                self.large_sidebar,
                cx,
                |s, v| s.large_sidebar = v,
            ),
        ]);

        let system_note = card(vec![value_row(
            "icons/info.svg",
            secondary(),
            "Current system appearance".into(),
            if appearance_is_dark() {
                "Dark".into()
            } else {
                "Light".into()
            },
        )]);

        self.pane(vec![appearance_card, accent_card, toggles, system_note])
    }

    // ---- Sound --------------------------------------------------------

    fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();

        let out = self.output_volume.read(cx).value().start().round() as i32;
        let alert = self.alert_volume.read(cx).value().start().round() as i32;
        let bal = self.balance.read(cx).value().start().round() as i32;

        let output_card = div()
            .v_flex()
            .mb_3()
            .rounded(px(10.0))
            .bg(card_bg())
            .border_1()
            .border_color(sep())
            .child(slider_row(
                "Output volume",
                &self.output_volume,
                format!("{out}%").into(),
            ))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(slider_row(
                "Balance",
                &self.balance,
                format!("{bal}").into(),
            ))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(switch_row(
                "icons/volume-2.svg",
                secondary(),
                "Mute".into(),
                None,
                self.mute,
                cx,
                |s, v| s.mute = v,
            ));

        let alert_card = div()
            .v_flex()
            .mb_3()
            .rounded(px(10.0))
            .bg(card_bg())
            .border_1()
            .border_color(sep())
            .child(label_row(
                "Alert sound",
                Some(ALERT_SOUNDS[self.alert_idx].into()),
            ))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(
                segmented_dynamic(
                    view.clone(),
                    "alert-seg",
                    ALERT_SOUNDS,
                    self.alert_idx,
                    |s, i| s.alert_idx = i,
                )
                .p_3(),
            )
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(slider_row(
                "Alert volume",
                &self.alert_volume,
                format!("{alert}%").into(),
            ));

        let toggles = card(vec![
            switch_row(
                "icons/power.svg",
                secondary(),
                "Play sound on startup".into(),
                None,
                self.play_on_startup,
                cx,
                |s, v| s.play_on_startup = v,
            ),
            switch_row(
                "icons/bell.svg",
                secondary(),
                "Play user interface sound effects".into(),
                None,
                self.play_ui_sounds,
                cx,
                |s, v| s.play_ui_sounds = v,
            ),
        ]);

        let output_devices = self.audio_device_card("Output Device", &self.audio.outputs);
        let input_devices = self.audio_device_card("Input Device", &self.audio.inputs);

        self.pane(vec![
            output_card,
            alert_card,
            toggles,
            output_devices,
            input_devices,
        ])
    }

    /// A card listing real audio devices, with the system default checked.
    fn audio_device_card(&self, title: &'static str, devices: &[AudioDevice]) -> Div {
        if devices.is_empty() {
            return div();
        }
        let blue = hsl(0x0a84ff);
        let rows: Vec<AnyElement> = devices
            .iter()
            .map(|d| {
                row_base()
                    .child(tile("icons/volume-2.svg", blue, 22.0))
                    .child(text_block(d.name.clone().into(), None))
                    .when(d.is_default, |el| {
                        el.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(secondary())
                                .child("Default"),
                        )
                        .child(glyph("icons/check.svg", 13.0, blue))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .v_flex()
            .child(section_header(title))
            .child(card(rows))
    }

    // ---- Battery (real read-only) -------------------------------------

    fn render_battery(&self) -> Div {
        let green = hsl(0x34c759);
        let gray = hsl(0x8e8e93);
        match &self.battery {
            Some(b) if b.present => {
                let mut status_rows = vec![
                    value_row(
                        "icons/battery-charging.svg",
                        green,
                        "Charge".into(),
                        b.percent.clone().into(),
                    ),
                    value_row(
                        "icons/info.svg",
                        gray,
                        "Status".into(),
                        b.status.clone().into(),
                    ),
                    value_row(
                        "icons/power.svg",
                        gray,
                        "Power Source".into(),
                        b.source.clone().into(),
                    ),
                ];
                if let Some(t) = &b.time_remaining {
                    status_rows.push(value_row(
                        "icons/clock.svg",
                        gray,
                        "Time Remaining".into(),
                        t.clone().into(),
                    ));
                }

                let mut health_rows = vec![value_row(
                    "icons/heart-handshake.svg",
                    green,
                    "Condition".into(),
                    b.condition.clone().into(),
                )];
                if let Some(h) = &b.health_percent {
                    health_rows.insert(
                        0,
                        value_row(
                            "icons/battery-charging.svg",
                            green,
                            "Maximum Capacity".into(),
                            h.clone().into(),
                        ),
                    );
                }
                if let Some(c) = &b.cycle_count {
                    health_rows.push(value_row(
                        "icons/history.svg",
                        gray,
                        "Cycle Count".into(),
                        c.clone().into(),
                    ));
                }

                self.pane(vec![
                    card(status_rows),
                    section_header("Battery Health"),
                    card(health_rows),
                ])
            }
            _ => self.pane(vec![note_card(
                "No battery detected — this Mac runs on continuous power.",
            )]),
        }
    }

    // ---- Displays (real read-only) ------------------------------------

    fn render_displays(&self) -> Div {
        let blue = hsl(0x0a84ff);
        let gray = hsl(0x8e8e93);
        let mut cards: Vec<Div> = Vec::new();

        if self.displays.is_empty() {
            cards.push(note_card("No displays were detected."));
        }
        for d in &self.displays {
            // A dynamic section header (display name) — the static helper only
            // takes &'static str, so build it inline.
            let mut title: SharedString = d.name.clone().into();
            if d.is_main {
                title = format!("{} (Main)", d.name).into();
            }
            cards.push(
                div()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(title),
            );
            let mut rows = vec![value_row(
                "icons/monitor.svg",
                blue,
                "Resolution".into(),
                d.resolution.clone().into(),
            )];
            if let Some(det) = &d.detail {
                rows.insert(
                    0,
                    value_row("icons/info.svg", gray, "Type".into(), det.clone().into()),
                );
            }
            cards.push(card(rows));
        }

        if !self.gpu.is_empty() {
            cards.push(section_header("Graphics"));
            cards.push(card(vec![value_row(
                "icons/settings.svg",
                gray,
                "Chipset".into(),
                self.gpu.clone().into(),
            )]));
        }

        self.pane(cards)
    }

    // ---- Network (real read-only) -------------------------------------

    fn render_network(&self) -> Div {
        let blue = hsl(0x0a84ff);
        let gray = hsl(0x8e8e93);
        let green = hsl(0x34c759);
        let n = &self.network;

        let status_color = if n.connected { green } else { gray };
        let mut conn_rows = vec![value_row(
            "icons/globe.svg",
            status_color,
            "Status".into(),
            if n.connected {
                "Connected".into()
            } else {
                "Not Connected".into()
            },
        )];
        conn_rows.push(value_row(
            "icons/wifi.svg",
            blue,
            "Service".into(),
            format!("{} ({})", n.service, n.interface).into(),
        ));

        let mut detail_rows: Vec<AnyElement> = Vec::new();
        if let Some(ip) = &n.ip {
            detail_rows.push(value_row(
                "icons/globe.svg",
                gray,
                "IP Address".into(),
                ip.clone().into(),
            ));
        }
        if let Some(r) = &n.router {
            detail_rows.push(value_row(
                "icons/folder-symlink.svg",
                gray,
                "Router".into(),
                r.clone().into(),
            ));
        }
        if let Some(d) = &n.dns {
            detail_rows.push(value_row(
                "icons/info.svg",
                gray,
                "DNS Server".into(),
                d.clone().into(),
            ));
        }
        if let Some(m) = &n.mac {
            detail_rows.push(value_row(
                "icons/key.svg",
                gray,
                "Hardware Address".into(),
                m.clone().into(),
            ));
        }

        let mut cards = vec![card(conn_rows)];
        if !detail_rows.is_empty() {
            cards.push(section_header("TCP/IP"));
            cards.push(card(detail_rows));
        }
        self.pane(cards)
    }

    /// Real boot-volume storage usage with a macOS-style fill bar.
    fn storage_body(&self) -> Div {
        let s = &self.storage;
        let frac = if s.total > 0 {
            (s.used as f32 / s.total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let bar_card = div()
            .v_flex()
            .gap_2()
            .mb_3()
            .p_4()
            .rounded(px(10.0))
            .bg(card_bg())
            .border_1()
            .border_color(sep())
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .items_baseline()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(s.volume.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(secondary())
                            .child(format!(
                                "{} available of {}",
                                fmt_gb(s.avail),
                                fmt_gb(s.total)
                            )),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(10.0))
                    .rounded(px(5.0))
                    .bg(hsl(0xe5e5ea))
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(frac))
                            .rounded(px(5.0))
                            .bg(accent()),
                    ),
            );

        let rows = card(vec![
            value_row(
                "icons/database.svg",
                accent(),
                "Capacity".into(),
                fmt_gb(s.total).into(),
            ),
            value_row(
                "icons/database.svg",
                hsl(0xff9500),
                "Used".into(),
                fmt_gb(s.used).into(),
            ),
            value_row(
                "icons/database.svg",
                hsl(0x34c759),
                "Available".into(),
                fmt_gb(s.avail).into(),
            ),
        ]);

        div().v_flex().child(bar_card).child(rows)
    }

    // ---- subpages -----------------------------------------------------

    fn render_subpage(&self, sub: &SubPage, _cx: &Context<Self>) -> Div {
        let (title, body): (SharedString, Div) = match sub {
            SubPage::About => ("About".into(), self.about_body()),
            SubPage::SoftwareUpdate => (
                "Software Update".into(),
                div()
                    .v_flex()
                    .child(card(vec![value_row(
                        "icons/refresh-cw.svg",
                        secondary(),
                        "Current version".into(),
                        self.sysinfo.os.clone().into(),
                    )]))
                    .child(note_card(
                        "rmac reads the installed macOS version but does not check Apple's \
                         update servers, so it can't report available updates.",
                    )),
            ),
            SubPage::Storage => ("Storage".into(), self.storage_body()),
            SubPage::Placeholder { icon, color, title } => (
                title.clone(),
                div()
                    .v_flex()
                    .child(card(vec![value_row(
                        icon,
                        *color,
                        title.clone(),
                        "Not implemented".into(),
                    )]))
                    .child(note_card(
                        "This pane isn't built in rmac yet — it doesn't read or change \
                         any real system setting.",
                    )),
            ),
        };

        let header = div()
            .v_flex()
            .items_center()
            .gap_1()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(secondary())
                    .child("‹ Back, or press ⌘["),
            );

        div().v_flex().child(header).child(body)
    }

    fn about_body(&self) -> Div {
        let si = &self.sysinfo;
        card(vec![
            value_row(
                "icons/info.svg",
                secondary(),
                "Name".into(),
                si.computer_name.clone().into(),
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Model".into(),
                si.model.clone().into(),
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "Chip".into(),
                si.chip.clone().into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Memory".into(),
                si.memory.clone().into(),
            ),
            value_row(
                "icons/refresh-cw.svg",
                secondary(),
                "macOS".into(),
                si.os.clone().into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Serial Number".into(),
                si.serial.clone().into(),
            ),
        ])
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus);
        }
        let persistence_error = self.persistence_error.clone();
        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .on_action(cx.listener(|t, _: &GoBack, _, cx| t.go_back(cx)))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .bg(pane_bg())
            .text_color(label())
            .child(self.render_topbar(cx))
            .when_some(persistence_error, |settings, message| {
                settings.child(
                    div()
                        .id("persistence-error")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(gpui::rgba(0xff3b301f))
                        .border_b_1()
                        .border_color(gpui::rgba(0xff3b3059))
                        .text_size(px(12.0))
                        .text_color(gpui::rgb(0xc62828))
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.persistence_error = None;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_detail(cx)),
            )
    }
}

const SIDEBAR_W: f32 = 248.0;

// ---- row / control builders ----------------------------------------------

fn row_base() -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(44.0))
        .px_3()
        .py_2()
}

fn text_block(title: SharedString, sub: Option<SharedString>) -> Div {
    let mut b = div()
        .v_flex()
        .flex_1()
        .child(div().text_size(px(13.0)).text_color(label()).child(title));
    if let Some(s) = sub {
        b = b.child(div().text_size(px(11.0)).text_color(secondary()).child(s));
    }
    b
}

/// A plain card-section label row (no control).
fn label_row(title: &'static str, value: Option<SharedString>) -> Div {
    let mut r = row_base().child(
        div()
            .flex_1()
            .text_size(px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(v) = value {
        r = r.child(div().text_size(px(13.0)).text_color(secondary()).child(v));
    }
    r
}

/// A read-only row with a right-aligned value.
fn value_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: SharedString,
) -> AnyElement {
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

/// An informational note card, e.g. to flag a pane as simulated/demo state
/// rather than a reflection of (or control over) real system hardware.
fn note_card(text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mb_3()
        .px_3()
        .py_2p5()
        .rounded(px(10.0))
        .bg(hsl(0xfff6da))
        .border_1()
        .border_color(hsl(0xeedca0))
        .child(glyph("icons/info.svg", 15.0, hsl(0xb8860b)))
        .child(
            div()
                .flex_1()
                .text_size(px(11.5))
                .text_color(hsl(0x7a5c00))
                .child(text),
        )
}

/// A section header above a card (gray small caps-ish title).
fn section_header(title: &'static str) -> Div {
    div()
        .px_1()
        .pt_2()
        .pb_1()
        .text_size(px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .text_color(secondary())
        .child(title)
}

/// A switch row whose state lives in the view; `set` writes the new bool.
#[allow(clippy::too_many_arguments)]
fn switch_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    sub: Option<SharedString>,
    checked: bool,
    cx: &Context<Settings>,
    set: fn(&mut Settings, bool),
) -> AnyElement {
    let view = cx.entity();
    let id = ElementId::from(SharedString::from(format!("sw-{title}")));
    let sw = Switch::new(id).checked(checked).on_click(move |v, _, cx| {
        let nv = *v;
        view.update(cx, |s, cx| {
            set(s, nv);
            s.persist(cx);
            cx.notify();
        });
    });
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, sub))
        .child(sw)
        .into_any_element()
}

/// A slider row (state held in its own SliderState entity).
fn slider_row(title: &'static str, state: &Entity<SliderState>, value: SharedString) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(div().flex_1().child(Slider::new(state).w_full()))
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(px(12.0))
                .text_color(secondary())
                .child(value),
        )
}

/// A clickable navigation row that pushes a subpage onto the back stack.
fn nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
    target: SubPage,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("nav-{title}")));
    let mut r = row_base()
        .id(id)
        .cursor_pointer()
        .hover(|h| h.bg(hsl(0x00000006)))
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None));
    if let Some(v) = value {
        r = r.child(div().text_size(px(13.0)).text_color(secondary()).child(v));
    }
    r.child(glyph("icons/chevron-right.svg", 14.0, hsl(0xc4c4c8)))
        .on_click(move |_, _, cx| {
            let target = target.clone();
            view.update(cx, |s, cx| s.push(target, cx));
        })
        .into_any_element()
}

/// A segmented control over a fixed set of options; `set` writes the index.
fn segmented(
    view: Entity<Settings>,
    id: &'static str,
    options: &[&'static str],
    selected: usize,
    set: fn(&mut Settings, usize),
) -> Div {
    let mut row = div().flex().gap_1().w_full();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let v = view.clone();
        row = row.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{i}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .when(is_sel, |el| el.bg(accent()).text_color(white()))
                .when(!is_sel, |el| {
                    el.bg(hsl(0xe9e9ec))
                        .text_color(label())
                        .hover(|h| h.bg(hsl(0xdedee2)))
                })
                .child(*opt)
                .on_click(move |_, _, cx| {
                    v.update(cx, |s, cx| {
                        set(s, i);
                        s.persist(cx);
                        cx.notify();
                    });
                }),
        );
    }
    row
}

/// Like [`segmented`] but for a runtime slice (e.g. alert sound names).
fn segmented_dynamic(
    view: Entity<Settings>,
    id: &'static str,
    options: &[&'static str],
    selected: usize,
    set: fn(&mut Settings, usize),
) -> Div {
    let mut row = div().flex().flex_wrap().gap_1().w_full();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let v = view.clone();
        row = row.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{i}"))))
                .flex()
                .items_center()
                .justify_center()
                .px_2()
                .h(px(26.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .when(is_sel, |el| el.bg(accent()).text_color(white()))
                .when(!is_sel, |el| {
                    el.bg(hsl(0xe9e9ec))
                        .text_color(label())
                        .hover(|h| h.bg(hsl(0xdedee2)))
                })
                .child(*opt)
                .on_click(move |_, _, cx| {
                    v.update(cx, |s, cx| {
                        set(s, i);
                        s.persist(cx);
                        cx.notify();
                    });
                }),
        );
    }
    row
}

/// Build a rounded white card from rows, inserting inset separators.
fn card(rows: Vec<AnyElement>) -> Div {
    let mut c = div()
        .v_flex()
        .mb_3()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep());
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        c = c.child(r);
        if i + 1 < n {
            c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
        }
    }
    c
}

// ---- real macOS reads (best-effort, read-only) ---------------------------

fn cmd(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn gather_system_snapshot() -> SystemSnapshot {
    let (gpu, displays) = gather_displays();
    SystemSnapshot {
        account: account_name(),
        sysinfo: gather_sysinfo(),
        battery: gather_battery(),
        displays,
        gpu,
        network: gather_network(),
        storage: gather_storage(),
        audio: gather_audio(),
        wifi_current: gather_wifi_current(),
        bt_devices: gather_bluetooth(),
    }
}

fn appearance_is_dark() -> bool {
    cmd("defaults", &["read", "-g", "AppleInterfaceStyle"])
        .map(|s| s.eq_ignore_ascii_case("Dark"))
        .unwrap_or(false)
}

/// Discover the Wi-Fi hardware port's device (e.g. `en0`, `en1`) instead of
/// assuming `en0`.
fn wifi_device() -> Option<String> {
    let out = cmd("networksetup", &["-listallhardwareports"])?;
    // Blocks look like: "Hardware Port: Wi-Fi\nDevice: en0\n...". Find the
    // Device line that follows the Wi-Fi port.
    let mut lines = out.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("Hardware Port:") && line.contains("Wi-Fi") {
            for next in lines.by_ref() {
                if let Some(dev) = next.trim_start().strip_prefix("Device:") {
                    return Some(dev.trim().to_string());
                }
                if next.trim().is_empty() {
                    break;
                }
            }
        }
    }
    None
}

/// The real current Wi-Fi network name, or `None` if not associated.
fn gather_wifi_current() -> Option<String> {
    let dev = wifi_device().unwrap_or_else(|| "en0".to_string());
    let out = cmd("networksetup", &["-getairportnetwork", &dev])?;
    // "Current Wi-Fi Network: <SSID>" when connected, otherwise a "not
    // associated" message we treat as None.
    out.split_once(':')
        .map(|(_, v)| v.trim().to_string())
        .filter(|s| !s.is_empty() && !out.contains("not associated"))
}

/// Real paired/known Bluetooth devices from `system_profiler SPBluetoothDataType`.
/// Connected devices first, capped to a reasonable number for the list.
fn gather_bluetooth() -> Vec<BtDevice> {
    let Some(out) = cmd("system_profiler", &["SPBluetoothDataType"]) else {
        return Vec::new();
    };
    let mut devices: Vec<BtDevice> = Vec::new();
    let mut connected = false;
    let mut cur: Option<BtDevice> = None;
    let flush = |cur: &mut Option<BtDevice>, out: &mut Vec<BtDevice>| {
        if let Some(d) = cur.take() {
            out.push(d);
        }
    };
    for line in out.lines() {
        let t = line.trim();
        if t == "Connected:" {
            flush(&mut cur, &mut devices);
            connected = true;
            continue;
        }
        if t == "Not Connected:" {
            flush(&mut cur, &mut devices);
            connected = false;
            continue;
        }
        // A device-name line ends with ':' and has no "key: value" body. Skip
        // the controller's own header rows ("Bluetooth", "Bluetooth Controller").
        if t.ends_with(':') && !t.contains(": ") {
            flush(&mut cur, &mut devices);
            let name = t.trim_end_matches(':').trim().to_string();
            let is_header = name.is_empty()
                || name == "Bluetooth"
                || name == "Bluetooth Controller"
                || name == "Controller";
            if !is_header {
                cur = Some(BtDevice {
                    name,
                    connected,
                    kind: String::new(),
                });
            }
        } else if let Some(d) = cur.as_mut() {
            if let Some(rest) = t.strip_prefix("Minor Type:") {
                d.kind = rest.trim().to_string();
            }
        }
    }
    flush(&mut cur, &mut devices);
    // Connected first, then by name; trim the list so the pane stays compact.
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
    devices.truncate(12);
    devices
}

fn account_name() -> String {
    cmd("id", &["-F"])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "User".into())
}

fn gather_sysinfo() -> SysInfo {
    let computer_name = cmd("scutil", &["--get", "ComputerName"])
        .or_else(|| cmd("hostname", &[]))
        .unwrap_or_else(|| "Mac".into());

    let os = {
        let name = cmd("sw_vers", &["-productName"]).unwrap_or_else(|| "macOS".into());
        let ver = cmd("sw_vers", &["-productVersion"]).unwrap_or_default();
        format!("{name} {ver}").trim().to_string()
    };

    let chip = cmd("sysctl", &["-n", "machdep.cpu.brand_string"]).unwrap_or_else(|| "—".into());

    let memory = cmd("sysctl", &["-n", "hw.memsize"])
        .and_then(|s| s.parse::<u64>().ok())
        .map(|bytes| format!("{} GB", bytes / 1024 / 1024 / 1024))
        .unwrap_or_else(|| "—".into());

    let model = cmd("sysctl", &["-n", "hw.model"]).unwrap_or_else(|| "Mac".into());

    // Serial number from the IOPlatformExpertDevice registry node.
    let serial = cmd("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])
        .and_then(|out| {
            out.lines()
                .find(|l| l.contains("IOPlatformSerialNumber"))
                .and_then(|l| l.split('=').nth(1))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());

    SysInfo {
        computer_name,
        os,
        chip,
        memory,
        model,
        serial,
    }
}

/// Capitalize the first letter of a word ("charged" → "Charged").
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Pull a top-level `"Key" = value` field out of `ioreg -r` output. The match
/// requires the spaced ` = ` form so it skips the same key inside packed blobs.
fn ioreg_field(io: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = ");
    io.lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with(&needle))
        .map(|l| l[needle.len()..].trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read battery state from `pmset -g batt` (live) + `ioreg` (health metrics).
fn gather_battery() -> Option<BatteryInfo> {
    let pm = cmd("pmset", &["-g", "batt"])?;
    let source = pm
        .lines()
        .next()
        .and_then(|l| l.split('\'').nth(1))
        .unwrap_or("—")
        .to_string();
    let bline = pm.lines().find(|l| l.contains('%'))?;
    let present = !bline.contains("present: false");
    let parts: Vec<&str> = bline.split(';').collect();
    let percent = parts
        .first()
        .and_then(|p| p.split_whitespace().find(|t| t.ends_with('%')))
        .unwrap_or("—")
        .to_string();
    let status = capitalize(parts.get(1).map(|s| s.trim()).unwrap_or("—"));
    let time_remaining = parts
        .get(2)
        .map(|s| s.replace("remaining", "").trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && !s.starts_with("0:00")
                && !s.starts_with("(no estimate)")
                && !s.starts_with("not")
        });

    let io = cmd("ioreg", &["-rn", "AppleSmartBattery"]).unwrap_or_default();
    let cycle_count = ioreg_field(&io, "\"CycleCount\"");
    let raw_max = ioreg_field(&io, "\"AppleRawMaxCapacity\"").and_then(|s| s.parse::<f64>().ok());
    let design = ioreg_field(&io, "\"DesignCapacity\"").and_then(|s| s.parse::<f64>().ok());
    let health_percent = match (raw_max, design) {
        (Some(m), Some(d)) if d > 0.0 => Some(format!("{}%", (m / d * 100.0).round() as i64)),
        _ => None,
    };
    let condition = match ioreg_field(&io, "\"PermanentFailureStatus\"").as_deref() {
        Some("0") | None => "Normal",
        _ => "Service Recommended",
    }
    .to_string();

    Some(BatteryInfo {
        present,
        percent,
        status,
        source,
        time_remaining,
        cycle_count,
        health_percent,
        condition,
    })
}

/// Read audio output/input devices via `system_profiler SPAudioDataType`.
fn gather_audio() -> AudioInfo {
    let out = cmd("system_profiler", &["SPAudioDataType"]).unwrap_or_default();
    let mut outputs: Vec<AudioDevice> = Vec::new();
    let mut inputs: Vec<AudioDevice> = Vec::new();

    let mut cur: Option<String> = None;
    let (mut has_out, mut has_in, mut def_out, mut def_in) = (false, false, false, false);

    let mut flush = |cur: &mut Option<String>, ho: bool, hi: bool, dofl: bool, difl: bool| {
        if let Some(name) = cur.take() {
            if ho {
                outputs.push(AudioDevice {
                    name: name.clone(),
                    is_default: dofl,
                });
            }
            if hi {
                inputs.push(AudioDevice {
                    name,
                    is_default: difl,
                });
            }
        }
    };

    for raw in out.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if indent == 8 && line.ends_with(':') {
            flush(&mut cur, has_out, has_in, def_out, def_in);
            cur = Some(line.trim_end_matches(':').to_string());
            has_out = false;
            has_in = false;
            def_out = false;
            def_in = false;
        } else if cur.is_some() {
            if line.starts_with("Output Source:") {
                has_out = true;
            }
            if line.starts_with("Input Source:") {
                has_in = true;
            }
            if line.starts_with("Default Output Device:") && line.ends_with("Yes") {
                def_out = true;
                has_out = true;
            }
            if line.starts_with("Default Input Device:") && line.ends_with("Yes") {
                def_in = true;
                has_in = true;
            }
        }
    }
    flush(&mut cur, has_out, has_in, def_out, def_in);

    AudioInfo { outputs, inputs }
}

/// Format bytes as decimal GB (matching macOS storage display).
fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

/// Read boot-volume storage usage via `df -k /`.
fn gather_storage() -> StorageInfo {
    let line = cmd("df", &["-k", "/"]).and_then(|o| o.lines().nth(1).map(|s| s.to_string()));
    let cols: Vec<u64> = line
        .as_deref()
        .map(|l| {
            l.split_whitespace()
                .skip(1)
                .take(3)
                .filter_map(|c| c.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let total = cols.first().copied().unwrap_or(0) * 1024;
    let avail = cols.get(2).copied().unwrap_or(0) * 1024;
    // macOS shows "used" as capacity minus free; derive it from total - available.
    let used = total.saturating_sub(avail);
    let volume = cmd("diskutil", &["info", "/"])
        .and_then(|o| {
            o.lines().find_map(|l| {
                l.split_once("Volume Name:")
                    .map(|(_, v)| v.trim().to_string())
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Macintosh HD".into());
    StorageInfo {
        volume,
        total,
        used,
        avail,
    }
}

/// Read the primary network connection (interface, IP, router, DNS, MAC).
fn gather_network() -> NetworkInfo {
    let default_route = cmd("route", &["-n", "get", "default"]).unwrap_or_default();
    let field = |key: &str| -> Option<String> {
        default_route.lines().find_map(|l| {
            let l = l.trim();
            l.strip_prefix(key).map(|s| s.trim().to_string())
        })
    };
    let interface = field("interface:").unwrap_or_default();
    let router = field("gateway:");

    // Map the interface to its hardware-port name (e.g. "Wi-Fi", "Ethernet").
    let ports = cmd("networksetup", &["-listallhardwareports"]).unwrap_or_default();
    let mut service = "Network".to_string();
    let mut mac = None;
    if !interface.is_empty() {
        for block in ports.split("Hardware Port:") {
            if block
                .lines()
                .any(|l| l.trim() == format!("Device: {interface}"))
            {
                if let Some(name) = block.lines().next() {
                    service = name.trim().to_string();
                }
                mac = block.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("Ethernet Address:")
                        .map(|s| s.trim().to_string())
                });
            }
        }
    }

    let ip = if interface.is_empty() {
        None
    } else {
        cmd("ipconfig", &["getifaddr", &interface])
    };

    let dns = cmd("scutil", &["--dns"]).and_then(|o| {
        o.lines().find_map(|l| {
            let l = l.trim();
            if l.starts_with("nameserver[0]") {
                l.split(':').nth(1).map(|s| s.trim().to_string())
            } else {
                None
            }
        })
    });

    NetworkInfo {
        connected: ip.is_some(),
        service,
        interface,
        ip,
        router,
        dns,
        mac,
    }
}

/// Read attached displays + GPU from `system_profiler SPDisplaysDataType`.
fn gather_displays() -> (String, Vec<DisplayInfo>) {
    let out = match cmd("system_profiler", &["SPDisplaysDataType"]) {
        Some(o) => o,
        None => return (String::new(), Vec::new()),
    };
    let mut gpu = String::new();
    let mut displays: Vec<DisplayInfo> = Vec::new();
    for raw in out.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Chipset Model:") {
            if gpu.is_empty() {
                gpu = rest.trim().to_string();
            }
        } else if indent == 8 && line.ends_with(':') && line != "Displays:" {
            displays.push(DisplayInfo {
                name: line.trim_end_matches(':').to_string(),
                resolution: "—".into(),
                detail: None,
                is_main: false,
            });
        } else if let Some(d) = displays.last_mut() {
            if let Some(r) = line.strip_prefix("Resolution:") {
                d.resolution = r.trim().to_string();
            } else if let Some(t) = line.strip_prefix("Display Type:") {
                d.detail = Some(t.trim().to_string());
            } else if line.starts_with("Main Display:") && line.ends_with("Yes") {
                d.is_main = true;
            }
        }
    }
    (gpu, displays)
}

fn categories() -> Vec<Vec<Category>> {
    let blue = hsl(0x0a84ff);
    let gray = hsl(0x8e8e93);
    let green = hsl(0x34c759);
    let red = hsl(0xff3b30);
    let pink = hsl(0xff2d55);
    let indigo = hsl(0x5e5ce6);
    let purple = hsl(0xaf52de);
    let teal = hsl(0x30b0c7);

    let row = |icon: &'static str, color: Hsla, label: &str| Row {
        icon,
        color,
        label: label.to_string().into(),
    };
    let cat =
        |name: &str, icon: &'static str, color: Hsla, desc: &str, cards: Vec<Vec<Row>>| Category {
            name: name.to_string().into(),
            icon,
            color,
            desc: desc.to_string().into(),
            cards,
        };

    vec![
        vec![
            cat("Wi-Fi", "icons/wifi.svg", blue, "Connect to Wi-Fi networks and manage known networks.", vec![]),
            cat("Bluetooth", "icons/bluetooth.svg", blue, "Pair and manage Bluetooth devices.", vec![]),
            cat("Network", "icons/globe.svg", blue, "Configure network services and connections.", vec![]),
            cat("VPN", "icons/key.svg", blue, "Set up and manage VPN configurations.", vec![]),
            cat("Battery", "icons/battery-charging.svg", green, "Monitor battery usage and energy settings.", vec![]),
        ],
        vec![
            cat(
                "General",
                "icons/settings.svg",
                gray,
                "Manage your overall setup and preferences for Mac, such as software updates, device language, AirDrop, and more.",
                vec![
                    vec![
                        row("icons/info.svg", gray, "About"),
                        row("icons/refresh-cw.svg", gray, "Software Update"),
                        row("icons/database.svg", gray, "Storage"),
                    ],
                    vec![row("icons/heart-handshake.svg", red, "AppleCare & Warranty")],
                    vec![row("icons/folder-symlink.svg", blue, "AirDrop & Continuity")],
                    vec![
                        row("icons/key.svg", gray, "AutoFill & Passwords"),
                        row("icons/clock.svg", gray, "Date & Time"),
                        row("icons/languages.svg", blue, "Language & Region"),
                        row("icons/power.svg", gray, "Login Items & Extensions"),
                        row("icons/folder-symlink.svg", blue, "Sharing"),
                        row("icons/hard-drive.svg", gray, "Startup Disk"),
                        row("icons/history.svg", green, "Time Machine"),
                    ],
                ],
            ),
            cat("Accessibility", "icons/accessibility.svg", blue, "Customize your Mac for the way you work.", vec![]),
            cat("Appearance", "icons/palette.svg", hsl(0x1d1d1f), "Change how windows, buttons, and menus look.", vec![]),
            cat("Apple Intelligence & Siri", "icons/sparkles.svg", purple, "Set up Apple Intelligence and Siri.", vec![]),
            cat("Desktop & Dock", "icons/app-window.svg", gray, "Adjust the Dock, Stage Manager, and windows.", vec![]),
            cat("Displays", "icons/monitor.svg", blue, "Arrange displays and adjust resolution.", vec![]),
            cat("Spotlight", "icons/search.svg", gray, "Choose which categories Spotlight searches.", vec![]),
            cat("Wallpaper", "icons/image.svg", teal, "Choose a wallpaper for your desktop.", vec![]),
        ],
        vec![
            cat("Notifications", "icons/bell.svg", red, "Choose how you receive notifications.", vec![]),
            cat("Sound", "icons/volume-2.svg", pink, "Adjust sound effects and output.", vec![]),
            cat("Focus", "icons/moon.svg", indigo, "Stay focused by silencing notifications.", vec![]),
            cat("Screen Time", "icons/timer.svg", indigo, "Monitor usage and set limits.", vec![]),
        ],
        vec![
            cat("Lock Screen", "icons/lock.svg", gray, "Adjust your lock screen and login.", vec![]),
            cat("Privacy & Security", "icons/shield.svg", blue, "Control what your Mac and apps can access.", vec![]),
        ],
    ]
}

fn main() {
    rmac_ui::boot_unified_with_assets(CombinedAssets, 1000.0, 720.0, |window, cx| {
        cx.bind_keys([KeyBinding::new("cmd-[", GoBack, Some("SystemSettings"))]);
        Settings::new(window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::Persisted;

    #[test]
    fn persisted_settings_round_trip() {
        let expected = Persisted {
            wifi_on: false,
            ask_to_join: false,
            joined: Some(3),
            bluetooth_on: false,
            bt_discoverable: false,
            appearance: 2,
            accent_idx: 4,
            show_color_in_menu: false,
            large_sidebar: true,
            output_volume: 31.0,
            alert_volume: 42.0,
            balance: 63.0,
            mute: true,
            play_on_startup: false,
            play_ui_sounds: false,
            alert_idx: 5,
            handoff: false,
            airdrop_idx: 2,
            airplay_receiver: true,
        };

        let parsed = Persisted::parse(&expected.to_json()).unwrap();

        assert_eq!(parsed, expected);
    }

    #[test]
    fn malformed_existing_settings_are_reported() {
        assert!(Persisted::parse("{\"wifi_on\": nope}").is_err());
        assert!(Persisted::parse("{\"wifi_on\": 1").is_err());
    }

    #[test]
    fn persisted_ranges_are_safe_for_ui_controls() {
        let parsed = Persisted::parse(
            r#"{
                "output_volume": 999,
                "alert_volume": -10,
                "balance": 101,
                "accent_idx": 999,
                "alert_idx": 999,
                "airdrop_idx": 999
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.output_volume, 100.0);
        assert_eq!(parsed.alert_volume, 0.0);
        assert_eq!(parsed.balance, 100.0);
        assert_eq!(parsed.accent_idx, 0);
        assert_eq!(parsed.alert_idx, 0);
        assert_eq!(parsed.airdrop_idx, 1);
    }
}
