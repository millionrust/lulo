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
mod storage;
mod system_info;
mod view_helpers;
mod vpn;
mod wallpaper;
mod wifi;

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
    Category, SubPage, GENERAL_DESTINATIONS,
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
    ParentElement, Render, Result, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    StyledImage as _, Svg, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{
    Button, EmptyState, InputState, ListRow, Progress, SearchField, Slider, SliderEvent,
    SliderState, TextField, Toast, ToastKind, Toggle,
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

// ---- interactive state enums --------------------------------------------

struct Settings {
    system_data_loading: bool,
    system_data_busy: bool,
    system_data_error: Option<SharedString>,
    system_data_stream_error: Option<SharedString>,
    system_data_generation: u64,
    system_data_refresh_pending: bool,
    system_data_stream_refreshing: bool,
    hostname_editor: Option<Entity<InputState>>,
    diagnostics_copied: bool,
    account: SharedString,
    sysinfo: rmac_system_info::Snapshot,
    screen_reader: ScreenReaderCapability,
    screen_reader_loading: bool,
    updates_loading: bool,
    updates_busy: bool,
    updates_error: Option<SharedString>,
    updates_stream_error: Option<SharedString>,
    updates_generation: u64,
    updates_refresh_pending: bool,
    updates_stream_refreshing: bool,
    updates_preparing: bool,
    updates_installing: bool,
    updates_cancellation: Option<rmac_updates::Cancellation>,
    updates_plan: Option<rmac_updates::InstallPlan>,
    updates_progress: Option<rmac_updates::InstallProgress>,
    updates_result: Option<rmac_updates::InstallResult>,
    updates: Option<rmac_updates::Snapshot>,
    time_loading: bool,
    time_busy: bool,
    time_error: Option<SharedString>,
    time_stream_error: Option<SharedString>,
    time_generation: u64,
    time_refresh_pending: bool,
    time_stream_refreshing: bool,
    clock_editor: Option<Entity<InputState>>,
    clock_confirmation: Option<rmac_time::ClockTarget>,
    clock_setting: bool,
    time: Option<rmac_time::Snapshot>,
    timezone_editor: Option<Entity<InputState>>,
    locale_loading: bool,
    locale_busy: bool,
    locale_error: Option<SharedString>,
    locale_stream_error: Option<SharedString>,
    locale_generation: u64,
    locale_refresh_pending: bool,
    locale_stream_refreshing: bool,
    locale: Option<rmac_locale::Snapshot>,
    locale_editor: Option<Entity<InputState>>,
    region_editor: Option<Entity<InputState>>,
    locale_revert: Option<rmac_locale::LocaleRollback>,
    x11_layout_editor: Option<Entity<InputState>>,
    x11_variant_editor: Option<Entity<InputState>>,
    x11_options_editor: Option<Entity<InputState>>,
    x11_keyboard_revert: Option<rmac_locale::KeyboardRollback>,
    login_items_loading: bool,
    login_item_busy: Option<String>,
    login_items_error: Option<SharedString>,
    login_items_stream_error: Option<SharedString>,
    login_items_generation: u64,
    login_items_refresh_pending: bool,
    login_items_stream_refreshing: bool,
    login_items: Option<rmac_login_items::Snapshot>,
    login_item_add: Option<rmac_login_items::AddPreview>,
    login_item_remove: Option<rmac_login_items::RemovePreview>,
    sharing_loading: bool,
    sharing_busy: bool,
    sharing_error: Option<SharedString>,
    sharing_stream_error: Option<SharedString>,
    sharing: Option<rmac_sharing::Snapshot>,
    sharing_confirmation: Option<bool>,
    file_sharing_confirmation: Option<bool>,
    power: rmac_power::Snapshot,
    display: rmac_display::Snapshot,
    network: rmac_network::NetworkSnapshot,
    storage: Vec<rmac_mounts::Volume>,
    storage_busy: bool,
    storage_error: Option<SharedString>,
    storage_stream_error: Option<SharedString>,
    storage_generation: u64,
    storage_refresh_pending: bool,
    storage_stream_refreshing: bool,
    storage_action_busy: Option<String>,
    audio: rmac_audio::Snapshot,
    input: rmac_input::Snapshot,
    gtk_text: Option<rmac_gtk_settings::Snapshot>,
    privacy: Option<rmac_privacy::Snapshot>,
    security_coverage: Option<rmac_privacy::SecurityCoverageSnapshot>,
    sections: Vec<Vec<Category>>,
    selected: (usize, usize),
    nav: Vec<SubPage>,
    search: Entity<InputState>,
    focus: FocusHandle,
    focused_once: bool,
    dragging: bool,
    wifi_error: Option<SharedString>,
    wifi_stream_error: Option<SharedString>,
    bluetooth_error: Option<SharedString>,
    bluetooth_stream_error: Option<SharedString>,
    network_error: Option<SharedString>,
    network_stream_error: Option<SharedString>,
    vpn_error: Option<SharedString>,
    vpn_stream_error: Option<SharedString>,
    audio_error: Option<SharedString>,
    audio_stream_error: Option<SharedString>,
    power_error: Option<SharedString>,
    power_stream_error: Option<SharedString>,
    display_error: Option<SharedString>,
    input_error: Option<SharedString>,
    input_stream_error: Option<SharedString>,
    theme_error: Option<SharedString>,
    theme_store_stream_error: Option<SharedString>,
    theme_portal_stream_error: Option<SharedString>,
    shell_settings_error: Option<SharedString>,
    shell_settings_stream_error: Option<SharedString>,
    gtk_text_error: Option<SharedString>,
    gtk_text_stream_error: Option<SharedString>,
    privacy_error: Option<SharedString>,
    privacy_stream_error: Option<SharedString>,
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
    lock_request_busy: bool,
    lock_request_error: Option<SharedString>,

    // Desktop & Dock
    shell_settings_loading: bool,
    shell_settings_busy: bool,
    shell_settings: Option<rmac_shell_settings::Snapshot>,
    shell_settings_revert: Option<rmac_shell_settings::DockSettings>,
    dock_compositor: rmac_compositor::State,
    wallpaper_target: WallpaperTarget,
    wallpaper_revert: Option<rmac_shell_settings::WallpaperSettings>,
    wallpaper_error: Option<SharedString>,
    wallpaper_preview: Option<std::sync::Arc<gpui::RenderImage>>,
    wallpaper_preview_loading: bool,
    wallpaper_preview_error: Option<SharedString>,
    wallpaper_preview_watch_error: Option<SharedString>,
    wallpaper_preview_generation: u64,
    _wallpaper_preview_watcher: Option<rmac_wallpaper_image::FileWatcher>,
    spotlight_revert: Option<SpotlightAuthority>,
    spotlight_error: Option<SharedString>,
    recent_history_confirmation: bool,
    recent_history_busy: bool,
    recent_history_notice: Option<SharedString>,
    shortcut_status_loading: bool,
    shortcut_status: Option<rmac_shortcuts::BackendStatus>,
    shortcut_status_error: Option<SharedString>,
    shortcut_configuration_busy: bool,
    shortcut_configuration_error: Option<SharedString>,

    // Network
    network_loading: bool,
    network_busy: bool,
    network_generation: u64,
    network_editor: Option<NetworkEditorState>,

    // VPN
    vpn: rmac_network::VpnSnapshot,
    vpn_loading: bool,
    vpn_refreshing: bool,
    vpn_busy: Option<rmac_network::VpnProfileId>,
    vpn_cancellation: Option<rmac_network::VpnCancellation>,
    vpn_generation: u64,
    vpn_import_capabilities: rmac_network::VpnImportCapabilities,
    vpn_import_loading: bool,
    vpn_import_busy: bool,
    vpn_import_preview: Option<rmac_network::VpnImportPreview>,
    vpn_editor_loading: Option<rmac_network::VpnProfileId>,
    vpn_editor_busy: bool,
    vpn_editor: Option<VpnEditorState>,
    vpn_secret_preparing: bool,
    vpn_secret_busy: bool,
    vpn_secret_preview: Option<rmac_network::VpnSecretClearPreview>,
    vpn_delete_preparing: Option<rmac_network::VpnProfileId>,
    vpn_delete_busy: bool,
    vpn_delete_preview: Option<rmac_network::VpnDeletePreview>,

    // Wi-Fi
    wifi_available: bool,
    wifi_loading: bool,
    wifi_busy: bool,
    wifi_generation: u64,
    wifi_connecting: Option<rmac_network::WifiNetworkId>,
    wifi_forgetting: Option<rmac_network::WifiNetworkId>,
    wifi_forget_confirmation: Option<WifiForgetPrompt>,
    wifi_password_prompt: Option<WifiPasswordPrompt>,
    wifi_enterprise_prompt: Option<WifiEnterprisePrompt>,
    wifi_cancellation: Option<rmac_network::WifiCancellation>,
    wifi_on: bool,
    wifi_interface: Option<String>,
    wifi_networks: Vec<rmac_network::WifiNetwork>,
    wifi_saved_networks: Vec<rmac_network::WifiSavedNetwork>,

    // Bluetooth
    bluetooth_available: bool,
    bluetooth_loading: bool,
    bluetooth_busy: bool,
    bluetooth_generation: u64,
    bluetooth_discovering: bool,
    bluetooth_adapter_name: Option<String>,
    bluetooth_on: bool,
    bt_discoverable: bool,
    bt_devices: Vec<rmac_bluetooth::Device>,
    bluetooth_pairing: Option<BluetoothPairingState>,
    bluetooth_forget_confirmation: Option<BluetoothForgetPrompt>,
    bluetooth_forgetting: Option<String>,

    // Appearance
    host_appearance: rmac_appearance::Snapshot,
    theme: Option<rmac_theme::Snapshot>,
    theme_loading: bool,
    theme_busy: bool,
    theme_generation: u64,
    theme_refresh_pending: bool,
    theme_stream_refreshing: bool,

    // External GTK application text
    gtk_text_loading: bool,
    gtk_text_busy: bool,
    gtk_text_generation: u64,
    gtk_text_refresh_pending: bool,
    gtk_text_stream_refreshing: bool,

    // Privacy & Security
    privacy_loading: bool,
    privacy_busy: Option<(rmac_privacy::PortalResource, String)>,
    privacy_reset_confirmation: Option<rmac_privacy::PortalDecision>,
    privacy_generation: u64,
    privacy_refresh_pending: bool,
    privacy_stream_refreshing: bool,
    security_coverage_loading: bool,

    // Sound
    audio_loading: bool,
    audio_busy: bool,
    audio_generation: u64,
    audio_refresh_pending: bool,
    output_volume_generation: u64,
    input_volume_generation: u64,
    output_balance_generation: u64,
    output_volume: Entity<SliderState>,
    input_volume: Entity<SliderState>,
    output_balance: Entity<SliderState>,

    // Battery and power profiles
    power_loading: bool,
    power_busy: bool,
    power_generation: u64,
    power_refresh_pending: bool,

    // Displays
    display_loading: bool,
    display_busy: bool,
    display_generation: u64,
    display_refresh_pending: bool,
    display_confirmation: Option<DisplayConfirmation>,

    // Keyboard, mouse, and trackpad
    input_loading: bool,
    input_busy: bool,
    input_generation: u64,
    input_refresh_pending: bool,
    input_stream_refreshing: bool,
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

impl Settings {
    // ---- explicit shell-owned panes ----------------------------------

    // ---- Wi-Fi --------------------------------------------------------

    // ---- Bluetooth ----------------------------------------------------

    // ---- Date & Time -------------------------------------------------

    // ---- Language & Region -------------------------------------------

    // ---- Login Items -------------------------------------------------

    // ---- Sharing -----------------------------------------------------

    // ---- Accessibility -----------------------------------------------

    // ---- Privacy & Security -----------------------------------------

    // ---- Appearance ---------------------------------------------------

    // ---- Notifications ------------------------------------------------

    // ---- Focus --------------------------------------------------------

    // ---- Lock Screen --------------------------------------------------

    // ---- Sound --------------------------------------------------------

    // ---- Keyboard, mouse, and trackpad -------------------------------

    // ---- Battery and power profiles ----------------------------------

    // ---- Displays -----------------------------------------------------

    // ---- Network ------------------------------------------------------

    // ---- VPN ----------------------------------------------------------

    // ---- subpages -----------------------------------------------------
}

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
mod tests {
    use super::{
        audio_change_needs_followup, audio_stream_snapshot_is_current,
        bluetooth_stream_snapshot_is_current, composite_wallpaper_pixel,
        gtk_text_stream_snapshot_is_current, input_stream_snapshot_is_current,
        locale_stream_snapshot_is_current, login_items_stream_snapshot_is_current,
        network_stream_snapshot_is_current, power_change_needs_followup,
        power_stream_snapshot_is_current, privacy_stream_snapshot_is_current,
        render_wallpaper_preview, shortcut_configuration_available,
        storage_stream_snapshot_is_current, system_info_stream_snapshot_is_current,
        theme_stream_snapshot_is_current, time_stream_snapshot_is_current,
        update_stream_snapshot_is_current, vpn_stream_snapshot_is_current, wallpaper_selection,
        wifi_stream_snapshot_is_current, DockChange, ShellSettingsMutation, SpotlightAuthority,
        SpotlightChange, WallpaperChange, WallpaperTarget,
    };

    #[test]
    fn input_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(input_stream_snapshot_is_current(4, 4, false, false));
        assert!(!input_stream_snapshot_is_current(3, 4, false, false));
        assert!(!input_stream_snapshot_is_current(4, 4, true, false));
        assert!(!input_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn system_info_stream_snapshots_cannot_cross_hostname_transactions() {
        assert!(system_info_stream_snapshot_is_current(4, 4, false, false));
        assert!(!system_info_stream_snapshot_is_current(3, 4, false, false));
        assert!(!system_info_stream_snapshot_is_current(4, 4, true, false));
        assert!(!system_info_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn update_stream_snapshots_cannot_cross_install_transactions() {
        assert!(update_stream_snapshot_is_current(4, 4, false, false));
        assert!(!update_stream_snapshot_is_current(3, 4, false, false));
        assert!(!update_stream_snapshot_is_current(4, 4, true, false));
        assert!(!update_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn storage_stream_snapshots_cannot_cross_manual_refreshes() {
        assert!(storage_stream_snapshot_is_current(4, 4, false, false));
        assert!(!storage_stream_snapshot_is_current(3, 4, false, false));
        assert!(!storage_stream_snapshot_is_current(4, 4, true, false));
        assert!(!storage_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn time_stream_snapshots_cannot_cross_clock_transactions() {
        assert!(time_stream_snapshot_is_current(4, 4, false, false));
        assert!(!time_stream_snapshot_is_current(3, 4, false, false));
        assert!(!time_stream_snapshot_is_current(4, 4, true, false));
        assert!(!time_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn locale_stream_snapshots_cannot_cross_locale_transactions() {
        assert!(locale_stream_snapshot_is_current(4, 4, false, false));
        assert!(!locale_stream_snapshot_is_current(3, 4, false, false));
        assert!(!locale_stream_snapshot_is_current(4, 4, true, false));
        assert!(!locale_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn login_item_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(login_items_stream_snapshot_is_current(4, 4, false, false));
        assert!(!login_items_stream_snapshot_is_current(3, 4, false, false));
        assert!(!login_items_stream_snapshot_is_current(4, 4, true, false));
        assert!(!login_items_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn gtk_text_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(gtk_text_stream_snapshot_is_current(4, 4, false, false));
        assert!(!gtk_text_stream_snapshot_is_current(3, 4, false, false));
        assert!(!gtk_text_stream_snapshot_is_current(4, 4, true, false));
        assert!(!gtk_text_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn theme_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(theme_stream_snapshot_is_current(4, 4, false, false));
        assert!(!theme_stream_snapshot_is_current(3, 4, false, false));
        assert!(!theme_stream_snapshot_is_current(4, 4, true, false));
        assert!(!theme_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn privacy_stream_snapshots_cannot_cross_reset_generations() {
        assert!(privacy_stream_snapshot_is_current(4, 4, false, false));
        assert!(!privacy_stream_snapshot_is_current(3, 4, false, false));
        assert!(!privacy_stream_snapshot_is_current(4, 4, true, false));
        assert!(!privacy_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn wifi_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(wifi_stream_snapshot_is_current(7, 7, false, false));
        assert!(!wifi_stream_snapshot_is_current(6, 7, false, false));
        assert!(!wifi_stream_snapshot_is_current(7, 7, true, false));
        assert!(!wifi_stream_snapshot_is_current(7, 7, false, true));
    }

    #[test]
    fn bluetooth_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(bluetooth_stream_snapshot_is_current(11, 11, false, false));
        assert!(!bluetooth_stream_snapshot_is_current(10, 11, false, false));
        assert!(!bluetooth_stream_snapshot_is_current(11, 11, true, false));
        assert!(!bluetooth_stream_snapshot_is_current(11, 11, false, true));
    }

    #[test]
    fn network_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(network_stream_snapshot_is_current(5, 5, false, false));
        assert!(!network_stream_snapshot_is_current(4, 5, false, false));
        assert!(!network_stream_snapshot_is_current(5, 5, true, false));
        assert!(!network_stream_snapshot_is_current(5, 5, false, true));
    }

    #[test]
    fn vpn_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(vpn_stream_snapshot_is_current(3, 3, false, false));
        assert!(!vpn_stream_snapshot_is_current(2, 3, false, false));
        assert!(!vpn_stream_snapshot_is_current(3, 3, true, false));
        assert!(!vpn_stream_snapshot_is_current(3, 3, false, true));
    }

    #[test]
    fn power_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(power_stream_snapshot_is_current(9, 9, false, false));
        assert!(!power_stream_snapshot_is_current(8, 9, false, false));
        assert!(!power_stream_snapshot_is_current(9, 9, true, false));
        assert!(!power_stream_snapshot_is_current(9, 9, false, true));
    }

    #[test]
    fn audio_stream_snapshots_cannot_cross_mutation_generations() {
        assert!(audio_stream_snapshot_is_current(4, 4, false, false));
        assert!(!audio_stream_snapshot_is_current(3, 4, false, false));
        assert!(!audio_stream_snapshot_is_current(4, 4, true, false));
        assert!(!audio_stream_snapshot_is_current(4, 4, false, true));
    }

    #[test]
    fn audio_changes_retain_recovery_without_duplicating_initial_load() {
        assert!(!audio_change_needs_followup(false, true, false));
        assert!(audio_change_needs_followup(false, true, true));
        assert!(audio_change_needs_followup(true, false, false));
        assert!(!audio_change_needs_followup(false, false, true));
    }

    #[test]
    fn power_changes_retain_recovery_without_duplicating_initial_load() {
        assert!(!power_change_needs_followup(false, true, false));
        assert!(power_change_needs_followup(false, true, true));
        assert!(power_change_needs_followup(true, false, false));
        assert!(!power_change_needs_followup(false, false, true));
    }

    #[test]
    fn dock_changes_touch_only_the_selected_policy() {
        let original = rmac_shell_settings::DockSettings::default();

        let mut dock = original.clone();
        DockChange::Placement(rmac_shell_settings::DockPlacement::Left).apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                placement: rmac_shell_settings::DockPlacement::Left,
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::Outputs(rmac_shell_settings::OutputScope::Named("DP-1".into()))
            .apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                outputs: rmac_shell_settings::OutputScope::Named("DP-1".into()),
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::Autohide(true).apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                autohide: true,
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::Magnification(false).apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                magnification: false,
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::MagnificationScale(2.0).apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                magnification_scale: 2.0,
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::ReserveSpace(false).apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                reserve_space: false,
                ..original.clone()
            }
        );

        let mut dock = original.clone();
        DockChange::RepeatedClick(rmac_shell_settings::RepeatedClickBehavior::DoNothing)
            .apply(&mut dock);
        assert_eq!(
            dock,
            rmac_shell_settings::DockSettings {
                repeated_click: rmac_shell_settings::RepeatedClickBehavior::DoNothing,
                ..original
            }
        );
    }

    #[test]
    fn dock_mutation_preserves_unrelated_shell_settings() {
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.clock.show_seconds = true;
        settings.spotlight.include_removable_mounts = true;

        ShellSettingsMutation::Change(DockChange::Autohide(true)).apply(&mut settings);

        assert!(settings.dock.autohide);
        assert!(settings.clock.show_seconds);
        assert!(settings.spotlight.include_removable_mounts);
    }

    #[test]
    fn spotlight_provider_changes_are_scoped_and_elide_default_policy() {
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.dock.autohide = true;
        let original_wallpaper = settings.wallpaper.clone();
        let provider = rmac_launcher_providers::FILES_PROVIDER.to_string();

        SpotlightChange::ProviderPrivateContent {
            id: provider.clone(),
            allowed: true,
        }
        .apply(&mut settings);
        let policy = settings
            .providers
            .get(&rmac_shell_settings::ProviderId(provider.clone()))
            .unwrap();
        assert!(policy.enabled);
        assert!(policy.allow_private_content);
        assert!(!policy.allow_network);
        assert!(settings.dock.autohide);
        assert_eq!(settings.wallpaper, original_wallpaper);

        SpotlightChange::ProviderPrivateContent {
            id: provider.clone(),
            allowed: false,
        }
        .apply(&mut settings);
        assert!(!settings
            .providers
            .contains_key(&rmac_shell_settings::ProviderId(provider)));
    }

    #[test]
    fn spotlight_configuration_requires_the_live_version_two_portal() {
        assert!(shortcut_configuration_available(Some(
            &rmac_shortcuts::BackendStatus::Portal {
                version: 2,
                can_configure: true,
            }
        )));
        assert!(!shortcut_configuration_available(Some(
            &rmac_shortcuts::BackendStatus::Portal {
                version: 1,
                can_configure: true,
            }
        )));
        assert!(!shortcut_configuration_available(Some(
            &rmac_shortcuts::BackendStatus::Portal {
                version: 2,
                can_configure: false,
            }
        )));
        assert!(!shortcut_configuration_available(Some(
            &rmac_shortcuts::BackendStatus::FallbackRequired {
                reason: "portal unavailable".into(),
            }
        )));
        assert!(!shortcut_configuration_available(None));
    }

    #[test]
    fn spotlight_scope_and_rollback_preserve_unrelated_shell_settings() {
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.dock.reserve_space = false;
        settings.wallpaper.default.source = Some("builtin:rmac-aurora".into());
        let previous = SpotlightAuthority::from_settings(&settings);

        ShellSettingsMutation::Spotlight(SpotlightChange::IncludeRemovableMounts(true))
            .apply(&mut settings);
        ShellSettingsMutation::Spotlight(SpotlightChange::AddExclusion(
            "/home/test/Private".into(),
        ))
        .apply(&mut settings);
        ShellSettingsMutation::Spotlight(SpotlightChange::AddExclusion(
            "/home/test/Private".into(),
        ))
        .apply(&mut settings);
        assert!(settings.spotlight.include_removable_mounts);
        assert_eq!(settings.spotlight.excluded_paths, ["/home/test/Private"]);
        assert!(!settings.dock.reserve_space);
        assert_eq!(
            settings.wallpaper.default.source.as_deref(),
            Some("builtin:rmac-aurora")
        );

        ShellSettingsMutation::Spotlight(SpotlightChange::RemoveExclusion(
            "/home/test/Private".into(),
        ))
        .apply(&mut settings);
        assert!(settings.spotlight.excluded_paths.is_empty());

        ShellSettingsMutation::RestoreSpotlight(previous).apply(&mut settings);
        assert_eq!(
            settings.spotlight,
            rmac_shell_settings::SpotlightSettings::default()
        );
        assert!(settings.providers.is_empty());
        assert!(!settings.dock.reserve_space);
        assert_eq!(
            settings.wallpaper.default.source.as_deref(),
            Some("builtin:rmac-aurora")
        );
    }

    #[test]
    fn wallpaper_output_changes_clone_the_default_and_preserve_other_settings() {
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.dock.autohide = true;
        settings.wallpaper.default.source = Some("builtin:rmac-aurora".into());
        let original_default = settings.wallpaper.default.clone();

        ShellSettingsMutation::Wallpaper {
            target: WallpaperTarget::Output("DP-1".into()),
            change: WallpaperChange::Fit(rmac_shell_settings::WallpaperFit::Center),
        }
        .apply(&mut settings);

        assert_eq!(settings.wallpaper.default, original_default);
        assert_eq!(
            settings.wallpaper.per_output.get("DP-1"),
            Some(&rmac_shell_settings::WallpaperSelection {
                source: Some("builtin:rmac-aurora".into()),
                fit: rmac_shell_settings::WallpaperFit::Center,
            })
        );
        assert!(settings.dock.autohide);

        ShellSettingsMutation::Wallpaper {
            target: WallpaperTarget::Output("DP-1".into()),
            change: WallpaperChange::UseDefault,
        }
        .apply(&mut settings);
        assert!(!settings.wallpaper.per_output.contains_key("DP-1"));
    }

    #[test]
    fn wallpaper_selection_reports_inheritance_without_inventing_an_override() {
        let wallpaper = rmac_shell_settings::WallpaperSettings::default();
        let (selection, owns_selection) =
            wallpaper_selection(&wallpaper, &WallpaperTarget::Output("HDMI-A-1".into()));
        assert_eq!(selection, wallpaper.default);
        assert!(!owns_selection);
        assert!(wallpaper.per_output.is_empty());
    }

    #[test]
    fn original_wallpaper_preview_uses_the_bounded_renderer() {
        let preview =
            render_wallpaper_preview(&rmac_shell_settings::WallpaperSelection::default()).unwrap();
        assert_eq!(preview.size(0).width.0, 480);
        assert_eq!(preview.size(0).height.0, 270);
        assert_eq!(preview.as_bytes(0).unwrap().len(), 480 * 270 * 4);
    }

    #[test]
    fn wallpaper_preview_alpha_compositing_does_not_overflow() {
        assert_eq!(
            composite_wallpaper_pixel([255, 64, 0, 128], [0, 0, 32, 255]),
            [128, 32, 15, 255]
        );
        assert_eq!(
            composite_wallpaper_pixel([1, 2, 3, 0], [20, 30, 40, 255]),
            [20, 30, 40, 255]
        );
    }
}
