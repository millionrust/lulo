//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · local account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive and backed by typed Linux/macOS services. Unsupported
//! mutations are explicitly unavailable rather than represented by local state.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Read-only panes use real platform state rather than fabricated values.

mod accessibility;
mod appearance;
mod bluetooth;
mod chrome;
mod date_time;
mod desktop_dock;
mod detail;
mod displays;
mod focus;
mod initialization;
mod input;
mod locale;
mod lock_screen;
mod login_items;
mod navigation_accessibility;
mod navigation_persistence;
mod navigation_state;
mod network;
mod notifications;
mod power;
mod privacy_security;
mod sharing;
mod shell_render;
mod shell_settings;
mod software_updates;
mod sound;
mod spotlight;
mod state;
mod storage;
mod system_info;
mod view_helpers;
mod vpn;
mod wallpaper;
mod wifi;

use state::Settings;
use view_helpers::*;

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::appearance::{
    accent_preference, apply_theme_change_authoritatively, load_theme_state, ThemeChange,
    ThemeLoad, ThemeOption, ThemeStoreWatchEvent, ACCENTS, THEME_CONTRAST_OPTIONS,
    THEME_MOTION_OPTIONS, THEME_TEXT_SCALE_OPTIONS,
};
use crate::connectivity::{
    wifi_join_action, BluetoothForgetPrompt, BluetoothPairingDisplay, BluetoothPairingState,
    NetworkEditorState, VpnEditorState, WifiEnterprisePrompt, WifiForgetPrompt, WifiJoinAction,
    WifiPasswordPrompt,
};
use crate::displays::{
    compositor_event_affects_displays, relative_display_position, DisplayChange,
    DisplayConfirmation, DisplayPlacement, DISPLAY_CONFIRMATION_SECONDS,
};
use crate::focus::{
    current_action as focus_current_action, load as load_focus, FocusCurrentAction, FocusLoad,
    DAYS as FOCUS_DAYS,
};
use crate::input::{
    compositor_event_affects_input, compositor_input_config_failed, speed_index, InputChange,
    InputOption, KEYBOARD_DELAYS, KEYBOARD_RATES, KEYBOARD_RESPONSE_PRESETS,
    MOUSE_PRECISION_PRESETS, MOUSE_PROFILES, MOUSE_SPEEDS, TOUCHPAD_PROFILES, TOUCHPAD_SPEEDS,
};
use crate::navigation::{
    categories, category_has_dedicated_renderer, category_name_for_pane_id, category_position,
    pane_id_for_category_name, Category, SubPage, GENERAL_DESTINATIONS,
};
use crate::notifications::{policy_with as notification_policy_with, NotificationPolicyChange};
use crate::power::{
    apply_charge_threshold, apply_profile as apply_power_profile, charge_threshold_description,
    degradation_label as power_degradation_label, format_duration as format_power_duration,
    sample_history as sample_battery_history,
};
use crate::service_updates::{
    change_needs_followup as audio_change_needs_followup,
    change_needs_followup as power_change_needs_followup,
    snapshot_is_current as audio_stream_snapshot_is_current,
    snapshot_is_current as bluetooth_stream_snapshot_is_current,
    snapshot_is_current as gtk_text_stream_snapshot_is_current,
    snapshot_is_current as input_stream_snapshot_is_current,
    snapshot_is_current as locale_stream_snapshot_is_current,
    snapshot_is_current as login_items_stream_snapshot_is_current,
    snapshot_is_current as network_stream_snapshot_is_current,
    snapshot_is_current as power_stream_snapshot_is_current,
    snapshot_is_current as privacy_stream_snapshot_is_current,
    snapshot_is_current as storage_stream_snapshot_is_current,
    snapshot_is_current as system_info_stream_snapshot_is_current,
    snapshot_is_current as theme_stream_snapshot_is_current,
    snapshot_is_current as time_stream_snapshot_is_current,
    snapshot_is_current as update_stream_snapshot_is_current,
    snapshot_is_current as vpn_stream_snapshot_is_current,
    snapshot_is_current as wifi_stream_snapshot_is_current,
};
#[cfg(test)]
use crate::shell_settings::composite_wallpaper_pixel;
use crate::shell_settings::{
    persist_shell_settings_mutation, render_wallpaper_preview, spotlight_provider_policy,
    validate_search_exclusion, validate_wallpaper_choice, wallpaper_selection,
    wallpaper_source_name, watch_shell_settings, DockChange, ShellSettingsMutation,
    ShellSettingsStreamUpdate, SpotlightAuthority, SpotlightChange, WallpaperChange,
    WallpaperTarget,
};
use crate::sound::{
    choice_is_actionable as audio_choice_is_actionable, SoundChange as AudioChange,
};
use crate::system_environment::{
    gather_screen_reader_capability, gather_system_snapshot, ScreenReaderCapability, SystemSnapshot,
};
use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AnyElement, AppContext as _,
    AssetSource, ClipboardItem, Context, Div, ElementId, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, ObjectFit,
    ParentElement, Render, Result, SharedString, StatefulInteractiveElement as _, Styled,
    StyledImage as _, Svg, Window,
};
use gpui_component::StyledExt as _;
use navigation_persistence::NavigationPersistence;
use rmac_ui::{
    Button, EmptyState, InputState, ListRow, Progress, SearchField, Slider, SliderEvent,
    SliderState, Tabs, TextField, Toast, ToastKind, Toggle,
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

fn current_system_time_usec() -> Option<u64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(elapsed.as_micros()).ok()
}

type DockOption = (&'static str, DockChange);

const DOCK_PLACEMENT_OPTIONS: [DockOption; 3] = [
    (
        "Left",
        DockChange::Placement(rmac_shell_settings::DockPlacement::Left),
    ),
    (
        "Bottom",
        DockChange::Placement(rmac_shell_settings::DockPlacement::Bottom),
    ),
    (
        "Right",
        DockChange::Placement(rmac_shell_settings::DockPlacement::Right),
    ),
];
const DOCK_MAGNIFICATION_OPTIONS: [DockOption; 3] = [
    ("1.25×", DockChange::MagnificationScale(1.25)),
    ("1.5×", DockChange::MagnificationScale(1.5)),
    ("2×", DockChange::MagnificationScale(2.0)),
];
const DOCK_REPEATED_CLICK_OPTIONS: [DockOption; 2] = [
    (
        "Cycle Windows",
        DockChange::RepeatedClick(rmac_shell_settings::RepeatedClickBehavior::CycleWindows),
    ),
    (
        "Do Nothing",
        DockChange::RepeatedClick(rmac_shell_settings::RepeatedClickBehavior::DoNothing),
    ),
];

const WALLPAPER_FIT_OPTIONS: [(&str, rmac_shell_settings::WallpaperFit); 5] = [
    ("Fill", rmac_shell_settings::WallpaperFit::Fill),
    ("Fit", rmac_shell_settings::WallpaperFit::Fit),
    ("Stretch", rmac_shell_settings::WallpaperFit::Stretch),
    ("Center", rmac_shell_settings::WallpaperFit::Center),
    ("Tile", rmac_shell_settings::WallpaperFit::Tile),
];

const SIDEBAR_W: f32 = 248.0;

pub(crate) fn run() {
    rmac_ui::boot_unified_app_with_assets(
        rmac_ui::app_id::SYSTEM_SETTINGS,
        CombinedAssets,
        1000.0,
        720.0,
        |window, cx| {
            cx.bind_keys([KeyBinding::new(
                rmac_ui::shortcuts::BACK.keystroke,
                GoBack,
                Some("SystemSettings"),
            )]);
            Settings::new(window, cx)
        },
    );
}

#[cfg(test)]
mod tests;
