//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · local account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive: Wi-Fi is service-backed; remaining local controls are being
//! migrated pane-by-pane to typed Linux/macOS services.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Read-only panes use real platform state rather than fabricated values.

mod storage;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AnyElement, App, AppContext as _,
    AssetSource, Context, Div, ElementId, Entity, FocusHandle, Hsla, InteractiveElement as _,
    IntoElement, KeyBinding, MouseButton, ParentElement, Render, Result, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Svg, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{
    EmptyState, InputState, ListRow, Progress, SearchField, Slider, SliderEvent, SliderState,
    Toast, ToastKind, Toggle,
};

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
    rmac_ui::mac::sidebar()
}
fn pane_bg() -> Hsla {
    rmac_ui::mac::window()
}
fn card_bg() -> Hsla {
    rmac_ui::mac::raised()
}
fn accent() -> Hsla {
    rmac_ui::mac::accent()
}
fn label() -> Hsla {
    rmac_ui::mac::text()
}
fn secondary() -> Hsla {
    rmac_ui::mac::text_secondary()
}
fn sep() -> Hsla {
    rmac_ui::mac::separator()
}
fn white() -> Hsla {
    gpui::white()
}
fn on_accent() -> Hsla {
    rmac_ui::mac::on_accent()
}
fn swatch_foreground(hex: u32) -> Hsla {
    let swatch = rmac_ui::theme::RgbaColor::opaque(hex);
    let white = rmac_ui::theme::RgbaColor::opaque(0xffffff);
    let black = rmac_ui::theme::RgbaColor::opaque(0x000000);
    if swatch.contrast_ratio(white) >= swatch.contrast_ratio(black) {
        white.hsla()
    } else {
        black.hsla()
    }
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

/// A navigation subpage pushed onto the back stack from a row chevron.
#[derive(Clone)]
enum SubPage {
    About,
    SoftwareUpdate,
    Storage,
    NotificationApp {
        app_id: String,
    },
    FocusMode {
        mode_id: String,
    },
    FocusSchedule {
        schedule_id: String,
    },
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

/// Boot-volume storage usage (read once after launch via `df`).
#[derive(Default)]
struct StorageInfo {
    volume: String,
    total: u64,
    used: u64,
    avail: u64,
}

struct Settings {
    system_data_loading: bool,
    account: SharedString,
    sysinfo: SysInfo,
    power: rmac_power::Snapshot,
    display: rmac_display::Snapshot,
    network: rmac_network::NetworkSnapshot,
    storage: StorageInfo,
    audio: rmac_audio::Snapshot,
    input: rmac_input::Snapshot,
    sections: Vec<Vec<Category>>,
    selected: (usize, usize),
    nav: Vec<SubPage>,
    search: Entity<InputState>,
    focus: FocusHandle,
    focused_once: bool,
    dragging: bool,
    persistence_error: Option<SharedString>,
    wifi_error: Option<SharedString>,
    bluetooth_error: Option<SharedString>,
    network_error: Option<SharedString>,
    vpn_error: Option<SharedString>,
    audio_error: Option<SharedString>,
    power_error: Option<SharedString>,
    display_error: Option<SharedString>,
    input_error: Option<SharedString>,
    theme_error: Option<SharedString>,
    notification_error: Option<SharedString>,
    notification_stream_error: Option<SharedString>,

    // Notifications
    notifications_loading: bool,
    notification_busy: Option<String>,
    notification_apps: Vec<rmac_notifications_linux::center::ApplicationPolicy>,
    app_catalog: Vec<rmac_apps::Application>,
    _app_catalog_watcher: Option<rmac_apps::CatalogWatcher>,

    // Focus
    focus_policy_loading: bool,
    focus_policy_busy: bool,
    focus_policy_error: Option<SharedString>,
    focus_policy_stream_error: Option<SharedString>,
    focus_policy_config: Option<rmac_focus::Config>,
    focus_policy_state: Option<rmac_focus_linux::client::Snapshot>,

    // Lock Screen
    lock_policy_loading: bool,
    lock_policy_busy: bool,
    lock_policy_error: Option<SharedString>,
    lock_policy_stream_error: Option<SharedString>,
    lock_policy: Option<rmac_shortcuts::lock_settings::Snapshot>,

    // Network
    network_loading: bool,
    network_busy: bool,

    // VPN
    vpn: rmac_network::VpnSnapshot,
    vpn_loading: bool,
    vpn_busy: Option<String>,

    // Wi-Fi
    wifi_available: bool,
    wifi_loading: bool,
    wifi_busy: bool,
    wifi_on: bool,
    ask_to_join: bool,
    joined: Option<usize>,
    wifi_interface: Option<String>,
    wifi_networks: Vec<rmac_network::WifiNetwork>,

    // Bluetooth
    bluetooth_available: bool,
    bluetooth_loading: bool,
    bluetooth_busy: bool,
    bluetooth_discovering: bool,
    bluetooth_adapter_name: Option<String>,
    bluetooth_on: bool,
    bt_discoverable: bool,
    bt_devices: Vec<rmac_bluetooth::Device>,

    // Appearance
    host_appearance: rmac_appearance::Snapshot,
    theme: Option<rmac_theme::Snapshot>,
    theme_loading: bool,
    theme_busy: bool,

    // Sound
    audio_loading: bool,
    audio_busy: bool,
    output_volume_generation: u64,
    input_volume_generation: u64,
    output_volume: Entity<SliderState>,
    input_volume: Entity<SliderState>,
    alert_volume: Entity<SliderState>,
    balance: Entity<SliderState>,
    play_on_startup: bool,
    play_ui_sounds: bool,
    alert_idx: usize,

    // Battery and power profiles
    power_loading: bool,
    power_busy: bool,

    // Displays
    display_loading: bool,
    display_busy: bool,
    display_revert: Option<DisplayChange>,

    // Keyboard, mouse, and trackpad
    input_loading: bool,
    input_busy: bool,

    // General
    handoff: bool,
    airdrop_idx: usize,
    airplay_receiver: bool,
}

enum AudioChange {
    Volume(rmac_audio::DeviceKind, u8),
    Muted(rmac_audio::DeviceKind, bool),
    DefaultDevice(rmac_audio::DeviceKind, String),
}

#[derive(Clone, Copy)]
enum InputChange {
    KeyboardRepeatDelay(u32),
    KeyboardRepeatRate(u32),
    KeyboardNumlock(bool),
    MouseNaturalScroll(bool),
    MouseLeftHanded(bool),
    MouseAccelSpeed(f64),
    MouseAccelProfile(rmac_input::AccelProfile),
    TouchpadNaturalScroll(bool),
    TouchpadLeftHanded(bool),
    TouchpadAccelSpeed(f64),
    TouchpadAccelProfile(rmac_input::AccelProfile),
    TouchpadTap(bool),
    TouchpadDwt(bool),
    TouchpadDragLock(bool),
}

#[derive(Clone, Copy)]
enum ThemeChange {
    Scheme(rmac_theme::SchemePreference),
    Accent(rmac_theme::AccentPreference),
    Contrast(rmac_theme::ContrastPreference),
    Motion(rmac_theme::MotionPreferenceSetting),
}

#[derive(Clone, Copy)]
enum NotificationPolicyChange {
    Enabled(bool),
    Badges(bool),
    History(bool),
}

fn notification_policy_with(
    mut policy: rmac_notifications_store::AppPolicy,
    change: NotificationPolicyChange,
) -> rmac_notifications_store::AppPolicy {
    match change {
        NotificationPolicyChange::Enabled(value) => policy.enabled = value,
        NotificationPolicyChange::Badges(value) => policy.badges = value,
        NotificationPolicyChange::History(value) => policy.history = value,
    }
    policy
}

#[derive(Clone)]
enum DisplayChange {
    Mode {
        output: String,
        mode: rmac_display::Mode,
    },
    Scale {
        output: String,
        scale: f64,
    },
    Transform {
        output: String,
        transform: rmac_display::Transform,
    },
}

impl DisplayChange {
    fn apply(&self) -> std::result::Result<(), rmac_display::Error> {
        match self {
            Self::Mode { output, mode } => rmac_display::set_mode(output, *mode),
            Self::Scale { output, scale } => rmac_display::set_scale(output, *scale),
            Self::Transform { output, transform } => rmac_display::set_transform(output, transform),
        }
    }
}

/// Read-only system data that is slow enough to keep off the first-frame path.
struct SystemSnapshot {
    account: String,
    sysinfo: SysInfo,
    storage: StorageInfo,
}

struct ThemeLoad {
    host: rmac_appearance::Snapshot,
    theme: rmac_theme::Snapshot,
}

struct FocusLoad {
    configuration: rmac_focus::Config,
    state: rmac_focus_linux::client::Snapshot,
}

fn load_focus() -> std::result::Result<FocusLoad, rmac_focus_linux::client::Error> {
    let snapshot = rmac_focus_linux::client::settings()?;
    Ok(FocusLoad {
        configuration: snapshot.configuration,
        state: snapshot.state,
    })
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

const FOCUS_DAYS: [(rmac_focus::Weekday, &str); 7] = [
    (rmac_focus::Weekday::Monday, "M"),
    (rmac_focus::Weekday::Tuesday, "T"),
    (rmac_focus::Weekday::Wednesday, "W"),
    (rmac_focus::Weekday::Thursday, "T"),
    (rmac_focus::Weekday::Friday, "F"),
    (rmac_focus::Weekday::Saturday, "S"),
    (rmac_focus::Weekday::Sunday, "S"),
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
}

impl Settings {
    fn audio_slider(
        cx: &mut Context<Self>,
        value: f32,
        kind: rmac_audio::DeviceKind,
    ) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event;
            this.schedule_audio_volume(kind, value.start(), cx);
            cx.notify();
        })
        .detach();
        slider
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        // A missing config is a normal first launch. Existing-but-unreadable or
        // malformed state falls back safely and remains visible to the user.
        let (saved, persistence_error) = match Persisted::load() {
            Ok(saved) => (saved, None),
            Err(failure) => (Persisted::default(), Some(failure.to_string().into())),
        };

        let (catalog_events, catalog_event_rx) = async_channel::bounded(1);
        let app_catalog_watcher = rmac_apps::watch_catalog(move || {
            let _ = catalog_events.try_send(());
        })
        .ok();

        // Shell-owned sliders persist locally. System audio sliders are created
        // separately below and write through the platform audio service.
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
        let output_volume =
            Self::audio_slider(cx, saved.output_volume, rmac_audio::DeviceKind::Output);
        let input_volume = Self::audio_slider(cx, 0.0, rmac_audio::DeviceKind::Input);
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

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_bluetooth::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::network_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_network_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::vpn_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_vpn_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_audio::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_power::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                cx.notify();
            });
        })
        .detach();

        let (notification_updates, notification_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_notifications_linux::center::watch_applications(notification_updates)
                    .await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = notification_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        this.apply_notification_stream_update(update);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            let result = cx
                .background_executor()
                .spawn(async { rmac_apps::discover() })
                .await;
            if let Ok(applications) = result {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        this.app_catalog = applications;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
            if catalog_event_rx.recv().await.is_err() {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            while catalog_event_rx.try_recv().is_ok() {}
        })
        .detach();

        let (focus_updates, focus_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_focus_linux::client::watch_settings(focus_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = focus_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        this.apply_focus_stream_update(update);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (lock_updates, lock_update_rx) = async_channel::bounded(4);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_shortcuts::lock_settings::watch(lock_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = lock_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        this.apply_lock_policy_stream_update(update);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self {
            system_data_loading: true,
            account: std::env::var("USER")
                .unwrap_or_else(|_| "User".into())
                .into(),
            sysinfo: SysInfo::default(),
            power: rmac_power::Snapshot::default(),
            display: rmac_display::Snapshot::default(),
            network: rmac_network::NetworkSnapshot::default(),
            storage: StorageInfo::default(),
            audio: rmac_audio::Snapshot::default(),
            input: rmac_input::Snapshot::default(),
            sections: categories(),
            selected: (1, 0), // General
            nav: Vec::new(),
            search,
            focus: cx.focus_handle(),
            focused_once: false,
            dragging: false,
            persistence_error,
            wifi_error: None,
            bluetooth_error: None,
            network_error: None,
            vpn_error: None,
            audio_error: None,
            power_error: None,
            display_error: None,
            input_error: None,
            theme_error: None,
            notification_error: None,
            notification_stream_error: None,

            notifications_loading: true,
            notification_busy: None,
            notification_apps: Vec::new(),
            app_catalog: Vec::new(),
            _app_catalog_watcher: app_catalog_watcher,

            focus_policy_loading: true,
            focus_policy_busy: false,
            focus_policy_error: None,
            focus_policy_stream_error: None,
            focus_policy_config: None,
            focus_policy_state: None,

            lock_policy_loading: true,
            lock_policy_busy: false,
            lock_policy_error: None,
            lock_policy_stream_error: None,
            lock_policy: None,

            network_loading: true,
            network_busy: false,

            vpn: rmac_network::VpnSnapshot::default(),
            vpn_loading: true,
            vpn_busy: None,

            wifi_available: false,
            wifi_loading: true,
            wifi_busy: false,
            wifi_on: saved.wifi_on,
            ask_to_join: saved.ask_to_join,
            joined: saved.joined,
            wifi_interface: None,
            wifi_networks: Vec::new(),

            bluetooth_available: false,
            bluetooth_loading: true,
            bluetooth_busy: false,
            bluetooth_discovering: false,
            bluetooth_adapter_name: None,
            bluetooth_on: saved.bluetooth_on,
            bt_discoverable: saved.bt_discoverable,
            bt_devices: Vec::new(),

            host_appearance: rmac_appearance::Snapshot::default(),
            theme: None,
            theme_loading: true,
            theme_busy: false,

            audio_loading: true,
            audio_busy: false,
            output_volume_generation: 0,
            input_volume_generation: 0,
            output_volume,
            input_volume,
            alert_volume,
            balance,
            play_on_startup: saved.play_on_startup,
            play_ui_sounds: saved.play_ui_sounds,
            alert_idx: saved.alert_idx,

            power_loading: true,
            power_busy: false,

            display_loading: true,
            display_busy: false,
            display_revert: None,

            input_loading: true,
            input_busy: false,

            handoff: saved.handoff,
            airdrop_idx: saved.airdrop_idx,
            airplay_receiver: saved.airplay_receiver,
        }
    }

    fn apply_system_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.account = snapshot.account.into();
        self.sysinfo = snapshot.sysinfo;
        self.storage = snapshot.storage;
        self.system_data_loading = false;
    }

    fn finish_wifi_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        match result {
            Ok(snapshot) => {
                self.wifi_available = snapshot.available;
                self.wifi_on = snapshot.enabled;
                self.wifi_interface = snapshot.interface;
                self.wifi_networks = snapshot.networks;
                self.wifi_error = None;
            }
            Err(error) => {
                self.wifi_error = Some(format!("Could not update Wi-Fi: {error}").into());
            }
        }
    }

    fn finish_network_update(
        &mut self,
        result: std::result::Result<rmac_network::NetworkSnapshot, rmac_network::Error>,
    ) {
        self.network_loading = false;
        self.network_busy = false;
        match result {
            Ok(snapshot) => {
                self.network = snapshot;
                self.network_error = None;
            }
            Err(error) => {
                self.network_error = Some(format!("Could not update Network: {error}").into());
            }
        }
    }

    fn refresh_network(&mut self, cx: &mut Context<Self>) {
        if self.network_busy || self.network_loading {
            return;
        }
        self.network_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::network_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_network_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_vpn_update(
        &mut self,
        result: std::result::Result<rmac_network::VpnSnapshot, rmac_network::Error>,
    ) {
        self.vpn_loading = false;
        self.vpn_busy = None;
        match result {
            Ok(snapshot) => {
                self.vpn = snapshot;
                self.vpn_error = None;
            }
            Err(error) => {
                self.vpn_error = Some(format!("Could not update VPN: {error}").into());
            }
        }
    }

    fn refresh_vpn(&mut self, cx: &mut Context<Self>) {
        if self.vpn_loading || self.vpn_busy.is_some() {
            return;
        }
        self.vpn_busy = Some(String::new());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::vpn_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_vpn_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_vpn_enabled(&mut self, identifier: String, enabled: bool, cx: &mut Context<Self>) {
        if self.vpn_loading || self.vpn_busy.is_some() {
            return;
        }
        self.vpn_busy = Some(identifier.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_network::set_vpn_enabled(&identifier, enabled)?;
                    std::thread::sleep(Duration::from_millis(500));
                    rmac_network::vpn_snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_vpn_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_audio_update(
        &mut self,
        result: std::result::Result<rmac_audio::Snapshot, rmac_audio::Error>,
        cx: &mut Context<Self>,
    ) {
        self.audio_loading = false;
        self.audio_busy = false;
        match result {
            Ok(snapshot) => {
                self.output_volume_generation = self.output_volume_generation.wrapping_add(1);
                self.input_volume_generation = self.input_volume_generation.wrapping_add(1);
                self.output_volume = Self::audio_slider(
                    cx,
                    f32::from(snapshot.output.volume),
                    rmac_audio::DeviceKind::Output,
                );
                self.input_volume = Self::audio_slider(
                    cx,
                    f32::from(snapshot.input.volume),
                    rmac_audio::DeviceKind::Input,
                );
                self.audio = snapshot;
                self.audio_error = None;
            }
            Err(error) => {
                self.audio_error = Some(format!("Could not update Sound: {error}").into());
            }
        }
    }

    fn refresh_audio(&mut self, cx: &mut Context<Self>) {
        if self.audio_loading || self.audio_busy {
            return;
        }
        self.audio_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_audio::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_audio_volume(
        &mut self,
        kind: rmac_audio::DeviceKind,
        volume: f32,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || !self.audio.available {
            return;
        }
        let generation = match kind {
            rmac_audio::DeviceKind::Output => {
                self.output_volume_generation = self.output_volume_generation.wrapping_add(1);
                self.output_volume_generation
            }
            rmac_audio::DeviceKind::Input => {
                self.input_volume_generation = self.input_volume_generation.wrapping_add(1);
                self.input_volume_generation
            }
        };
        let volume = volume.round().clamp(0.0, 100.0) as u8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let current = match kind {
                    rmac_audio::DeviceKind::Output => this.output_volume_generation,
                    rmac_audio::DeviceKind::Input => this.input_volume_generation,
                };
                if current == generation && !this.audio_busy {
                    this.apply_audio_change(AudioChange::Volume(kind, volume), cx);
                }
            });
        })
        .detach();
    }

    fn set_audio_muted(
        &mut self,
        kind: rmac_audio::DeviceKind,
        muted: bool,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Muted(kind, muted), cx);
    }

    fn set_default_audio_device(
        &mut self,
        kind: rmac_audio::DeviceKind,
        id: String,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading
            || self.audio_busy
            || !self.audio.available
            || !self.audio.can_set_default
        {
            return;
        }
        self.apply_audio_change(AudioChange::DefaultDevice(kind, id), cx);
    }

    fn apply_audio_change(&mut self, change: AudioChange, cx: &mut Context<Self>) {
        self.audio_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match change {
                        AudioChange::Volume(kind, volume) => rmac_audio::set_volume(kind, volume)?,
                        AudioChange::Muted(kind, muted) => rmac_audio::set_muted(kind, muted)?,
                        AudioChange::DefaultDevice(kind, id) => {
                            rmac_audio::set_default_device(kind, &id)?
                        }
                    }
                    rmac_audio::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_power_update(
        &mut self,
        result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
    ) {
        self.power_loading = false;
        self.power_busy = false;
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_error = None;
            }
            Err(error) => {
                self.power_error = Some(format!("Could not update Battery: {error}").into());
            }
        }
    }

    fn refresh_power(&mut self, cx: &mut Context<Self>) {
        if self.power_loading || self.power_busy {
            return;
        }
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_power::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_power_profile(&mut self, profile: rmac_power::PowerProfile, cx: &mut Context<Self>) {
        if self.power_loading
            || self.power_busy
            || !self.power.profiles.available
            || !self.power.profiles.supported.contains(&profile)
        {
            return;
        }
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_power::set_profile(profile)?;
                    rmac_power::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_display_update(
        &mut self,
        result: std::result::Result<rmac_display::Snapshot, rmac_display::Error>,
    ) {
        self.display_loading = false;
        self.display_busy = false;
        match result {
            Ok(snapshot) => {
                self.display = snapshot;
                self.display_error = None;
            }
            Err(error) => {
                self.display_error = Some(format!("Could not update Displays: {error}").into());
            }
        }
    }

    fn refresh_displays(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy {
            return;
        }
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                if succeeded {
                    this.display_revert = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_display_change(
        &mut self,
        change: DisplayChange,
        revert: DisplayChange,
        cx: &mut Context<Self>,
    ) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    change.apply()?;
                    rmac_display::snapshot()
                })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                if succeeded {
                    this.display_revert = Some(revert);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        let Some(revert) = self.display_revert.clone() else {
            return;
        };
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    revert.apply()?;
                    rmac_display::snapshot()
                })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                if succeeded {
                    this.display_revert = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_input_update(
        &mut self,
        result: std::result::Result<rmac_input::Snapshot, rmac_input::Error>,
    ) {
        self.input_loading = false;
        self.input_busy = false;
        match result {
            Ok(snapshot) => {
                self.input = snapshot;
                self.input_error = None;
            }
            Err(error) => {
                self.input_error = Some(format!("Could not update Input settings: {error}").into());
            }
        }
    }

    fn finish_theme_update(&mut self, result: std::result::Result<ThemeLoad, String>) {
        self.theme_loading = false;
        self.theme_busy = false;
        match result {
            Ok(load) => {
                self.host_appearance = load.host;
                self.theme = Some(load.theme);
                self.theme_error = None;
            }
            Err(error) => {
                self.theme_error = Some(format!("Could not update Appearance: {error}").into());
            }
        }
    }

    fn finish_notifications_update(
        &mut self,
        result: std::result::Result<
            Vec<rmac_notifications_linux::center::ApplicationPolicy>,
            rmac_notifications_linux::center::Error,
        >,
    ) {
        self.notifications_loading = false;
        self.notification_busy = None;
        match result {
            Ok(applications) => {
                self.notification_apps = applications;
                self.notification_error = None;
            }
            Err(error) => {
                self.notification_error =
                    Some(format!("Could not update Notifications: {error}").into());
            }
        }
    }

    fn apply_notification_stream_update(
        &mut self,
        update: std::result::Result<
            Vec<rmac_notifications_linux::center::ApplicationPolicy>,
            String,
        >,
    ) {
        self.notifications_loading = false;
        match update {
            Ok(applications) => {
                self.notification_apps = applications;
                self.notification_stream_error = None;
            }
            Err(error) => {
                self.notification_stream_error =
                    Some(format!("Live Notification updates unavailable: {error}").into());
            }
        }
    }

    fn refresh_notifications(&mut self, cx: &mut Context<Self>) {
        if self.notifications_loading || self.notification_busy.is_some() {
            return;
        }
        self.notifications_loading = true;
        self.notification_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_notifications_linux::center::applications() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_notifications_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_notification_policy(
        &mut self,
        app_id: String,
        change: NotificationPolicyChange,
        cx: &mut Context<Self>,
    ) {
        if self.notifications_loading || self.notification_busy.is_some() {
            return;
        }
        let Some(application) = self
            .notification_apps
            .iter()
            .find(|application| application.app_id == app_id)
        else {
            self.notification_error = Some("That application is no longer available.".into());
            cx.notify();
            return;
        };
        let policy = notification_policy_with(application.policy, change);
        self.notification_busy = Some(app_id.clone());
        self.notification_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, applications) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_notifications_linux::center::set_policy(&app_id, policy);
                    let applications = rmac_notifications_linux::center::applications();
                    (mutation, applications)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.notification_busy = None;
                let refresh_error = match applications {
                    Ok(applications) => {
                        this.notification_apps = applications;
                        None
                    }
                    Err(error) => Some(format!("Could not refresh Notifications: {error}").into()),
                };
                this.notification_error = mutation
                    .err()
                    .map(|error| format!("Could not change Notifications: {error}").into())
                    .or(refresh_error);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_focus_update(
        &mut self,
        result: std::result::Result<FocusLoad, rmac_focus_linux::client::Error>,
    ) {
        self.focus_policy_loading = false;
        self.focus_policy_busy = false;
        match result {
            Ok(load) => {
                self.focus_policy_config = Some(load.configuration);
                self.focus_policy_state = Some(load.state);
                self.focus_policy_error = None;
            }
            Err(error) => {
                self.focus_policy_error = Some(format!("Could not update Focus: {error}").into());
            }
        }
    }

    fn apply_focus_stream_update(
        &mut self,
        update: std::result::Result<rmac_focus_linux::client::SettingsSnapshot, String>,
    ) {
        self.focus_policy_loading = false;
        match update {
            Ok(update) => {
                self.focus_policy_config = Some(update.configuration);
                self.focus_policy_state = Some(update.state);
                self.focus_policy_stream_error = None;
            }
            Err(error) => {
                self.focus_policy_stream_error =
                    Some(format!("Live Focus updates unavailable: {error}").into());
            }
        }
    }

    fn refresh_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_loading = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx.background_executor().spawn(async { load_focus() }).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn activate_focus(&mut self, mode_id: String, duration_ms: u64, cx: &mut Context<Self>) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_focus_linux::client::activate(&mode_id, duration_ms);
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, None);
                cx.notify();
            });
        })
        .detach();
    }

    fn disable_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async {
                    let mutation = rmac_focus_linux::client::disable();
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, None);
                cx.notify();
            });
        })
        .detach();
    }

    fn replace_focus_configuration(
        &mut self,
        configuration: rmac_focus::Config,
        cx: &mut Context<Self>,
    ) {
        if self.focus_policy_loading || self.focus_policy_busy {
            return;
        }
        let previous_configuration = self.focus_policy_config.clone();
        self.focus_policy_busy = true;
        self.focus_policy_error = None;
        self.focus_policy_config = Some(configuration.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (mutation, reload) = cx
                .background_executor()
                .spawn(async move {
                    let mutation = rmac_focus_linux::client::replace_configuration(&configuration);
                    let reload = load_focus();
                    (mutation, reload)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_focus_mutation(mutation, reload, previous_configuration);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_focus_mutation(
        &mut self,
        mutation: std::result::Result<
            rmac_focus_linux::client::Snapshot,
            rmac_focus_linux::client::Error,
        >,
        reload: std::result::Result<FocusLoad, rmac_focus_linux::client::Error>,
        rollback: Option<rmac_focus::Config>,
    ) {
        self.focus_policy_busy = false;
        let mutation_failed = mutation.is_err();
        let reload_error = match reload {
            Ok(load) => {
                self.focus_policy_config = Some(load.configuration);
                self.focus_policy_state = Some(load.state);
                None
            }
            Err(error) => {
                if mutation_failed {
                    self.focus_policy_config = rollback;
                }
                Some(format!("Could not refresh Focus: {error}").into())
            }
        };
        self.focus_policy_error = mutation
            .err()
            .map(|error| format!("Could not change Focus: {error}").into())
            .or(reload_error);
    }

    fn set_focus_urgent(&mut self, mode_id: String, enabled: bool, cx: &mut Context<Self>) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(mode_id) = rmac_focus::ModeId::parse(mode_id) else {
            self.focus_policy_error = Some("The Focus mode is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_mode_urgent(configuration, &mode_id, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus mode.".into());
                cx.notify();
            }
        }
    }

    fn set_focus_allowed_app(
        &mut self,
        mode_id: String,
        app_id: String,
        allowed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let (Ok(mode_id), Ok(app_id)) = (
            rmac_focus::ModeId::parse(mode_id),
            rmac_notifications::AppId::parse(app_id),
        ) else {
            self.focus_policy_error = Some("The Focus application rule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_allowed_app(configuration, &mode_id, app_id, allowed) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus mode.".into());
                cx.notify();
            }
        }
    }

    fn set_focus_schedule_enabled(
        &mut self,
        schedule_id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_schedule_enabled(configuration, &schedule_id, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    fn add_focus_schedule(&mut self, mode_id: String, cx: &mut Context<Self>) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(mode_id) = rmac_focus::ModeId::parse(mode_id) else {
            self.focus_policy_error = Some("The Focus mode is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::create_schedule(configuration, &mode_id) {
            Ok((configuration, schedule_id)) => {
                let schedule_id = schedule_id.as_str().to_owned();
                self.replace_focus_configuration(configuration, cx);
                self.push(SubPage::FocusSchedule { schedule_id }, cx);
            }
            Err(rmac_focus_settings::Error::Limit) => {
                self.focus_policy_error =
                    Some("Focus already has the maximum number of schedules.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not create a Focus schedule.".into());
                cx.notify();
            }
        }
    }

    fn set_focus_schedule_day(
        &mut self,
        schedule_id: String,
        day: rmac_focus::Weekday,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::set_schedule_day(configuration, &schedule_id, day, enabled) {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(rmac_focus_settings::Error::Invalid) => {
                self.focus_policy_error =
                    Some("A Focus schedule must include at least one day.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    fn set_focus_schedule_time(
        &mut self,
        schedule_id: String,
        start: bool,
        minute: u16,
        cx: &mut Context<Self>,
    ) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        let result = if start {
            rmac_focus_settings::set_schedule_start(configuration, &schedule_id, minute)
        } else {
            rmac_focus_settings::set_schedule_end(configuration, &schedule_id, minute)
        };
        match result {
            Ok(configuration) => self.replace_focus_configuration(configuration, cx),
            Err(rmac_focus_settings::Error::Invalid) => {
                self.focus_policy_error =
                    Some("A Focus schedule needs different start and end times.".into());
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not edit that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    fn remove_focus_schedule(&mut self, schedule_id: String, cx: &mut Context<Self>) {
        let Some(configuration) = &self.focus_policy_config else {
            return;
        };
        let Ok(schedule_id) = rmac_focus::ScheduleId::parse(schedule_id) else {
            self.focus_policy_error = Some("The Focus schedule is invalid.".into());
            cx.notify();
            return;
        };
        match rmac_focus_settings::remove_schedule(configuration, &schedule_id) {
            Ok(configuration) => {
                self.replace_focus_configuration(configuration, cx);
                self.nav.pop();
                cx.notify();
            }
            Err(_) => {
                self.focus_policy_error = Some("Could not remove that Focus schedule.".into());
                cx.notify();
            }
        }
    }

    fn refresh_theme(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy {
            return;
        }
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_theme_change(&mut self, change: ThemeChange, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy {
            return;
        }
        let Some(theme) = &self.theme else {
            return;
        };
        let mut preferences = theme.preferences.clone();
        match change {
            ThemeChange::Scheme(value) => preferences.color_scheme = value,
            ThemeChange::Accent(value) => preferences.accent_color = value,
            ThemeChange::Contrast(value) => preferences.contrast = value,
            ThemeChange::Motion(value) => preferences.motion = value,
        }
        let host = self.host_appearance.clone();
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = rmac_theme::ThemeStore::from_environment()
                        .map_err(|error| error.to_string())?;
                    let theme = store
                        .save(&preferences, &host)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(ThemeLoad { host, theme })
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_input(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy {
            return;
        }
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_input_change(&mut self, change: InputChange, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || !self.input.can_configure {
            return;
        }
        let mut settings = self.input.settings.clone();
        match change {
            InputChange::KeyboardRepeatDelay(value) => settings.keyboard.repeat_delay_ms = value,
            InputChange::KeyboardRepeatRate(value) => settings.keyboard.repeat_rate = value,
            InputChange::KeyboardNumlock(value) => settings.keyboard.numlock = value,
            InputChange::MouseNaturalScroll(value) => settings.mouse.natural_scroll = value,
            InputChange::MouseLeftHanded(value) => settings.mouse.left_handed = value,
            InputChange::MouseAccelSpeed(value) => settings.mouse.accel_speed = value,
            InputChange::MouseAccelProfile(value) => settings.mouse.accel_profile = value,
            InputChange::TouchpadNaturalScroll(value) => {
                settings.touchpad.pointer.natural_scroll = value
            }
            InputChange::TouchpadLeftHanded(value) => settings.touchpad.pointer.left_handed = value,
            InputChange::TouchpadAccelSpeed(value) => settings.touchpad.pointer.accel_speed = value,
            InputChange::TouchpadAccelProfile(value) => {
                settings.touchpad.pointer.accel_profile = value
            }
            InputChange::TouchpadTap(value) => settings.touchpad.tap_to_click = value,
            InputChange::TouchpadDwt(value) => settings.touchpad.disable_while_typing = value,
            InputChange::TouchpadDragLock(value) => settings.touchpad.drag_lock = value,
        }
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_input::save(&settings) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_wifi_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.wifi_busy || self.wifi_loading || !self.wifi_available {
            return;
        }
        self.wifi_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_network::set_enabled(enabled)?;
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_wifi(&mut self, cx: &mut Context<Self>) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        self.wifi_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    rmac_network::request_scan()?;
                    std::thread::sleep(Duration::from_millis(750));
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_bluetooth_update(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
    ) {
        self.bluetooth_loading = false;
        self.bluetooth_busy = false;
        match result {
            Ok(snapshot) => {
                self.bluetooth_available = snapshot.available;
                self.bluetooth_on = snapshot.powered;
                self.bt_discoverable = snapshot.discoverable;
                self.bluetooth_discovering = snapshot.discovering;
                self.bluetooth_adapter_name = snapshot.adapter_name;
                self.bt_devices = snapshot.devices;
                self.bluetooth_error = None;
            }
            Err(error) => {
                self.bluetooth_error = Some(format!("Could not update Bluetooth: {error}").into());
            }
        }
    }

    fn set_bluetooth_powered(&mut self, powered: bool, cx: &mut Context<Self>) {
        if self.bluetooth_busy || self.bluetooth_loading || !self.bluetooth_available {
            return;
        }
        self.bluetooth_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_powered(powered)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_bluetooth_discoverable(&mut self, discoverable: bool, cx: &mut Context<Self>) {
        if self.bluetooth_busy || !self.bluetooth_available || !self.bluetooth_on {
            return;
        }
        self.bluetooth_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_discoverable(discoverable)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_bluetooth(&mut self, cx: &mut Context<Self>) {
        if self.bluetooth_busy || !self.bluetooth_available || !self.bluetooth_on {
            return;
        }
        self.bluetooth_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let initial = rmac_bluetooth::snapshot()?;
                    let started = !initial.discovering;
                    if started {
                        rmac_bluetooth::start_discovery()?;
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                    if started {
                        rmac_bluetooth::stop_discovery()?;
                    }
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_bluetooth_device_connected(
        &mut self,
        device_id: String,
        connected: bool,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy {
            return;
        }
        self.bluetooth_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_bluetooth::set_connected(&device_id, connected)?;
                    rmac_bluetooth::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_lock_policy_update(
        &mut self,
        result: std::result::Result<
            rmac_shortcuts::lock_settings::Snapshot,
            rmac_shortcuts::lock_settings::Error,
        >,
    ) {
        self.lock_policy_loading = false;
        self.lock_policy_busy = false;
        match result {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_error = None;
            }
            Err(error) => {
                self.lock_policy_error =
                    Some(format!("Could not update Lock Screen: {error}").into());
            }
        }
    }

    fn apply_lock_policy_stream_update(
        &mut self,
        update: std::result::Result<rmac_shortcuts::lock_settings::Snapshot, String>,
    ) {
        self.lock_policy_loading = false;
        match update {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_stream_error = None;
            }
            Err(error) => {
                self.lock_policy_stream_error =
                    Some(format!("Live Lock Screen updates unavailable: {error}").into());
            }
        }
    }

    fn refresh_lock_policy(&mut self, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_loading = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_shortcuts::lock_settings::settings() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_lock_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_lock_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_suspend_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_suspend_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Capture the current interactive state and write it to disk.
    fn persist(&mut self, cx: &App) {
        let snapshot = Persisted {
            wifi_on: self.wifi_on,
            ask_to_join: self.ask_to_join,
            joined: self.joined,
            bluetooth_on: self.bluetooth_on,
            bt_discoverable: self.bt_discoverable,
            output_volume: self.output_volume.read(cx).value().start(),
            alert_volume: self.alert_volume.read(cx).value().start(),
            balance: self.balance.read(cx).value().start(),
            mute: self.audio.output.muted,
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

    fn application_identity(&self, app_id: &str) -> Option<&rmac_apps::Application> {
        rmac_apps::find_desktop_entry(&self.app_catalog, app_id)
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
                el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    .cursor_pointer()
                    .on_click(cx.listener(|t, _, _, cx| t.go_back(cx)))
            })
            .child(glyph(
                "icons/chevron-left.svg",
                17.0,
                if can_back {
                    accent()
                } else {
                    rmac_ui::mac::text_tertiary()
                },
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
                    .child(glyph(
                        "icons/chevron-right.svg",
                        17.0,
                        rmac_ui::mac::text_tertiary(),
                    )),
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
            .bg(rmac_ui::mac::control_fill())
            .child(glyph("icons/search.svg", 13.0, secondary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
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
                    .bg(rmac_ui::mac::control_fill())
                    .child(glyph("icons/user.svg", 22.0, secondary())),
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
                            .child("Local Account"),
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
                    ListRow::new(
                        SharedString::from(format!("cat-{si}-{ci}")),
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .child(tile(cat.icon, cat.color, 20.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(if selected { on_accent() } else { label() })
                                    .child(cat.name.clone()),
                            ),
                    )
                    .selected(selected)
                    .mx_2()
                    .px_2()
                    .on_activate(cx.listener(move |t, _, _, cx| {
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
                "Notifications" => self.render_notifications(cx),
                "Focus" => self.render_focus(cx),
                "Lock Screen" => self.render_lock_screen(cx),
                "Sound" => self.render_sound(cx),
                "Keyboard" => self.render_keyboard(cx),
                "Mouse" => self.render_mouse(cx),
                "Trackpad" => self.render_trackpad(cx),
                "Battery" => self.render_battery(cx),
                "Displays" => self.render_displays(cx),
                "Network" => self.render_network(cx),
                "VPN" => self.render_vpn(cx),
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
                        el.child(
                            Progress::indeterminate()
                                .label("Loading system information…")
                                .mb_3(),
                        )
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
        let power_subtitle = if self.wifi_loading {
            Some("Reading system state…".into())
        } else if self.wifi_busy {
            Some("Applying change…".into())
        } else {
            self.wifi_interface
                .as_ref()
                .map(|interface| format!("NetworkManager · {interface}").into())
        };
        let power_view = view.clone();
        let power =
            Toggle::new("wifi-power")
                .checked(self.wifi_on)
                .on_click(move |enabled, _, cx| {
                    power_view.update(cx, |settings, cx| settings.set_wifi_enabled(*enabled, cx));
                });
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/wifi.svg", accent(), 22.0))
            .child(text_block("Wi-Fi".into(), power_subtitle))
            .child(power)
            .into_any_element()])];

        if self.wifi_loading {
            cards.push(note_card("Loading Wi-Fi state from the system…"));
            return self.pane(cards);
        }
        if !self.wifi_available {
            cards.push(note_card(
                "No Wi-Fi adapter is available through the system network service.",
            ));
            return self.pane(cards);
        }

        if self.wifi_on {
            let refresh_view = view.clone();
            let refresh_label = if self.wifi_busy {
                "Scanning…"
            } else {
                "Refresh"
            };
            cards.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(secondary())
                            .child("Networks"),
                    )
                    .child(
                        div()
                            .id("wifi-refresh")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .child(refresh_label)
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |settings, cx| settings.refresh_wifi(cx));
                            }),
                    ),
            );

            let rows = if self.wifi_networks.is_empty() {
                vec![EmptyState::new("No networks found")
                    .message("Refresh to scan again")
                    .into_any_element()]
            } else {
                self.wifi_networks
                    .iter()
                    .map(|network| {
                        let status = if network.connected {
                            format!("Connected · {}%", network.strength)
                        } else if network.secure {
                            format!("Secured · {}%", network.strength)
                        } else {
                            format!("Open · {}%", network.strength)
                        };
                        value_row(
                            "icons/wifi.svg",
                            if network.connected {
                                accent()
                            } else {
                                secondary()
                            },
                            network.ssid.clone().into(),
                            status.into(),
                        )
                    })
                    .collect()
            };
            cards.push(card(rows));
            cards.push(note_card(
                "Network discovery and Wi-Fi power are live. Joining a new protected network will be added with the NetworkManager secret-agent flow.",
            ));
        }

        self.pane(cards)
    }

    // ---- Bluetooth ----------------------------------------------------

    fn render_bluetooth(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let power_subtitle = if self.bluetooth_loading {
            Some("Reading system state…".into())
        } else if self.bluetooth_busy {
            Some("Applying change…".into())
        } else {
            self.bluetooth_adapter_name.clone().map(Into::into)
        };
        let power_view = view.clone();
        let power = Toggle::new("bluetooth-power")
            .checked(self.bluetooth_on)
            .on_click(move |powered, _, cx| {
                power_view.update(cx, |settings, cx| {
                    settings.set_bluetooth_powered(*powered, cx)
                });
            });
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/bluetooth.svg", accent(), 22.0))
            .child(text_block("Bluetooth".into(), power_subtitle))
            .child(power)
            .into_any_element()])];

        if self.bluetooth_loading {
            cards.push(note_card("Loading Bluetooth state from the system…"));
            return self.pane(cards);
        }
        if !self.bluetooth_available {
            cards.push(note_card(
                "No Bluetooth adapter is available through the system Bluetooth service.",
            ));
            return self.pane(cards);
        }

        if self.bluetooth_on {
            let discoverable_view = view.clone();
            let discoverable = Toggle::new("bluetooth-discoverable")
                .checked(self.bt_discoverable)
                .on_click(move |enabled, _, cx| {
                    discoverable_view.update(cx, |settings, cx| {
                        settings.set_bluetooth_discoverable(*enabled, cx)
                    });
                });
            cards.push(card(vec![row_base()
                .child(tile("icons/bluetooth.svg", secondary(), 22.0))
                .child(text_block(
                    "Discoverable".into(),
                    Some("Allow nearby devices to find this computer.".into()),
                ))
                .child(discoverable)
                .into_any_element()]));

            let refresh_view = view.clone();
            let refresh_label = if self.bluetooth_busy || self.bluetooth_discovering {
                "Scanning…"
            } else {
                "Refresh"
            };
            cards.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(secondary())
                            .child("Devices"),
                    )
                    .child(
                        div()
                            .id("bluetooth-refresh")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .child(refresh_label)
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_bluetooth(cx));
                            }),
                    ),
            );

            let connected: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.connected)
                .map(|device| bluetooth_device_row(&view, device))
                .collect();
            if !connected.is_empty() {
                cards.push(section_header("Connected"));
                cards.push(card(connected));
            }

            let known: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.paired && !device.connected)
                .map(|device| bluetooth_device_row(&view, device))
                .collect();
            if !known.is_empty() {
                cards.push(section_header("Known Devices"));
                cards.push(card(known));
            }

            let nearby: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| !device.paired && !device.connected)
                .map(|device| bluetooth_device_row(&view, device))
                .collect();
            if !nearby.is_empty() {
                cards.push(section_header("Nearby Devices"));
                cards.push(card(nearby));
            }
            if self.bt_devices.is_empty() {
                cards.push(note_card(
                    "No Bluetooth devices found. Refresh to scan again.",
                ));
            }
            cards.push(note_card(
                "Power, discovery, and known-device connections are live. Pairing a new device requires the confirmation-agent flow and is not enabled yet.",
            ));
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
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("rmac Appearance"),
            )
            .child(
                div()
                    .id("theme-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.theme_busy {
                        "Applying…"
                    } else {
                        "Refresh"
                    })
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_theme(cx));
                    }),
            )];
        if self.theme_loading {
            cards.push(note_card("Loading appearance preferences…"));
            return self.pane(cards);
        }
        let Some(theme) = &self.theme else {
            cards.push(note_card(
                "The rmac theme preference service is unavailable.",
            ));
            return self.pane(cards);
        };
        let enabled = !self.theme_busy;
        let preferences = &theme.preferences;
        let scheme_card = {
            let option = |id: &'static str,
                          name: &'static str,
                          preference: rmac_theme::SchemePreference,
                          swatch: Hsla| {
                let selected = preferences.color_scheme == preference;
                let option_view = view.clone();
                div()
                    .id(ElementId::from(id))
                    .v_flex()
                    .items_center()
                    .gap_1p5()
                    .when(enabled, |element| {
                        element.cursor_pointer().on_click(move |_, _, cx| {
                            option_view.update(cx, |settings, cx| {
                                settings.apply_theme_change(ThemeChange::Scheme(preference), cx)
                            });
                        })
                    })
                    .when(!enabled, |element| element.opacity(0.55))
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
                .child(option(
                    "theme-light",
                    "Light",
                    rmac_theme::SchemePreference::Light,
                    hsl(0xf5f5f7),
                ))
                .child(option(
                    "theme-dark",
                    "Dark",
                    rmac_theme::SchemePreference::Dark,
                    hsl(0x2c2c2e),
                ))
                .child(option(
                    "theme-auto",
                    "Automatic",
                    rmac_theme::SchemePreference::Automatic,
                    hsl(0x8e8e93),
                ))
        };
        cards.push(scheme_card);

        let mut swatches = Vec::new();
        let automatic_selected =
            preferences.accent_color == rmac_theme::AccentPreference::Automatic;
        let auto_view = view.clone();
        swatches.push(
            div()
                .id("theme-accent-auto")
                .h(px(24.0))
                .px_2()
                .rounded(px(6.0))
                .flex()
                .items_center()
                .text_size(px(11.0))
                .text_color(if automatic_selected {
                    on_accent()
                } else {
                    label()
                })
                .bg(if automatic_selected {
                    accent()
                } else {
                    rmac_ui::mac::control_fill()
                })
                .when(enabled, |element| {
                    element.cursor_pointer().on_click(move |_, _, cx| {
                        auto_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(
                                ThemeChange::Accent(rmac_theme::AccentPreference::Automatic),
                                cx,
                            )
                        });
                    })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child("Automatic")
                .into_any_element(),
        );
        for (index, (name, hex)) in ACCENTS.iter().copied().enumerate() {
            let preference = accent_preference(hex);
            let selected = preferences.accent_color == preference;
            let swatch_foreground = swatch_foreground(hex);
            let swatch_view = view.clone();
            swatches.push(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "theme-accent-{index}"
                    ))))
                    .w(px(48.0))
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .when(enabled, |element| {
                        element.cursor_pointer().on_click(move |_, _, cx| {
                            swatch_view.update(cx, |settings, cx| {
                                settings.apply_theme_change(ThemeChange::Accent(preference), cx)
                            });
                        })
                    })
                    .when(!enabled, |element| element.opacity(0.55))
                    .child(
                        div()
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded_full()
                            .bg(hsl(hex))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |element| {
                                element
                                    .border_2()
                                    .border_color(swatch_foreground)
                                    .shadow_sm()
                                    .child(glyph("icons/check.svg", 12.0, swatch_foreground))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(secondary())
                            .child(name),
                    )
                    .into_any_element(),
            );
        }
        cards.push(
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
                ),
        );
        cards.push(card(vec![
            theme_segment_row(
                view.clone(),
                "theme-contrast",
                "Contrast",
                &THEME_CONTRAST_OPTIONS,
                match preferences.contrast {
                    rmac_theme::ContrastPreference::Automatic => 0,
                    rmac_theme::ContrastPreference::Normal => 1,
                    rmac_theme::ContrastPreference::Higher => 2,
                },
                enabled,
            ),
            theme_segment_row(
                view,
                "theme-motion",
                "Motion",
                &THEME_MOTION_OPTIONS,
                match preferences.motion {
                    rmac_theme::MotionPreferenceSetting::Automatic => 0,
                    rmac_theme::MotionPreferenceSetting::Full => 1,
                    rmac_theme::MotionPreferenceSetting::Reduced => 2,
                },
                enabled,
            ),
        ]));

        let host_scheme = if self.host_appearance.capabilities.color_scheme {
            self.host_appearance.color_scheme.label()
        } else {
            "Not exposed"
        };
        cards.push(section_header("Authority"));
        cards.push(card(vec![
            value_row(
                "icons/info.svg",
                secondary(),
                "Host preference".into(),
                host_scheme.into(),
            ),
            value_row(
                "icons/palette.svg",
                accent(),
                "Effective appearance".into(),
                match theme.effective.color_scheme {
                    rmac_appearance::ResolvedColorScheme::Light => "Light".into(),
                    rmac_appearance::ResolvedColorScheme::Dark => "Dark".into(),
                },
            ),
        ]));
        if let Some(detail) = theme.detail.clone() {
            cards.push(note_card(detail));
        }
        if !self.host_appearance.available {
            cards.push(note_card(
                "The Linux Settings portal is unavailable here. Automatic values use safe rmac defaults; explicit choices remain writable.",
            ));
        }
        self.pane(cards)
    }

    // ---- Notifications ------------------------------------------------

    fn render_notifications(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        let refresh_view = view.clone();
        cards.push(
            div().flex().justify_end().mb_2().child(
                div()
                    .id("notifications-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .when(!self.notifications_loading, |button| {
                        button
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_notifications(cx));
                            })
                    })
                    .child(if self.notifications_loading {
                        "Loading…"
                    } else {
                        "Refresh"
                    }),
            ),
        );
        if self.notifications_loading {
            cards.push(
                div()
                    .mb_3()
                    .child(Progress::indeterminate().label("Loading notification settings…")),
            );
        }
        if let Some(error) = &self.notification_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.notification_stream_error {
            cards.push(note_card(error.clone()));
        }
        if !self.notifications_loading {
            let rows = if self.notification_apps.is_empty() {
                vec![EmptyState::new("No applications yet")
                    .message("Applications appear after they send a notification")
                    .into_any_element()]
            } else {
                self.notification_apps
                    .iter()
                    .map(|application| {
                        let value = if application.policy.enabled {
                            "On"
                        } else {
                            "Off"
                        };
                        let identity = self.application_identity(&application.app_id);
                        application_nav_row(
                            &view,
                            &application.app_id,
                            identity
                                .map(|identity| identity.name.as_str())
                                .unwrap_or(&application.app_id),
                            identity.and_then(|identity| identity.icon.as_ref()),
                            value,
                            application.policy.enabled,
                            SubPage::NotificationApp {
                                app_id: application.app_id.clone(),
                            },
                        )
                    })
                    .collect()
            };
            cards.push(card(rows));
        }
        self.pane(cards)
    }

    fn notification_app_body(&self, app_id: &str, cx: &Context<Self>) -> Div {
        let Some(application) = self
            .notification_apps
            .iter()
            .find(|application| application.app_id == app_id)
        else {
            return note_card("This application is no longer in Notification Center.");
        };
        let view = cx.entity();
        let policy = application.policy;
        let busy = self.notification_busy.as_deref() == Some(app_id);
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying notification policy…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.notification_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.notification_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body.child(card(vec![
            notification_toggle_row(
                &view,
                app_id,
                "enabled",
                "Allow notifications",
                Some("Blocks banners, sounds, badges, and history when off"),
                policy.enabled,
                busy,
                NotificationPolicyChange::Enabled,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "badges",
                "Badge indicator",
                Some("Count unread notifications in the top bar"),
                policy.badges,
                busy || !policy.enabled,
                NotificationPolicyChange::Badges,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "history",
                "Notification Center history",
                Some("Turning this off immediately removes saved history"),
                policy.history,
                busy || !policy.enabled,
                NotificationPolicyChange::History,
            ),
        ]))
        .child(note_card(
            "Banner, sound, Focus-bypass, and lock-screen controls stay hidden until their presentation and secure-lock adapters are active.",
        ))
    }

    // ---- Focus --------------------------------------------------------

    fn render_focus(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        let refresh_view = view.clone();
        cards.push(
            div().flex().justify_end().mb_2().child(
                div()
                    .id("focus-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .when(!self.focus_policy_loading, |button| {
                        button
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |settings, cx| settings.refresh_focus(cx));
                            })
                    })
                    .child(if self.focus_policy_loading {
                        "Loading…"
                    } else {
                        "Refresh"
                    }),
            ),
        );
        if self.focus_policy_loading || self.focus_policy_busy {
            cards.push(div().mb_3().child(Progress::indeterminate().label(
                if self.focus_policy_busy {
                    "Applying Focus change…"
                } else {
                    "Loading Focus…"
                },
            )));
        }
        if let Some(error) = &self.focus_policy_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(state) = &self.focus_policy_state {
            let active = state.projection.enabled;
            let status = state
                .projection
                .mode_name
                .clone()
                .unwrap_or_else(|| "Off".into());
            let turn_off_view = view.clone();
            cards.push(card(vec![row_base()
                .child(tile(
                    "icons/moon.svg",
                    if active { accent() } else { secondary() },
                    22.0,
                ))
                .child(text_block("Current Focus".into(), Some(status.into())))
                .when(active, |row| {
                    row.child(
                        div()
                            .id("focus-turn-off")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                turn_off_view.update(cx, |settings, cx| settings.disable_focus(cx));
                            })
                            .child("Turn Off"),
                    )
                })
                .into_any_element()]));
        }
        if let Some(configuration) = &self.focus_policy_config {
            let active_mode = self
                .focus_policy_state
                .as_ref()
                .and_then(|state| state.mode_id.as_deref());
            let mode_rows = configuration
                .modes()
                .map(|mode| {
                    nav_row(
                        view.clone(),
                        "icons/moon.svg",
                        if active_mode == Some(mode.id().as_str()) {
                            accent()
                        } else {
                            secondary()
                        },
                        mode.name().to_owned().into(),
                        (active_mode == Some(mode.id().as_str())).then(|| "Active".into()),
                        SubPage::FocusMode {
                            mode_id: mode.id().as_str().to_owned(),
                        },
                    )
                })
                .collect();
            cards.push(card(mode_rows));

            let schedules = configuration.schedules().collect::<Vec<_>>();
            if schedules.is_empty() {
                cards.push(note_card(
                    "No Focus schedules are configured. Schedule creation stays hidden until its complete day-and-time editor is available.",
                ));
            } else {
                cards.push(
                    div()
                        .text_size(px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .mt_3()
                        .mb_1()
                        .px_1()
                        .child("Schedules"),
                );
                cards.push(card(
                    schedules
                        .into_iter()
                        .map(|schedule| {
                            let mode_name = configuration
                                .mode(&schedule.mode)
                                .map(rmac_focus::Mode::name)
                                .unwrap_or("Unknown Focus");
                            focus_schedule_row(&view, schedule, mode_name, self.focus_policy_busy)
                        })
                        .collect(),
                ));
            }
        }
        self.pane(cards)
    }

    fn focus_mode_body(&self, mode_id: &str, cx: &Context<Self>) -> Div {
        let Some(configuration) = &self.focus_policy_config else {
            return note_card("Focus configuration is unavailable.");
        };
        let Ok(parsed_mode) = rmac_focus::ModeId::parse(mode_id) else {
            return note_card("This Focus mode is invalid.");
        };
        let Some(mode) = configuration.mode(&parsed_mode) else {
            return note_card("This Focus mode no longer exists.");
        };
        let view = cx.entity();
        let busy = self.focus_policy_busy;
        let activate_forever_view = view.clone();
        let activate_hour_view = view.clone();
        let mode_forever = mode_id.to_owned();
        let mode_hour = mode_id.to_owned();
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying Focus change…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.focus_policy_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body = body.child(card(vec![
            ListRow::new(
                SharedString::from(format!("focus-activate-{mode_id}")),
                div().child("Turn On"),
            )
            .disabled(busy)
            .on_activate(move |_, _, cx| {
                activate_forever_view.update(cx, |settings, cx| {
                    settings.activate_focus(mode_forever.clone(), 0, cx)
                });
            })
            .into_any_element(),
            ListRow::new(
                SharedString::from(format!("focus-hour-{mode_id}")),
                div().child("Turn On for 1 Hour"),
            )
            .disabled(busy)
            .on_activate(move |_, _, cx| {
                activate_hour_view.update(cx, |settings, cx| {
                    settings.activate_focus(mode_hour.clone(), 60 * 60 * 1_000, cx)
                });
            })
            .into_any_element(),
            focus_urgent_row(&view, mode_id, mode.allow_urgent(), busy),
        ]));

        body = body.child(
            div()
                .text_size(px(12.0))
                .font_weight(rmac_ui::mac::SEMIBOLD)
                .text_color(secondary())
                .mt_3()
                .mb_1()
                .px_1()
                .child("Allowed Applications"),
        );
        body = if self.notification_apps.is_empty() {
            body.child(note_card(
                "Applications appear here after they send a notification.",
            ))
        } else {
            body.child(card(
                self.notification_apps
                    .iter()
                    .filter_map(|application| {
                        let app_id = rmac_notifications::AppId::parse(&application.app_id).ok()?;
                        let identity = self.application_identity(&application.app_id);
                        Some(focus_allowed_app_row(
                            &view,
                            mode_id,
                            &application.app_id,
                            identity
                                .map(|identity| identity.name.as_str())
                                .unwrap_or(&application.app_id),
                            identity.and_then(|identity| identity.icon.as_ref()),
                            mode.allowed_apps().contains(&app_id),
                            busy,
                        ))
                    })
                    .collect(),
            ))
        };

        body = body.child(
            div()
                .text_size(px(12.0))
                .font_weight(rmac_ui::mac::SEMIBOLD)
                .text_color(secondary())
                .mt_3()
                .mb_1()
                .px_1()
                .child("Schedules"),
        );
        let schedules = configuration
            .schedules()
            .filter(|schedule| schedule.mode == parsed_mode)
            .collect::<Vec<_>>();
        if !schedules.is_empty() {
            body = body.child(card(
                schedules
                    .into_iter()
                    .map(|schedule| focus_schedule_row(&view, schedule, mode.name(), busy))
                    .collect(),
            ));
        }
        let add_view = view.clone();
        let add_mode_id = mode_id.to_owned();
        body.child(card(vec![ListRow::new(
            SharedString::from(format!("focus-add-schedule-{mode_id}")),
            div().text_color(accent()).child("Add Schedule…"),
        )
        .disabled(busy)
        .on_activate(move |_, _, cx| {
            add_view.update(cx, |settings, cx| {
                settings.add_focus_schedule(add_mode_id.clone(), cx);
            });
        })
        .into_any_element()]))
    }

    fn focus_schedule_body(&self, schedule_id: &str, cx: &Context<Self>) -> Div {
        let Some(configuration) = &self.focus_policy_config else {
            return note_card("Focus configuration is unavailable.");
        };
        let Ok(parsed_schedule) = rmac_focus::ScheduleId::parse(schedule_id) else {
            return note_card("This Focus schedule is invalid.");
        };
        let Some(schedule) = configuration
            .schedules()
            .find(|schedule| schedule.id == parsed_schedule)
        else {
            return note_card("This Focus schedule no longer exists.");
        };
        let mode_name = configuration
            .mode(&schedule.mode)
            .map(rmac_focus::Mode::name)
            .unwrap_or("Unknown Focus");
        let view = cx.entity();
        let busy = self.focus_policy_busy;
        let mut body = div().v_flex();
        if busy {
            body = body.child(
                Progress::indeterminate()
                    .label("Applying Focus schedule…")
                    .mb_3(),
            );
        }
        if let Some(error) = &self.focus_policy_error {
            body = body.child(note_card(error.clone()));
        }
        if let Some(error) = &self.focus_policy_stream_error {
            body = body.child(note_card(error.clone()));
        }
        body = body.child(card(vec![focus_schedule_toggle_row(
            &view, schedule, mode_name, busy,
        )]));
        body = body.child(section_header("Repeat"));
        body = body.child(
            div()
                .flex()
                .gap_1()
                .mb_3()
                .children(FOCUS_DAYS.into_iter().map(|(day, label)| {
                    focus_day_button(
                        &view,
                        schedule_id,
                        day,
                        label,
                        schedule.days.contains(&day),
                        busy,
                    )
                })),
        );
        body = body.child(section_header("Time"));
        body = body.child(card(vec![
            focus_time_row(
                &view,
                schedule_id,
                "From",
                true,
                schedule.start_minute,
                busy,
            ),
            focus_time_row(&view, schedule_id, "To", false, schedule.end_minute, busy),
        ]));
        body = body.child(note_card(
            "Times use this computer’s local time. A finish time before the start time continues into the next day.",
        ));
        let remove_view = view.clone();
        let remove_schedule_id = schedule_id.to_owned();
        body.child(card(vec![ListRow::new(
            SharedString::from(format!("focus-remove-schedule-{schedule_id}")),
            div()
                .text_color(rmac_ui::mac::danger())
                .child("Delete Schedule"),
        )
        .disabled(busy)
        .on_activate(move |_, _, cx| {
            remove_view.update(cx, |settings, cx| {
                settings.remove_focus_schedule(remove_schedule_id.clone(), cx);
            });
        })
        .into_any_element()]))
    }

    // ---- Lock Screen --------------------------------------------------

    fn render_lock_screen(&self, cx: &Context<Self>) -> Div {
        const TIMEOUTS: [(Option<u32>, &str, &str); 5] = [
            (
                None,
                "Never",
                "Keep the session unlocked until a manual or system lock",
            ),
            (
                Some(60),
                "After 1 Minute",
                "Lock after one minute without input",
            ),
            (Some(300), "After 5 Minutes", "Recommended default"),
            (
                Some(900),
                "After 15 Minutes",
                "Lock after fifteen minutes without input",
            ),
            (
                Some(3_600),
                "After 1 Hour",
                "Lock after one hour without input",
            ),
        ];

        let view = cx.entity();
        let refresh_view = view.clone();
        let mut cards = vec![div().flex().justify_end().mb_2().child(
            div()
                .id("lock-policy-refresh")
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(accent())
                .when(
                    !self.lock_policy_loading && !self.lock_policy_busy,
                    |button| {
                        button
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_lock_policy(cx));
                            })
                    },
                )
                .child(if self.lock_policy_loading {
                    "Loading…"
                } else {
                    "Refresh"
                }),
        )];
        if self.lock_policy_loading || self.lock_policy_busy {
            cards.push(div().mb_3().child(Progress::indeterminate().label(
                if self.lock_policy_busy {
                    "Applying Lock Screen timeout…"
                } else {
                    "Loading Lock Screen…"
                },
            )));
        }
        if let Some(error) = &self.lock_policy_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.lock_policy_stream_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(policy) = self.lock_policy {
            cards.push(section_header("Lock After Inactivity"));
            let mut timeout_rows = TIMEOUTS
                .into_iter()
                .map(|(timeout, title, detail)| {
                    let selected = policy.lock_after_seconds == timeout;
                    let option_view = view.clone();
                    row_base()
                        .id(ElementId::from(SharedString::from(format!(
                            "lock-timeout-{}",
                            timeout.unwrap_or(0)
                        ))))
                        .child(tile("icons/lock.svg", secondary(), 22.0))
                        .child(text_block(title.into(), Some(detail.into())))
                        .when(selected, |row| {
                            row.child(glyph("icons/check.svg", 14.0, accent()))
                        })
                        .when(!selected && !self.lock_policy_busy, |row| {
                            row.cursor_pointer()
                                .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                .on_click(move |_, _, cx| {
                                    option_view.update(cx, |settings, cx| {
                                        settings.set_lock_after(timeout, cx)
                                    });
                                })
                        })
                        .when(self.lock_policy_busy, |row| row.opacity(0.55))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            if !TIMEOUTS
                .iter()
                .any(|(timeout, _, _)| *timeout == policy.lock_after_seconds)
            {
                let value = policy
                    .lock_after_seconds
                    .map(u64::from)
                    .map(format_power_duration)
                    .unwrap_or_else(|| "Never".into());
                timeout_rows.insert(
                    0,
                    value_row(
                        "icons/history.svg",
                        secondary(),
                        "Current Custom Timeout".into(),
                        value.into(),
                    ),
                );
            }
            cards.push(card(timeout_rows));
            cards.push(section_header("Automatic Suspend"));
            let suspend_authorized = policy.suspend_capability
                == rmac_shortcuts::lock_settings::SuspendCapability::Authorized;
            let suspend_options = [
                (None, "Never", "Do not suspend automatically"),
                (
                    Some(15 * 60),
                    "After 15 Minutes",
                    "Suspend after fifteen minutes without input",
                ),
                (
                    Some(30 * 60),
                    "After 30 Minutes",
                    "Suspend after thirty minutes without input",
                ),
                (
                    Some(60 * 60),
                    "After 1 Hour",
                    "Suspend after one hour without input",
                ),
                (
                    Some(3 * 60 * 60),
                    "After 3 Hours",
                    "Suspend after three hours without input",
                ),
            ];
            let mut suspend_rows = suspend_options
                .into_iter()
                .filter(|(timeout, _, _)| timeout.is_none() || suspend_authorized)
                .map(|(timeout, title, detail)| {
                    let selected = policy.suspend_after_seconds == timeout;
                    let option_view = view.clone();
                    row_base()
                        .id(ElementId::from(SharedString::from(format!(
                            "suspend-timeout-{}",
                            timeout.unwrap_or(0)
                        ))))
                        .child(tile("icons/power.svg", secondary(), 22.0))
                        .child(text_block(title.into(), Some(detail.into())))
                        .when(selected, |row| {
                            row.child(glyph("icons/check.svg", 14.0, accent()))
                        })
                        .when(!selected && !self.lock_policy_busy, |row| {
                            row.cursor_pointer()
                                .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                .on_click(move |_, _, cx| {
                                    option_view.update(cx, |settings, cx| {
                                        settings.set_suspend_after(timeout, cx)
                                    });
                                })
                        })
                        .when(self.lock_policy_busy, |row| row.opacity(0.55))
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            if let Some(seconds) = policy.suspend_after_seconds {
                let is_visible_choice = suspend_authorized
                    && suspend_options
                        .iter()
                        .any(|(timeout, _, _)| *timeout == Some(seconds));
                if !is_visible_choice {
                    suspend_rows.insert(
                        0,
                        value_row(
                            "icons/history.svg",
                            secondary(),
                            "Current Suspend Timeout".into(),
                            format_power_duration(u64::from(seconds)).into(),
                        ),
                    );
                }
            }
            cards.push(card(suspend_rows));
            match policy.suspend_capability {
                rmac_shortcuts::lock_settings::SuspendCapability::Authorized => {
                    cards.push(note_card(
                        "Automatic suspend uses the system login manager, respects active inhibitors, and always passes through the pre-sleep lock boundary.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::RequiresAuthentication => {
                    cards.push(note_card(
                        "Automatic suspend is unavailable because this computer requires interactive authorization. You can still suspend manually and approve the system prompt.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::Denied => {
                    cards.push(note_card(
                        "Automatic suspend is disabled by this computer’s authorization policy.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::Unavailable => {
                    cards.push(note_card(
                        "Automatic suspend is not supported by this computer or its current system service.",
                    ));
                }
            }
            cards.push(section_header("Security"));
            cards.push(card(vec![
                value_row(
                    "icons/shield.svg",
                    hsl(0x34c759),
                    "Before Sleep".into(),
                    "Always Lock".into(),
                ),
                value_row(
                    "icons/key.svg",
                    secondary(),
                    "Authentication".into(),
                    "Password Required".into(),
                ),
            ]));
            cards.push(note_card(
                "Notification previews and login presentation controls remain hidden until their secure adapters are available. Lid close and suspend follow the system’s supported logind policy and always pass through the pre-sleep lock boundary.",
            ));
        }
        self.pane(cards)
    }

    // ---- Sound --------------------------------------------------------

    fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let out = self.output_volume.read(cx).value().start().round() as i32;
        let input = self.input_volume.read(cx).value().start().round() as i32;
        let alert = self.alert_volume.read(cx).value().start().round() as i32;
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("System Audio"),
            )
            .child(
                div()
                    .id("audio-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.audio_busy {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    })
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_audio(cx));
                    }),
            )];

        if self.audio_loading {
            cards.push(note_card("Loading audio state from the system…"));
        } else if !self.audio.available {
            cards.push(note_card(
                "The system audio service is not available on this computer.",
            ));
        } else {
            let output_view = view.clone();
            let output_mute = Toggle::new("audio-output-mute")
                .checked(self.audio.output.muted)
                .on_click(move |muted, _, cx| {
                    output_view.update(cx, |settings, cx| {
                        settings.set_audio_muted(rmac_audio::DeviceKind::Output, *muted, cx)
                    });
                });
            cards.push(
                div()
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
                    .child(
                        row_base()
                            .child(tile("icons/volume-2.svg", secondary(), 22.0))
                            .child(text_block("Mute output".into(), None))
                            .child(output_mute),
                    ),
            );

            let mut input_card = div()
                .v_flex()
                .mb_3()
                .rounded(px(10.0))
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(slider_row(
                    "Input volume",
                    &self.input_volume,
                    format!("{input}%").into(),
                ));
            if self.audio.can_mute_input {
                let input_view = view.clone();
                let input_mute = Toggle::new("audio-input-mute")
                    .checked(self.audio.input.muted)
                    .on_click(move |muted, _, cx| {
                        input_view.update(cx, |settings, cx| {
                            settings.set_audio_muted(rmac_audio::DeviceKind::Input, *muted, cx)
                        });
                    });
                input_card = input_card.child(div().h(px(1.0)).bg(sep()).mx_3()).child(
                    row_base()
                        .child(tile("icons/volume-2.svg", secondary(), 22.0))
                        .child(text_block("Mute microphone".into(), None))
                        .child(input_mute),
                );
            }
            cards.push(input_card);
            cards.push(self.audio_device_card(
                "Output Device",
                &self.audio.outputs,
                rmac_audio::DeviceKind::Output,
                cx,
            ));
            cards.push(self.audio_device_card(
                "Input Device",
                &self.audio.inputs,
                rmac_audio::DeviceKind::Input,
                cx,
            ));
        }

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
        cards.push(section_header("rmac Sounds"));
        cards.push(alert_card);
        cards.push(toggles);
        cards.push(note_card(
            "Output, microphone, and default devices use the system audio service. Alert sounds and interface effects belong to the rmac desktop session.",
        ));
        self.pane(cards)
    }

    fn audio_device_card(
        &self,
        title: &'static str,
        devices: &[rmac_audio::Device],
        kind: rmac_audio::DeviceKind,
        cx: &Context<Self>,
    ) -> Div {
        if devices.is_empty() {
            return div();
        }
        let view = cx.entity();
        let rows: Vec<AnyElement> = devices
            .iter()
            .map(|d| {
                let id = d.id.clone();
                let device_view = view.clone();
                row_base()
                    .id(ElementId::from(SharedString::from(format!(
                        "audio-device-{title}-{id}"
                    ))))
                    .child(tile("icons/volume-2.svg", accent(), 22.0))
                    .child(text_block(d.name.clone().into(), None))
                    .when(d.is_default, |el| {
                        el.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(secondary())
                                .child("Default"),
                        )
                        .child(glyph("icons/check.svg", 13.0, accent()))
                    })
                    .when(self.audio.can_set_default && !d.is_default, |el| {
                        el.cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                device_view.update(cx, |settings, cx| {
                                    settings.set_default_audio_device(kind, id, cx)
                                });
                            })
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .v_flex()
            .child(section_header(title))
            .child(card(rows))
    }

    // ---- Keyboard, mouse, and trackpad -------------------------------

    fn input_header(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("niri · libinput"),
            )
            .child(
                div()
                    .id("input-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.input_busy {
                        "Applying…"
                    } else {
                        "Refresh"
                    })
                    .on_click(move |_, _, cx| {
                        view.update(cx, |settings, cx| settings.refresh_input(cx));
                    }),
            )
    }

    fn input_unavailable_card(&self) -> Option<Div> {
        if self.input_loading {
            return Some(note_card("Loading input settings from niri…"));
        }
        if !self.input.available || !self.input.can_configure {
            return Some(note_card(self.input.detail.clone().unwrap_or_else(|| {
                "Input configuration is unavailable in this desktop session.".into()
            })));
        }
        None
    }

    fn render_keyboard(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.keyboard;
        cards.push(section_header("Key Repeat"));
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-delay",
                "Delay until repeat",
                &KEYBOARD_DELAYS,
                KEYBOARD_DELAYS
                    .iter()
                    .position(|(_, change)| matches!(change, InputChange::KeyboardRepeatDelay(value) if *value == settings.repeat_delay_ms))
                    .unwrap_or(2),
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-rate",
                "Key repeat rate",
                &KEYBOARD_RATES,
                KEYBOARD_RATES
                    .iter()
                    .position(|(_, change)| matches!(change, InputChange::KeyboardRepeatRate(value) if *value == settings.repeat_rate))
                    .unwrap_or(2),
                self.input.can_configure && !self.input_busy,
            ),
            input_switch_row(
                cx.entity(),
                "keyboard-numlock",
                "icons/keyboard.svg",
                "Use Num Lock on startup",
                None,
                settings.numlock,
                self.input.can_configure && !self.input_busy,
                InputChange::KeyboardNumlock,
            ),
        ]));
        cards.push(note_card(
            "Changes are validated, saved to the niri configuration, and applied by niri's live reload.",
        ));
        self.pane(cards)
    }

    fn render_mouse(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.mouse;
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "mouse-tracking",
                "Tracking speed",
                &MOUSE_SPEEDS,
                speed_index(settings.accel_speed),
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "mouse-acceleration",
                "Acceleration",
                &MOUSE_PROFILES,
                usize::from(settings.accel_profile == rmac_input::AccelProfile::Flat),
                self.input.can_configure && !self.input_busy,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-natural-scroll",
                "icons/mouse.svg",
                "Natural scrolling",
                Some("Move content in the direction your finger travels"),
                settings.natural_scroll,
                self.input.can_configure && !self.input_busy,
                InputChange::MouseNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-left-handed",
                "icons/mouse.svg",
                "Primary button on right",
                Some("Swap the left and right mouse buttons"),
                settings.left_handed,
                self.input.can_configure && !self.input_busy,
                InputChange::MouseLeftHanded,
            ),
        ]));
        self.pane(cards)
    }

    fn render_trackpad(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.touchpad;
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "touchpad-tracking",
                "Tracking speed",
                &TOUCHPAD_SPEEDS,
                speed_index(settings.pointer.accel_speed),
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "touchpad-acceleration",
                "Acceleration",
                &TOUCHPAD_PROFILES,
                usize::from(settings.pointer.accel_profile == rmac_input::AccelProfile::Flat),
                self.input.can_configure && !self.input_busy,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-tap",
                "icons/touchpad.svg",
                "Tap to click",
                None,
                settings.tap_to_click,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadTap,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-natural-scroll",
                "icons/touchpad.svg",
                "Natural scrolling",
                Some("Move content in the direction your fingers travel"),
                settings.pointer.natural_scroll,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-dwt",
                "icons/keyboard.svg",
                "Ignore while typing",
                Some("Prevent accidental pointer movement while typing"),
                settings.disable_while_typing,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadDwt,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-drag-lock",
                "icons/touchpad.svg",
                "Drag lock",
                Some("Keep dragging briefly after lifting your finger"),
                settings.drag_lock,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadDragLock,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-left-handed",
                "icons/touchpad.svg",
                "Primary click on right",
                None,
                settings.pointer.left_handed,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadLeftHanded,
            ),
        ]));
        self.pane(cards)
    }

    // ---- Battery and power profiles ----------------------------------

    fn render_battery(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("Battery & Energy"),
            )
            .child(
                div()
                    .id("power-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.power_busy {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    })
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_power(cx));
                    }),
            )];
        if self.power_loading {
            cards.push(note_card("Loading battery state from the system…"));
            return self.pane(cards);
        }

        if let Some(battery) = &self.power.battery {
            let mut status_rows = vec![
                value_row(
                    "icons/battery-charging.svg",
                    hsl(0x34c759),
                    "Charge".into(),
                    format!("{}%", battery.percentage).into(),
                ),
                value_row(
                    "icons/info.svg",
                    secondary(),
                    "Status".into(),
                    battery.state.label().into(),
                ),
                value_row(
                    "icons/power.svg",
                    secondary(),
                    "Power Source".into(),
                    if battery.on_battery {
                        "Battery".into()
                    } else {
                        "Power Adapter".into()
                    },
                ),
            ];
            if let Some(seconds) = battery.seconds_remaining {
                status_rows.push(value_row(
                    "icons/clock.svg",
                    secondary(),
                    if matches!(
                        battery.state,
                        rmac_power::BatteryState::Charging
                            | rmac_power::BatteryState::PendingCharge
                    ) {
                        "Time to Full".into()
                    } else {
                        "Time Remaining".into()
                    },
                    format_power_duration(seconds).into(),
                ));
            }
            if let Some(rate) = battery.energy_rate_watts {
                status_rows.push(value_row(
                    "icons/power.svg",
                    secondary(),
                    "Energy Rate".into(),
                    format!("{rate:.1} W").into(),
                ));
            }
            if let Some(model) = &battery.model {
                status_rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Battery".into(),
                    model.clone().into(),
                ));
            }
            cards.push(card(status_rows));

            let condition = battery.capacity.map_or("Unknown", |capacity| {
                if capacity < 75 {
                    "Service Recommended"
                } else {
                    "Normal"
                }
            });
            let mut health_rows = vec![value_row(
                "icons/heart-handshake.svg",
                hsl(0x34c759),
                "Condition".into(),
                condition.into(),
            )];
            if let Some(capacity) = battery.capacity {
                health_rows.insert(
                    0,
                    value_row(
                        "icons/battery-charging.svg",
                        hsl(0x34c759),
                        "Maximum Capacity".into(),
                        format!("{capacity}%").into(),
                    ),
                );
            }
            if let Some(cycles) = battery.charge_cycles {
                health_rows.push(value_row(
                    "icons/history.svg",
                    secondary(),
                    "Cycle Count".into(),
                    cycles.to_string().into(),
                ));
            }
            cards.push(section_header("Battery Health"));
            cards.push(card(health_rows));
        } else {
            cards.push(note_card(
                "No system battery was detected. This computer is using external power.",
            ));
        }

        if self.power.profiles.available && !self.power.profiles.supported.is_empty() {
            cards.push(section_header("Energy Mode"));
            let rows = self
                .power
                .profiles
                .supported
                .iter()
                .map(|profile| {
                    let profile = *profile;
                    let selected = self.power.profiles.active == Some(profile);
                    let profile_view = view.clone();
                    row_base()
                        .id(ElementId::from(SharedString::from(format!(
                            "power-profile-{}",
                            profile.id()
                        ))))
                        .child(tile("icons/power.svg", accent(), 22.0))
                        .child(text_block(profile.label().into(), None))
                        .when(selected, |row| {
                            row.child(glyph("icons/check.svg", 14.0, accent()))
                        })
                        .when(!selected, |row| {
                            row.cursor_pointer()
                                .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                .on_click(move |_, _, cx| {
                                    profile_view.update(cx, |settings, cx| {
                                        settings.set_power_profile(profile, cx)
                                    });
                                })
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
            if let Some(reason) = &self.power.profiles.performance_degraded {
                cards.push(card(vec![value_row(
                    "icons/info.svg",
                    hsl(0xff9500),
                    "High Power Limited".into(),
                    power_degradation_label(reason).into(),
                )]));
            }
        } else {
            cards.push(note_card(
                "Power profile selection is unavailable on this computer.",
            ));
        }
        self.pane(cards)
    }

    // ---- Displays -----------------------------------------------------

    fn render_displays(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(if self.display.compositor.is_empty() {
                        "Displays".to_string()
                    } else {
                        format!("Displays · {}", self.display.compositor)
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.display_revert.is_some(), |actions| {
                        actions.child(
                            div()
                                .id("display-revert")
                                .px_2()
                                .py_1()
                                .rounded(px(6.0))
                                .text_size(px(12.0))
                                .text_color(hsl(0xff3b30))
                                .cursor_pointer()
                                .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                .child("Revert")
                                .on_click(move |_, _, cx| {
                                    revert_view.update(cx, |settings, cx| {
                                        settings.revert_display_change(cx)
                                    });
                                }),
                        )
                    })
                    .child(
                        div()
                            .id("display-refresh")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .child(if self.display_busy {
                                "Applying…"
                            } else {
                                "Refresh"
                            })
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_displays(cx));
                            }),
                    ),
            )];
        if self.display_loading {
            cards.push(note_card("Loading displays from the compositor…"));
            return self.pane(cards);
        }
        if !self.display.available {
            cards.push(note_card(
                "The display service is not available in this desktop session.",
            ));
            return self.pane(cards);
        }
        if self.display.outputs.is_empty() {
            cards.push(note_card("No displays were detected."));
        }

        for output in &self.display.outputs {
            let title = if output.primary {
                format!("{} · Main", output.name)
            } else {
                output.name.clone()
            };
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
            let mut rows = Vec::new();
            if let Some(detail) = &output.detail {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Type".into(),
                    detail.clone().into(),
                ));
            }
            rows.push(value_row(
                "icons/monitor.svg",
                accent(),
                "Connector".into(),
                output.connector.clone().into(),
            ));
            if let Some(mode) = output.current_mode() {
                rows.push(value_row(
                    "icons/monitor.svg",
                    secondary(),
                    "Resolution".into(),
                    mode.label().into(),
                ));
            } else {
                rows.push(value_row(
                    "icons/monitor.svg",
                    secondary(),
                    "Status".into(),
                    "Disabled".into(),
                ));
            }
            if let Some(logical) = &output.logical {
                rows.push(value_row(
                    "icons/settings.svg",
                    secondary(),
                    "Scale".into(),
                    format!("{}%", (logical.scale * 100.0).round() as u32).into(),
                ));
                rows.push(value_row(
                    "icons/refresh-cw.svg",
                    secondary(),
                    "Rotation".into(),
                    logical.transform.label().into(),
                ));
                rows.push(value_row(
                    "icons/folder-symlink.svg",
                    secondary(),
                    "Position".into(),
                    format!("{}, {}", logical.x, logical.y).into(),
                ));
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Logical Size".into(),
                    format!("{} × {}", logical.width, logical.height).into(),
                ));
            }
            if let Some((width, height)) = output.physical_size_mm {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Physical Size".into(),
                    format!("{width} × {height} mm").into(),
                ));
            }
            cards.push(card(rows));

            let Some(logical) = output.logical.as_ref() else {
                continue;
            };
            if !self.display.can_configure {
                continue;
            }

            if (0.5..=4.0).contains(&logical.scale) {
                cards.push(section_header("Scale"));
                let scale_rows = [1.0, 1.25, 1.5, 1.75, 2.0]
                    .into_iter()
                    .map(|scale| {
                        let selected = (logical.scale - scale).abs() < 0.001;
                        let output_id = output.id.clone();
                        let current = logical.scale;
                        let scale_view = view.clone();
                        row_base()
                            .id(SharedString::from(format!(
                                "display-scale-{output_id}-{scale}"
                            )))
                            .child(text_block(
                                format!("{}%", (scale * 100.0) as u32).into(),
                                None,
                            ))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Scale {
                                            output: output_id.clone(),
                                            scale,
                                        };
                                        let revert = DisplayChange::Scale {
                                            output: output_id.clone(),
                                            scale: current,
                                        };
                                        scale_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, revert, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(scale_rows));
            }

            if logical.transform.is_configurable() {
                cards.push(section_header("Rotation"));
                let rotations = [
                    rmac_display::Transform::Normal,
                    rmac_display::Transform::Rotate90,
                    rmac_display::Transform::Rotate180,
                    rmac_display::Transform::Rotate270,
                ];
                let rotation_rows = rotations
                    .into_iter()
                    .map(|transform| {
                        let selected = logical.transform == transform;
                        let label = transform.label();
                        let output_id = output.id.clone();
                        let current = logical.transform.clone();
                        let rotation_view = view.clone();
                        row_base()
                            .id(SharedString::from(format!(
                                "display-rotation-{output_id}-{label}"
                            )))
                            .child(text_block(label.into(), None))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Transform {
                                            output: output_id.clone(),
                                            transform: transform.clone(),
                                        };
                                        let revert = DisplayChange::Transform {
                                            output: output_id.clone(),
                                            transform: current.clone(),
                                        };
                                        rotation_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, revert, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(rotation_rows));
            }

            if let Some(current_mode) = output.current_mode() {
                cards.push(section_header("Resolution"));
                let mode_rows = output
                    .modes
                    .iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        let mode = *mode;
                        let selected = output.current_mode == Some(index);
                        let output_id = output.id.clone();
                        let mode_view = view.clone();
                        let subtitle = mode.preferred.then(|| "Preferred".into());
                        row_base()
                            .id(SharedString::from(format!(
                                "display-mode-{output_id}-{index}"
                            )))
                            .child(text_block(mode.label().into(), subtitle))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Mode {
                                            output: output_id.clone(),
                                            mode,
                                        };
                                        let revert = DisplayChange::Mode {
                                            output: output_id.clone(),
                                            mode: current_mode,
                                        };
                                        mode_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, revert, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(mode_rows));
            }
        }

        if let Some(graphics) = &self.display.graphics {
            cards.push(section_header("Graphics"));
            cards.push(card(vec![value_row(
                "icons/settings.svg",
                secondary(),
                "Chipset".into(),
                graphics.clone().into(),
            )]));
        }
        if self.display.can_configure {
            cards.push(note_card(
                "Display changes are temporary in niri. Revert restores the previous value; persistent layout editing will write a validated niri configuration later.",
            ));
        }
        self.pane(cards)
    }

    // ---- Network (real read-only) -------------------------------------

    fn render_network(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let connected = matches!(
            self.network.connectivity,
            rmac_network::Connectivity::Full
                | rmac_network::Connectivity::Limited
                | rmac_network::Connectivity::Portal
        );
        let summary = self
            .network
            .primary_connection
            .clone()
            .unwrap_or_else(|| "No primary connection".into());
        let refresh_view = view.clone();
        let refresh_label = if self.network_busy {
            "Refreshing…"
        } else {
            "Refresh"
        };
        let mut cards = vec![card(vec![
            value_row(
                "icons/globe.svg",
                if connected {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                "Status".into(),
                self.network.connectivity.label().into(),
            ),
            value_row(
                "icons/folder-symlink.svg",
                accent(),
                "Primary Connection".into(),
                summary.into(),
            ),
        ])];
        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_1()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child("Interfaces"),
                )
                .child(
                    div()
                        .id("network-refresh")
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .text_color(accent())
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .child(refresh_label)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| settings.refresh_network(cx));
                        }),
                ),
        );

        if self.network_loading {
            cards.push(note_card("Loading network state from the system…"));
            return self.pane(cards);
        }
        if !self.network.available {
            cards.push(note_card(
                "The system network service is not available on this computer.",
            ));
            return self.pane(cards);
        }
        if self.network.devices.is_empty() {
            cards.push(note_card("No managed network interfaces were found."));
            return self.pane(cards);
        }

        for device in &self.network.devices {
            let title = device
                .connection
                .as_deref()
                .unwrap_or_else(|| device.kind.label());
            let heading = if device.primary {
                format!("{title} · Primary")
            } else {
                title.to_string()
            };
            cards.push(
                div()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .text_size(px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(heading),
            );
            let mut rows = vec![value_row(
                if device.kind == rmac_network::DeviceKind::WiFi {
                    "icons/wifi.svg"
                } else {
                    "icons/globe.svg"
                },
                if device.state.is_connected() {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                format!("{} ({})", device.kind.label(), device.interface).into(),
                device.state.label().into(),
            )];
            if !device.addresses.is_empty() {
                rows.push(value_row(
                    "icons/globe.svg",
                    secondary(),
                    "IP Addresses".into(),
                    device.addresses.join(", ").into(),
                ));
            }
            if let Some(gateway) = &device.gateway {
                rows.push(value_row(
                    "icons/folder-symlink.svg",
                    secondary(),
                    "Router".into(),
                    gateway.clone().into(),
                ));
            }
            if !device.dns.is_empty() {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "DNS Servers".into(),
                    device.dns.join(", ").into(),
                ));
            }
            if let Some(address) = &device.hardware_address {
                rows.push(value_row(
                    "icons/key.svg",
                    secondary(),
                    "Hardware Address".into(),
                    address.clone().into(),
                ));
            }
            cards.push(card(rows));
        }
        self.pane(cards)
    }

    // ---- VPN ----------------------------------------------------------

    fn render_vpn(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let connected = self
            .vpn
            .profiles
            .iter()
            .filter(|profile| profile.state == rmac_network::VpnState::Connected)
            .count();
        let status = match connected {
            0 => "No VPN Connected".to_string(),
            1 => "1 VPN Connected".to_string(),
            count => format!("{count} VPNs Connected"),
        };
        let refresh_view = view.clone();
        let refresh_label = if self.vpn_busy.is_some() {
            "Refreshing…"
        } else {
            "Refresh"
        };
        let mut cards = vec![card(vec![value_row(
            "icons/key.svg",
            if connected > 0 {
                hsl(0x34c759)
            } else {
                secondary()
            },
            "Status".into(),
            status.into(),
        )])];
        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_1()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child("VPN Configurations"),
                )
                .child(
                    div()
                        .id("vpn-refresh")
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .text_color(accent())
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .child(refresh_label)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| settings.refresh_vpn(cx));
                        }),
                ),
        );
        if self.vpn_loading {
            cards.push(note_card("Loading VPN configurations from the system…"));
            return self.pane(cards);
        }
        if !self.vpn.available {
            cards.push(note_card(
                "The system VPN service is not available on this computer.",
            ));
            return self.pane(cards);
        }
        if self.vpn.profiles.is_empty() {
            cards.push(note_card(
                "No VPN configurations are installed. Profile import will be added next.",
            ));
            return self.pane(cards);
        }

        let rows = self
            .vpn
            .profiles
            .iter()
            .map(|profile| {
                let identifier = profile.identifier.clone();
                let switch_identifier = identifier.clone();
                let profile_view = view.clone();
                let applying = self.vpn_busy.as_deref() == Some(identifier.as_str());
                let subtitle = if applying {
                    format!("{} · Applying change…", profile.service)
                } else {
                    format!("{} · {}", profile.service, profile.state.label())
                };
                let control = Toggle::new(ElementId::from(SharedString::from(format!(
                    "vpn-{identifier}"
                ))))
                .checked(profile.state.is_enabled())
                .on_click(move |enabled, _, cx| {
                    let identifier = switch_identifier.clone();
                    profile_view.update(cx, |settings, cx| {
                        settings.set_vpn_enabled(identifier, *enabled, cx)
                    });
                });
                row_base()
                    .child(tile(
                        "icons/key.svg",
                        if profile.state == rmac_network::VpnState::Connected {
                            hsl(0x34c759)
                        } else {
                            accent()
                        },
                        22.0,
                    ))
                    .child(text_block(
                        profile.name.clone().into(),
                        Some(subtitle.into()),
                    ))
                    .child(control)
                    .into_any_element()
            })
            .collect();
        cards.push(card(rows));
        cards.push(note_card(
            "Connections are controlled by the system network service. Authentication prompts are handled by the installed VPN plugin.",
        ));
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
                    .bg(rmac_ui::mac::control_fill())
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

    fn render_subpage(&self, sub: &SubPage, cx: &Context<Self>) -> Div {
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
                        "rmac reads the installed operating-system version. The Ubuntu update service is not connected yet, so available updates are not reported.",
                    )),
            ),
            SubPage::Storage => ("Storage".into(), self.storage_body()),
            SubPage::NotificationApp { app_id } => (
                self.application_identity(app_id)
                    .map(|identity| identity.name.clone())
                    .unwrap_or_else(|| app_id.clone())
                    .into(),
                self.notification_app_body(app_id, cx),
            ),
            SubPage::FocusMode { mode_id } => {
                let title = rmac_focus::ModeId::parse(mode_id)
                    .ok()
                    .and_then(|mode_id| {
                        self.focus_policy_config
                            .as_ref()
                            .and_then(|configuration| configuration.mode(&mode_id))
                            .map(|mode| mode.name().to_owned())
                    })
                    .unwrap_or_else(|| "Focus".into());
                (title.into(), self.focus_mode_body(mode_id, cx))
            }
            SubPage::FocusSchedule { schedule_id } => (
                "Schedule".into(),
                self.focus_schedule_body(schedule_id, cx),
            ),
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
                "Operating System".into(),
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
        let settings_error = self
            .persistence_error
            .clone()
            .or_else(|| self.wifi_error.clone())
            .or_else(|| self.bluetooth_error.clone())
            .or_else(|| self.network_error.clone())
            .or_else(|| self.vpn_error.clone())
            .or_else(|| self.audio_error.clone())
            .or_else(|| self.power_error.clone())
            .or_else(|| self.display_error.clone())
            .or_else(|| self.input_error.clone())
            .or_else(|| self.theme_error.clone());
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
            .when_some(settings_error, |settings, message| {
                settings.child(
                    Toast::new("settings-error", ToastKind::Error, "Settings error")
                        .message(message)
                        .rounded(px(0.0))
                        .border_l_0()
                        .border_r_0()
                        .on_dismiss(cx.listener(|this, _, _, cx| {
                            this.persistence_error = None;
                            this.wifi_error = None;
                            this.bluetooth_error = None;
                            this.network_error = None;
                            this.vpn_error = None;
                            this.audio_error = None;
                            this.power_error = None;
                            this.display_error = None;
                            this.input_error = None;
                            this.theme_error = None;
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
fn note_card(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mb_3()
        .px_3()
        .py_2p5()
        .rounded(px(10.0))
        .bg(rmac_ui::mac::warning_background())
        .border_1()
        .border_color(rmac_ui::mac::warning_border())
        .child(glyph("icons/info.svg", 15.0, rmac_ui::mac::warning_text()))
        .child(
            div()
                .flex_1()
                .text_size(px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
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
    let sw = Toggle::new(id).checked(checked).on_change(move |v, _, cx| {
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

#[allow(clippy::too_many_arguments)]
fn notification_toggle_row(
    view: &Entity<Settings>,
    app_id: &str,
    id: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    disabled: bool,
    change: fn(bool) -> NotificationPolicyChange,
) -> AnyElement {
    let app = app_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "notification-{id}-{app_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |value, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.apply_notification_policy(app.clone(), change(*value), cx);
        });
    });
    row_base()
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(toggle)
        .into_any_element()
}

fn application_icon(
    icon: Option<&PathBuf>,
    fallback_icon: &'static str,
    fallback_color: Hsla,
) -> AnyElement {
    match icon {
        Some(icon) => img(icon.clone())
            .w(px(22.0))
            .h(px(22.0))
            .flex_none()
            .into_any_element(),
        None => tile(fallback_icon, fallback_color, 22.0).into_any_element(),
    }
}

#[allow(clippy::too_many_arguments)]
fn application_nav_row(
    view: &Entity<Settings>,
    app_id: &str,
    display_name: &str,
    icon: Option<&PathBuf>,
    status: &'static str,
    enabled: bool,
    target: SubPage,
) -> AnyElement {
    let target_view = view.clone();
    row_base()
        .id(SharedString::from(format!("notification-app-{app_id}")))
        .cursor_pointer()
        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
        .child(application_icon(
            icon,
            "icons/bell.svg",
            if enabled { accent() } else { secondary() },
        ))
        .child(text_block(display_name.to_owned().into(), None))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(secondary())
                .child(status),
        )
        .child(glyph(
            "icons/chevron-right.svg",
            14.0,
            rmac_ui::mac::text_tertiary(),
        ))
        .on_click(move |_, _, cx| {
            let target = target.clone();
            target_view.update(cx, |settings, cx| settings.push(target, cx));
        })
        .into_any_element()
}

fn focus_schedule_row(
    view: &Entity<Settings>,
    schedule: &rmac_focus::Schedule,
    mode_name: &str,
    disabled: bool,
) -> AnyElement {
    let schedule_id = schedule.id.as_str().to_owned();
    let toggle_schedule_id = schedule_id.clone();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-schedule-{schedule_id}"
    ))))
    .checked(schedule.enabled)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_schedule_enabled(toggle_schedule_id.clone(), *enabled, cx);
        });
    });
    let edit_view = view.clone();
    row_base()
        .child(tile("icons/clock.svg", accent(), 22.0))
        .child(text_block(
            mode_name.to_owned().into(),
            Some(focus_schedule_summary(schedule).into()),
        ))
        .child(
            div()
                .id(SharedString::from(format!(
                    "focus-edit-schedule-{schedule_id}"
                )))
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(accent())
                .when(!disabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .on_click(move |_, _, cx| {
                            let schedule_id = schedule_id.clone();
                            edit_view.update(cx, |settings, cx| {
                                settings.push(SubPage::FocusSchedule { schedule_id }, cx);
                            });
                        })
                })
                .child("Edit"),
        )
        .child(toggle)
        .into_any_element()
}

fn focus_schedule_toggle_row(
    view: &Entity<Settings>,
    schedule: &rmac_focus::Schedule,
    mode_name: &str,
    disabled: bool,
) -> AnyElement {
    let schedule_id = schedule.id.as_str().to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-schedule-enabled-{schedule_id}"
    ))))
    .checked(schedule.enabled)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_schedule_enabled(schedule_id.clone(), *enabled, cx);
        });
    });
    row_base()
        .child(tile("icons/moon.svg", accent(), 22.0))
        .child(text_block(
            mode_name.to_owned().into(),
            Some("Turn this schedule on automatically".into()),
        ))
        .child(toggle)
        .into_any_element()
}

fn focus_day_button(
    view: &Entity<Settings>,
    schedule_id: &str,
    day: rmac_focus::Weekday,
    day_label: &'static str,
    selected: bool,
    disabled: bool,
) -> Div {
    let control_view = view.clone();
    let schedule_id = schedule_id.to_owned();
    div()
        .id(SharedString::from(format!(
            "focus-day-{schedule_id}-{day:?}"
        )))
        .flex_1()
        .h(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(16.0))
        .text_size(px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .bg(if selected { accent() } else { card_bg() })
        .text_color(if selected { on_accent() } else { label() })
        .border_1()
        .border_color(if selected { accent() } else { sep() })
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(|hover| hover.opacity(0.82))
                .on_click(move |_, _, cx| {
                    control_view.update(cx, |settings, cx| {
                        settings.set_focus_schedule_day(schedule_id.clone(), day, !selected, cx);
                    });
                })
        })
        .child(day_label)
}

fn focus_time_row(
    view: &Entity<Settings>,
    schedule_id: &str,
    title: &'static str,
    start: bool,
    minute: u16,
    disabled: bool,
) -> AnyElement {
    let previous = (minute + 1_440 - 15) % 1_440;
    let next = (minute + 15) % 1_440;
    let previous_view = view.clone();
    let next_view = view.clone();
    let previous_schedule = schedule_id.to_owned();
    let next_schedule = schedule_id.to_owned();
    let decrement = div()
        .id(SharedString::from(format!(
            "focus-time-minus-{schedule_id}-{start}"
        )))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rmac_ui::mac::hover())
        .text_size(px(18.0))
        .when(!disabled, |button| {
            button.cursor_pointer().on_click(move |_, _, cx| {
                previous_view.update(cx, |settings, cx| {
                    settings.set_focus_schedule_time(
                        previous_schedule.clone(),
                        start,
                        previous,
                        cx,
                    );
                });
            })
        })
        .child("−");
    let increment = div()
        .id(SharedString::from(format!(
            "focus-time-plus-{schedule_id}-{start}"
        )))
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rmac_ui::mac::hover())
        .text_size(px(18.0))
        .when(!disabled, |button| {
            button.cursor_pointer().on_click(move |_, _, cx| {
                next_view.update(cx, |settings, cx| {
                    settings.set_focus_schedule_time(next_schedule.clone(), start, next, cx);
                });
            })
        })
        .child("+");
    row_base()
        .child(text_block(title.into(), None))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(decrement)
                .child(
                    div()
                        .w(px(74.0))
                        .text_center()
                        .text_size(px(12.0))
                        .child(focus_time(minute)),
                )
                .child(increment),
        )
        .into_any_element()
}

fn focus_urgent_row(
    view: &Entity<Settings>,
    mode_id: &str,
    checked: bool,
    disabled: bool,
) -> AnyElement {
    let mode_id = mode_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-urgent-{mode_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |enabled, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_urgent(mode_id.clone(), *enabled, cx);
        });
    });
    row_base()
        .child(text_block(
            "Allow urgent notifications".into(),
            Some("Let urgent notifications through while this Focus is on".into()),
        ))
        .child(toggle)
        .into_any_element()
}

fn focus_allowed_app_row(
    view: &Entity<Settings>,
    mode_id: &str,
    app_id: &str,
    display_name: &str,
    icon: Option<&PathBuf>,
    checked: bool,
    disabled: bool,
) -> AnyElement {
    let mode_id = mode_id.to_owned();
    let application_id = app_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "focus-allowed-{mode_id}-{app_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |allowed, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.set_focus_allowed_app(mode_id.clone(), application_id.clone(), *allowed, cx);
        });
    });
    row_base()
        .child(application_icon(icon, "icons/app-window.svg", secondary()))
        .child(text_block(display_name.to_owned().into(), None))
        .child(toggle)
        .into_any_element()
}

fn focus_schedule_summary(schedule: &rmac_focus::Schedule) -> String {
    let days = schedule
        .days
        .iter()
        .map(|day| match day {
            rmac_focus::Weekday::Monday => "Mon",
            rmac_focus::Weekday::Tuesday => "Tue",
            rmac_focus::Weekday::Wednesday => "Wed",
            rmac_focus::Weekday::Thursday => "Thu",
            rmac_focus::Weekday::Friday => "Fri",
            rmac_focus::Weekday::Saturday => "Sat",
            rmac_focus::Weekday::Sunday => "Sun",
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{days} · {}–{}",
        focus_time(schedule.start_minute),
        focus_time(schedule.end_minute)
    )
}

fn focus_time(minute: u16) -> String {
    let hour = minute / 60;
    let minute = minute % 60;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    format!("{hour}:{minute:02} {suffix}")
}

fn bluetooth_device_row(view: &Entity<Settings>, device: &rmac_bluetooth::Device) -> AnyElement {
    let subtitle = match (device.kind.is_empty(), device.address.is_empty()) {
        (false, false) => Some(format!("{} · {}", device.kind, device.address).into()),
        (false, true) => Some(device.kind.clone().into()),
        (true, false) => Some(device.address.clone().into()),
        (true, true) => None,
    };
    let action = if device.connected {
        "Disconnect"
    } else if device.paired {
        "Connect"
    } else {
        "Not Paired"
    };
    let device_id = device.id.clone();
    let connect = !device.connected;
    let action_view = view.clone();
    row_base()
        .child(tile(
            "icons/bluetooth.svg",
            if device.connected {
                accent()
            } else {
                secondary()
            },
            22.0,
        ))
        .child(text_block(device.name.clone().into(), subtitle))
        .child(
            div()
                .id(SharedString::from(format!("bluetooth-device-{device_id}")))
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(if device.paired { accent() } else { secondary() })
                .when(device.paired, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .on_click(move |_, _, cx| {
                            action_view.update(cx, |settings, cx| {
                                settings.set_bluetooth_device_connected(
                                    device_id.clone(),
                                    connect,
                                    cx,
                                );
                            });
                        })
                })
                .child(action),
        )
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

type InputOption = (&'static str, InputChange);
type ThemeOption = (&'static str, ThemeChange);

const THEME_CONTRAST_OPTIONS: [ThemeOption; 3] = [
    (
        "Automatic",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Automatic),
    ),
    (
        "Normal",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Normal),
    ),
    (
        "Higher",
        ThemeChange::Contrast(rmac_theme::ContrastPreference::Higher),
    ),
];
const THEME_MOTION_OPTIONS: [ThemeOption; 3] = [
    (
        "Automatic",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Automatic),
    ),
    (
        "Full",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Full),
    ),
    (
        "Reduced",
        ThemeChange::Motion(rmac_theme::MotionPreferenceSetting::Reduced),
    ),
];

const KEYBOARD_DELAYS: [InputOption; 5] = [
    ("Short", InputChange::KeyboardRepeatDelay(200)),
    ("300", InputChange::KeyboardRepeatDelay(300)),
    ("500", InputChange::KeyboardRepeatDelay(500)),
    ("750", InputChange::KeyboardRepeatDelay(750)),
    ("Long", InputChange::KeyboardRepeatDelay(1_000)),
];
const KEYBOARD_RATES: [InputOption; 5] = [
    ("Slow", InputChange::KeyboardRepeatRate(10)),
    ("20", InputChange::KeyboardRepeatRate(20)),
    ("30", InputChange::KeyboardRepeatRate(30)),
    ("40", InputChange::KeyboardRepeatRate(40)),
    ("Fast", InputChange::KeyboardRepeatRate(60)),
];
const MOUSE_SPEEDS: [InputOption; 5] = [
    ("Slow", InputChange::MouseAccelSpeed(-1.0)),
    ("−0.5", InputChange::MouseAccelSpeed(-0.5)),
    ("Default", InputChange::MouseAccelSpeed(0.0)),
    ("0.5", InputChange::MouseAccelSpeed(0.5)),
    ("Fast", InputChange::MouseAccelSpeed(1.0)),
];
const TOUCHPAD_SPEEDS: [InputOption; 5] = [
    ("Slow", InputChange::TouchpadAccelSpeed(-1.0)),
    ("−0.5", InputChange::TouchpadAccelSpeed(-0.5)),
    ("Default", InputChange::TouchpadAccelSpeed(0.0)),
    ("0.5", InputChange::TouchpadAccelSpeed(0.5)),
    ("Fast", InputChange::TouchpadAccelSpeed(1.0)),
];
const MOUSE_PROFILES: [InputOption; 2] = [
    (
        "Adaptive",
        InputChange::MouseAccelProfile(rmac_input::AccelProfile::Adaptive),
    ),
    (
        "Flat",
        InputChange::MouseAccelProfile(rmac_input::AccelProfile::Flat),
    ),
];
const TOUCHPAD_PROFILES: [InputOption; 2] = [
    (
        "Adaptive",
        InputChange::TouchpadAccelProfile(rmac_input::AccelProfile::Adaptive),
    ),
    (
        "Flat",
        InputChange::TouchpadAccelProfile(rmac_input::AccelProfile::Flat),
    ),
];

fn speed_index(speed: f64) -> usize {
    [-1.0, -0.5, 0.0, 0.5, 1.0]
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (speed - **a).abs().total_cmp(&(speed - **b).abs()))
        .map(|(index, _)| index)
        .unwrap_or(2)
}

fn accent_preference(hex: u32) -> rmac_theme::AccentPreference {
    rmac_theme::AccentPreference::Custom([
        f64::from((hex >> 16) & 0xff) / 255.0,
        f64::from((hex >> 8) & 0xff) / 255.0,
        f64::from(hex & 0xff) / 255.0,
    ])
}

fn theme_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [ThemeOption],
    selected: usize,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{index}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .when(index == selected, |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(index != selected, |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.apply_theme_change(change, cx));
                        })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child(option_label),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

fn input_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [InputOption],
    selected: usize,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{index}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .when(index == selected, |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(index != selected, |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.apply_input_change(change, cx));
                        })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child(option_label),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn input_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> InputChange,
) -> AnyElement {
    let mut switch = Toggle::new(id).checked(checked);
    if enabled {
        switch = switch.on_click(move |value, _, cx| {
            view.update(cx, |settings, cx| {
                settings.apply_input_change(change(*value), cx)
            });
        });
    }
    row_base()
        .child(tile(icon, secondary(), 22.0))
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(
            div()
                .when(!enabled, |element| element.opacity(0.55))
                .child(switch),
        )
        .into_any_element()
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
        .hover(|h| h.bg(rmac_ui::mac::hover()))
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None));
    if let Some(v) = value {
        r = r.child(div().text_size(px(13.0)).text_color(secondary()).child(v));
    }
    r.child(glyph(
        "icons/chevron-right.svg",
        14.0,
        rmac_ui::mac::text_tertiary(),
    ))
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
                .when(is_sel, |el| el.bg(accent()).text_color(on_accent()))
                .when(!is_sel, |el| {
                    el.bg(rmac_ui::mac::control_fill())
                        .text_color(label())
                        .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
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
                .when(is_sel, |el| el.bg(accent()).text_color(on_accent()))
                .when(!is_sel, |el| {
                    el.bg(rmac_ui::mac::control_fill())
                        .text_color(label())
                        .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
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
    SystemSnapshot {
        account: account_name(),
        sysinfo: gather_sysinfo(),
        storage: gather_storage(),
    }
}

async fn load_theme_state() -> std::result::Result<ThemeLoad, String> {
    let host = match rmac_appearance_portal::snapshot().await {
        Ok(host) => host,
        Err(error) => rmac_appearance::Snapshot::unavailable(error.to_string()),
    };
    let store = rmac_theme::ThemeStore::from_environment().map_err(|error| error.to_string())?;
    let theme = store.load(&host).map_err(|error| error.to_string())?;
    Ok(ThemeLoad { host, theme })
}

fn account_name() -> String {
    cmd("id", &["-F"])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "User".into())
}

#[cfg(target_os = "macos")]
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

#[cfg(not(target_os = "macos"))]
fn gather_sysinfo() -> SysInfo {
    let computer_name = cmd("hostname", &[]).unwrap_or_else(|| "Linux computer".into());
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| os_release_value(&contents, "PRETTY_NAME"))
        .unwrap_or_else(|| "Linux".into());
    let chip = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            ["model name", "Hardware", "Processor"]
                .into_iter()
                .find_map(|key| colon_value(&contents, key))
        })
        .unwrap_or_else(|| "—".into());
    let memory = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| colon_value(&contents, "MemTotal"))
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kibibytes| format!("{:.1} GB", kibibytes as f64 / 1024.0 / 1024.0))
        .unwrap_or_else(|| "—".into());
    let model = read_trimmed("/sys/class/dmi/id/product_name").unwrap_or_else(|| "Computer".into());
    let serial = read_trimmed("/sys/class/dmi/id/product_serial").unwrap_or_else(|| "—".into());
    SysInfo {
        computer_name,
        os,
        chip,
        memory,
        model,
        serial,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn os_release_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key).then(|| value.trim().trim_matches(['\'', '"']).to_string())
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn colon_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        (candidate.trim() == key).then(|| value.trim().to_string())
    })
}

#[cfg(not(target_os = "macos"))]
fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn format_power_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours} hr {minutes} min")
    } else {
        format!("{minutes} min")
    }
}

fn power_degradation_label(reason: &str) -> String {
    match reason {
        "lap-detected" => "Limited while the computer is on a lap".to_string(),
        "high-operating-temperature" => "Limited because of high temperature".to_string(),
        _ => "Limited by the system".to_string(),
    }
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
            cat(
                "Wi-Fi",
                "icons/wifi.svg",
                blue,
                "Connect to Wi-Fi networks and manage known networks.",
                vec![],
            ),
            cat(
                "Bluetooth",
                "icons/bluetooth.svg",
                blue,
                "Pair and manage Bluetooth devices.",
                vec![],
            ),
            cat(
                "Network",
                "icons/globe.svg",
                blue,
                "Configure network services and connections.",
                vec![],
            ),
            cat(
                "VPN",
                "icons/key.svg",
                blue,
                "Set up and manage VPN configurations.",
                vec![],
            ),
            cat(
                "Battery",
                "icons/battery-charging.svg",
                green,
                "Monitor battery usage and energy settings.",
                vec![],
            ),
        ],
        vec![
            cat(
                "General",
                "icons/settings.svg",
                gray,
                "Manage system information, updates, storage, language, startup, and sharing.",
                vec![
                    vec![
                        row("icons/info.svg", gray, "About"),
                        row("icons/refresh-cw.svg", gray, "Software Update"),
                        row("icons/database.svg", gray, "Storage"),
                    ],
                    vec![row(
                        "icons/heart-handshake.svg",
                        red,
                        "AppleCare & Warranty",
                    )],
                    vec![row(
                        "icons/folder-symlink.svg",
                        blue,
                        "AirDrop & Continuity",
                    )],
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
            cat(
                "Accessibility",
                "icons/accessibility.svg",
                blue,
                "Customize the computer for the way you work.",
                vec![],
            ),
            cat(
                "Appearance",
                "icons/palette.svg",
                hsl(0x1d1d1f),
                "Change how windows, buttons, and menus look.",
                vec![],
            ),
            cat(
                "Assistant & Intelligence",
                "icons/sparkles.svg",
                purple,
                "Configure supported local or connected assistant services.",
                vec![],
            ),
            cat(
                "Desktop & Dock",
                "icons/app-window.svg",
                gray,
                "Adjust the Dock, Stage Manager, and windows.",
                vec![],
            ),
            cat(
                "Displays",
                "icons/monitor.svg",
                blue,
                "Arrange displays and adjust resolution.",
                vec![],
            ),
            cat(
                "Spotlight",
                "icons/search.svg",
                gray,
                "Choose which categories Spotlight searches.",
                vec![],
            ),
            cat(
                "Wallpaper",
                "icons/image.svg",
                teal,
                "Choose a wallpaper for your desktop.",
                vec![],
            ),
        ],
        vec![
            cat(
                "Notifications",
                "icons/bell.svg",
                red,
                "Choose how you receive notifications.",
                vec![],
            ),
            cat(
                "Sound",
                "icons/volume-2.svg",
                pink,
                "Adjust sound effects and output.",
                vec![],
            ),
            cat(
                "Keyboard",
                "icons/keyboard.svg",
                gray,
                "Adjust key repeat behavior and keyboard startup options.",
                vec![],
            ),
            cat(
                "Mouse",
                "icons/mouse.svg",
                gray,
                "Adjust tracking, scrolling, acceleration, and buttons.",
                vec![],
            ),
            cat(
                "Trackpad",
                "icons/touchpad.svg",
                gray,
                "Adjust tracking, tapping, scrolling, and gestures.",
                vec![],
            ),
            cat(
                "Focus",
                "icons/moon.svg",
                indigo,
                "Stay focused by silencing notifications.",
                vec![],
            ),
            cat(
                "Screen Time",
                "icons/timer.svg",
                indigo,
                "Monitor usage and set limits.",
                vec![],
            ),
        ],
        vec![
            cat(
                "Lock Screen",
                "icons/lock.svg",
                gray,
                "Adjust your lock screen and login.",
                vec![],
            ),
            cat(
                "Privacy & Security",
                "icons/shield.svg",
                blue,
                "Control what the system and applications can access.",
                vec![],
            ),
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
    use super::{
        colon_value, notification_policy_with, os_release_value, NotificationPolicyChange,
        Persisted,
    };

    #[test]
    fn persisted_settings_round_trip() {
        let expected = Persisted {
            wifi_on: false,
            ask_to_join: false,
            joined: Some(3),
            bluetooth_on: false,
            bt_discoverable: false,
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
                "alert_idx": 999,
                "airdrop_idx": 999
            }"#,
        )
        .unwrap();

        assert_eq!(parsed.output_volume, 100.0);
        assert_eq!(parsed.alert_volume, 0.0);
        assert_eq!(parsed.balance, 100.0);
        assert_eq!(parsed.alert_idx, 0);
        assert_eq!(parsed.airdrop_idx, 1);
    }

    #[test]
    fn legacy_local_appearance_keys_are_ignored_and_not_rewritten() {
        let parsed = Persisted::parse(
            r#"{"appearance":2,"accent_idx":4,"show_color_in_menu":0,"large_sidebar":1}"#,
        )
        .unwrap();
        let serialized = parsed.to_json();
        assert!(!serialized.contains("appearance"));
        assert!(!serialized.contains("accent_idx"));
        assert!(!serialized.contains("show_color_in_menu"));
        assert!(!serialized.contains("large_sidebar"));
    }

    #[test]
    fn linux_system_information_parsers_handle_standard_files() {
        let release = "NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\n";
        let cpu = "processor : 0\nmodel name : Example CPU\n";

        assert_eq!(
            os_release_value(release, "PRETTY_NAME").as_deref(),
            Some("Ubuntu 26.04 LTS")
        );
        assert_eq!(
            colon_value(cpu, "model name").as_deref(),
            Some("Example CPU")
        );
    }

    #[test]
    fn notification_policy_changes_touch_only_the_selected_field() {
        let original = rmac_notifications_store::AppPolicy::default();
        let changed = notification_policy_with(original, NotificationPolicyChange::History(false));
        assert!(!changed.history);
        assert_eq!(changed.enabled, original.enabled);
        assert_eq!(changed.banners, original.banners);
        assert_eq!(changed.sounds, original.sounds);
        assert_eq!(changed.badges, original.badges);
        assert_eq!(changed.urgent_through_focus, original.urgent_through_focus);
        assert_eq!(changed.lock_preview, original.lock_preview);
    }
}
