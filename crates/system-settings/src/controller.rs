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
mod software_updates;
mod sound;
mod spotlight;
mod storage;
mod system_info;
mod vpn;
mod wallpaper;
mod wifi;

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

    fn audio_balance_slider(cx: &mut Context<Self>, value: f32) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(-100.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event;
            this.schedule_audio_balance(value.start(), cx);
            cx.notify();
        })
        .detach();
        slider
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        let (catalog_events, catalog_event_rx) = async_channel::bounded(1);
        let app_catalog_watcher = rmac_apps::watch_catalog(move || {
            let _ = catalog_events.try_send(());
        })
        .ok();

        // System audio sliders write through the platform audio service.
        let output_volume = Self::audio_slider(cx, 0.0, rmac_audio::DeviceKind::Output);
        let input_volume = Self::audio_slider(cx, 0.0, rmac_audio::DeviceKind::Input);
        let output_balance = Self::audio_balance_slider(cx, 0.0);

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
                this.run_pending_system_info_refresh(cx);
                this.run_pending_storage_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let capability = cx
                .background_executor()
                .spawn(async { gather_screen_reader_capability() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.screen_reader = capability;
                this.screen_reader_loading = false;
                cx.notify();
            });
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let (system_info_updates, system_info_update_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_system_info::watch(system_info_updates).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = system_info_update_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_system_info::WatchEvent::Changed => {
                                    this.queue_system_info_stream_refresh(cx);
                                }
                                rmac_system_info::WatchEvent::Unavailable => {
                                    this.system_data_stream_error = Some(
                                        "Live hostname updates are temporarily unavailable".into(),
                                    );
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        #[cfg(target_os = "linux")]
        {
            let (storage_updates, storage_update_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_mounts::watch(storage_updates).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = storage_update_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_mounts::WatchEvent::Changed => {
                                    this.queue_storage_stream_refresh(cx);
                                }
                                rmac_mounts::WatchEvent::Unavailable => {
                                    this.storage_stream_error = Some(
                                        "Live mounted-volume updates are temporarily unavailable"
                                            .into(),
                                    );
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_sharing_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();

        let (privacy_updates, privacy_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_privacy_linux::watch(privacy_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = privacy_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_privacy::WatchEvent::Changed => {
                                this.queue_privacy_stream_refresh(cx);
                            }
                            rmac_privacy::WatchEvent::Unavailable => {
                                this.privacy_stream_error = Some(
                                    "Live portal permission updates are temporarily unavailable"
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (login_item_updates, login_item_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_login_items_linux::watch(login_item_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = login_item_update_rx.recv().await {
                match event {
                    rmac_login_items::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_login_items_stream_refresh(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_login_items::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.login_items_stream_error = Some(
                                    "Live Login Items updates are temporarily unavailable".into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (time_updates, time_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_time_linux::watch(time_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = time_update_rx.recv().await {
                match event {
                    rmac_time::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_time_stream_refresh(cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_time::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.time_stream_error = Some(
                                    "Live date and time updates are temporarily unavailable".into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (sharing_updates, sharing_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_sharing_linux::watch(sharing_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = sharing_update_rx.recv().await {
                match event {
                    rmac_sharing::WatchEvent::Changed => {
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_sharing_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if !this.sharing_busy {
                                    this.finish_sharing_update(result);
                                    this.sharing_stream_error = None;
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_sharing::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.sharing_stream_error =
                                    Some("Live Sharing updates are temporarily unavailable".into());
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        // Keep the visible clock current without putting System Settings on a
        // frame-rate loop. Two low-frequency wakeups per minute are enough for
        // the minute-resolution label and stop redrawing when another pane is
        // selected.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            async_io::Timer::after(std::time::Duration::from_secs(30)).await;
            if this
                .update(cx, |this: &mut Settings, cx| {
                    if this.nav.is_empty() && this.current().name.as_ref() == "Date & Time" {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let (update_events, update_event_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_updates_linux::watch(update_events).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = update_event_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_updates::WatchEvent::Changed => {
                                    this.queue_update_stream_refresh(cx);
                                }
                                rmac_updates::WatchEvent::Unavailable => {
                                    this.updates_stream_error = Some(
                                        "Live PackageKit updates are temporarily unavailable"
                                            .into(),
                                    );
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (locale_updates, locale_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_locale_linux::watch(locale_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = locale_update_rx.recv().await {
                match event {
                    rmac_locale::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_locale_stream_refresh(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_locale::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.locale_stream_error = Some(
                                    "Live language and region updates are temporarily unavailable"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
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

        let (bluetooth_updates, bluetooth_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_bluetooth::watch(bluetooth_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = bluetooth_update_rx.recv().await {
                match event {
                    rmac_bluetooth::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            this.bluetooth_stream_error = None;
                            cx.notify();
                            (!this.bluetooth_busy && !this.bluetooth_loading)
                                .then_some(this.bluetooth_generation)
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_bluetooth::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if bluetooth_stream_snapshot_is_current(
                                    generation,
                                    this.bluetooth_generation,
                                    this.bluetooth_busy,
                                    this.bluetooth_loading,
                                ) {
                                    this.finish_bluetooth_stream_update(result);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_bluetooth::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.bluetooth_stream_error = Some(
                                    "Live Bluetooth updates are temporarily unavailable while BlueZ reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (audio_updates, audio_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_audio::watch(audio_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = audio_update_rx.recv().await {
                match event {
                    rmac_audio::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            if this.audio_busy || this.audio_loading {
                                if audio_change_needs_followup(
                                    this.audio_busy,
                                    this.audio_loading,
                                    this.audio_stream_error.is_some(),
                                ) {
                                    this.audio_refresh_pending = true;
                                }
                                None
                            } else {
                                this.audio_stream_error = None;
                                cx.notify();
                                Some(this.audio_generation)
                            }
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_audio::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if audio_stream_snapshot_is_current(
                                    generation,
                                    this.audio_generation,
                                    this.audio_busy,
                                    this.audio_loading,
                                ) {
                                    this.finish_audio_stream_update(result, cx);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_audio::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.audio_stream_error = Some(
                                    "Live audio updates are temporarily unavailable while PipeWire reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (power_updates, power_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_power::watch(power_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = power_update_rx.recv().await {
                match event {
                    rmac_power::WatchEvent::Changed => {
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            if this.power_busy || this.power_loading {
                                if power_change_needs_followup(
                                    this.power_busy,
                                    this.power_loading,
                                    this.power_stream_error.is_some(),
                                ) {
                                    this.power_refresh_pending = true;
                                }
                                None
                            } else {
                                this.power_stream_error = None;
                                cx.notify();
                                Some(this.power_generation)
                            }
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_power::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if power_stream_snapshot_is_current(
                                    generation,
                                    this.power_generation,
                                    this.power_busy,
                                    this.power_loading,
                                ) {
                                    this.finish_power_stream_update(result);
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_power::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.power_stream_error = Some(
                                    "Live battery updates are temporarily unavailable while UPower reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let (wifi_updates, wifi_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_network::watch(wifi_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = wifi_update_rx.recv().await {
                match event {
                    rmac_network::WifiWatchEvent::Changed => {
                        let generations = match this.update(cx, |this: &mut Settings, cx| {
                            this.wifi_stream_error = None;
                            this.network_stream_error = None;
                            this.vpn_stream_error = None;
                            cx.notify();
                            (
                                (!this.wifi_busy && !this.wifi_loading)
                                    .then_some(this.wifi_generation),
                                (!this.network_busy && !this.network_loading)
                                    .then_some(this.network_generation),
                                (this.vpn_busy.is_none()
                                    && !this.vpn_loading
                                    && !this.vpn_refreshing
                                    && !this.vpn_import_busy
                                    && this.vpn_import_preview.is_none()
                                    && this.vpn_delete_preparing.is_none()
                                    && !this.vpn_delete_busy
                                    && this.vpn_delete_preview.is_none())
                                    .then_some(this.vpn_generation),
                            )
                        }) {
                            Ok(generations) => generations,
                            Err(_) => break,
                        };
                        if generations.0.is_none()
                            && generations.1.is_none()
                            && generations.2.is_none()
                        {
                            continue;
                        }
                        let results = cx
                            .background_executor()
                            .spawn(async move {
                                (
                                    generations.0.map(|_| rmac_network::snapshot()),
                                    generations.1.map(|_| rmac_network::network_snapshot()),
                                    generations.2.map(|_| rmac_network::vpn_snapshot()),
                                )
                            })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if let (Some(generation), Some(result)) =
                                    (generations.0, results.0)
                                {
                                    if wifi_stream_snapshot_is_current(
                                        generation,
                                        this.wifi_generation,
                                        this.wifi_busy,
                                        this.wifi_loading,
                                    ) {
                                        this.finish_wifi_stream_update(result);
                                    }
                                }
                                if let (Some(generation), Some(result)) =
                                    (generations.1, results.1)
                                {
                                    if network_stream_snapshot_is_current(
                                        generation,
                                        this.network_generation,
                                        this.network_busy,
                                        this.network_loading,
                                    ) {
                                        this.finish_network_stream_update(result);
                                    }
                                }
                                if let (Some(generation), Some(result)) =
                                    (generations.2, results.2)
                                {
                                    if vpn_stream_snapshot_is_current(
                                        generation,
                                        this.vpn_generation,
                                        this.vpn_busy.is_some()
                                            || this.vpn_refreshing
                                            || this.vpn_import_busy
                                            || this.vpn_import_preview.is_some()
                                            || this.vpn_editor_loading.is_some()
                                            || this.vpn_editor_busy
                                            || this.vpn_editor.is_some()
                                            || this.vpn_secret_preparing
                                            || this.vpn_secret_busy
                                            || this.vpn_secret_preview.is_some()
                                            || this.vpn_delete_preparing.is_some()
                                            || this.vpn_delete_busy
                                            || this.vpn_delete_preview.is_some(),
                                        this.vpn_loading,
                                    ) {
                                        this.finish_vpn_stream_update(result);
                                    }
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_network::WifiWatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.wifi_stream_error = Some(
                                    "Live Wi-Fi updates are temporarily unavailable while NetworkManager reconnects"
                                        .into(),
                                );
                                this.network_stream_error = Some(
                                    "Live Network updates are temporarily unavailable while NetworkManager reconnects"
                                        .into(),
                                );
                                this.vpn_stream_error = Some(
                                    "Live VPN updates are temporarily unavailable while NetworkManager reconnects"
                                        .into(),
                                );
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
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
            let capabilities = cx
                .background_executor()
                .spawn(async { rmac_network::vpn_import_capabilities() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_import_capabilities = capabilities;
                this.vpn_import_loading = false;
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
                this.finish_power_update(result, cx);
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
                this.flush_display_stream_refresh(cx);
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
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (gtk_text_updates, gtk_text_update_rx) = async_channel::bounded(2);
        std::thread::spawn(move || {
            let _ = rmac_gtk_settings::watch(gtk_text_updates);
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = gtk_text_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_gtk_settings::WatchEvent::Available => {
                                this.gtk_text_stream_error = None;
                            }
                            rmac_gtk_settings::WatchEvent::Changed => {
                                this.gtk_text_stream_error = None;
                                this.queue_gtk_text_stream_refresh(cx);
                            }
                            rmac_gtk_settings::WatchEvent::Unavailable => {
                                this.gtk_text_stream_error = Some(
                                    "Live GTK text-scale updates are temporarily unavailable"
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::security_coverage_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.security_coverage = Some(snapshot);
                this.security_coverage_loading = false;
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
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (theme_store_updates, theme_store_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                loop {
                    let watcher = match rmac_theme::ThemeStore::from_environment()
                        .and_then(|store| store.watch())
                    {
                        Ok(watcher) => watcher,
                        Err(_) => {
                            if theme_store_updates
                                .send(ThemeStoreWatchEvent::Unavailable)
                                .await
                                .is_err()
                            {
                                return;
                            }
                            async_io::Timer::after(Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    if theme_store_updates
                        .send(ThemeStoreWatchEvent::Available)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    loop {
                        match watcher.recv().await {
                            Ok(rmac_theme::StoreEvent::Changed) => {
                                if theme_store_updates
                                    .send(ThemeStoreWatchEvent::Changed)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Ok(rmac_theme::StoreEvent::WatchError(_)) | Err(_) => {
                                if theme_store_updates
                                    .send(ThemeStoreWatchEvent::Unavailable)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                    async_io::Timer::after(Duration::from_secs(1)).await;
                }
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = theme_store_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            ThemeStoreWatchEvent::Available => {
                                this.theme_store_stream_error = None;
                            }
                            ThemeStoreWatchEvent::Changed => {
                                this.theme_store_stream_error = None;
                                this.queue_theme_stream_refresh(cx);
                            }
                            ThemeStoreWatchEvent::Unavailable => {
                                this.theme_store_stream_error = Some(
                                    "Live rmac appearance updates are temporarily unavailable"
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (portal_appearance_updates, portal_appearance_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_appearance_portal::watch(portal_appearance_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = portal_appearance_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_appearance::Event::Snapshot(_) => {
                                this.theme_portal_stream_error = None;
                                this.queue_theme_stream_refresh(cx);
                            }
                            rmac_appearance::Event::Unavailable(_) => {
                                this.theme_portal_stream_error = Some(
                                    "Live desktop appearance updates are temporarily unavailable"
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
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

        let (shell_settings_updates, shell_settings_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(watch_shell_settings(shell_settings_updates))
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = shell_settings_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.apply_shell_settings_stream_update(update) {
                            this.refresh_wallpaper_preview(cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (dock_compositor_events, dock_compositor_event_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                loop {
                    let result = rmac_compositor_niri::watch(dock_compositor_events.clone()).await;
                    if dock_compositor_events.is_closed() {
                        return;
                    }
                    if result.is_err()
                        && dock_compositor_events
                            .send(rmac_compositor::Event::ConnectionChanged {
                                state: rmac_compositor::ConnectionState::Disconnected,
                            })
                            .await
                            .is_err()
                    {
                        return;
                    }
                    async_io::Timer::after(Duration::from_secs(1)).await;
                }
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = dock_compositor_event_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        let refresh_displays = compositor_event_affects_displays(&event);
                        let refresh_input = compositor_event_affects_input(&event);
                        let input_config_failed = compositor_input_config_failed(&event);
                        this.dock_compositor.apply(event);
                        if refresh_displays {
                            this.request_display_stream_refresh(cx);
                        }
                        if refresh_input {
                            this.request_input_stream_refresh(cx);
                        } else if input_config_failed == Some(true) {
                            this.input_error = Some(
                                "Could not update Input settings: niri rejected its latest configuration reload; the last known-good values remain visible."
                                    .into(),
                            );
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let (input_events, input_event_rx) = async_channel::bounded(2);
            cx.background_executor()
                .spawn(async move {
                    loop {
                        let result = rmac_input::watch(input_events.clone()).await;
                        if input_events.is_closed() {
                            return;
                        }
                        if let Err(error) = result {
                            if input_events
                                .send(rmac_input::WatchEvent::WatchError(error.to_string()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        async_io::Timer::after(Duration::from_secs(1)).await;
                    }
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = input_event_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_input::WatchEvent::Changed => {
                                    this.input_stream_error = None;
                                    this.request_input_stream_refresh(cx);
                                }
                                rmac_input::WatchEvent::WatchError(error) => {
                                    this.input_stream_error = Some(
                                        format!(
                                            "Live input-device updates are unavailable: {error}"
                                        )
                                        .into(),
                                    );
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_shortcuts::backend_status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shortcut_status_update(result);
                cx.notify();
            });
        })
        .detach();

        let sections = categories();
        let selected = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|arguments| {
                (arguments[0] == "--pane")
                    .then(|| category_name_for_pane_id(&arguments[1]))
                    .flatten()
            })
            .and_then(|category| category_position(&sections, category))
            .unwrap_or((1, 0));

        Self {
            system_data_loading: true,
            system_data_busy: false,
            system_data_error: None,
            system_data_stream_error: None,
            system_data_generation: 0,
            system_data_refresh_pending: false,
            system_data_stream_refreshing: false,
            hostname_editor: None,
            diagnostics_copied: false,
            account: std::env::var("USER")
                .unwrap_or_else(|_| "User".into())
                .into(),
            sysinfo: rmac_system_info::Snapshot::default(),
            screen_reader: ScreenReaderCapability::default(),
            screen_reader_loading: true,
            updates_loading: true,
            updates_busy: false,
            updates_error: None,
            updates_stream_error: None,
            updates_generation: 0,
            updates_refresh_pending: false,
            updates_stream_refreshing: false,
            updates_preparing: false,
            updates_installing: false,
            updates_cancellation: None,
            updates_plan: None,
            updates_progress: None,
            updates_result: None,
            updates: None,
            time_loading: true,
            time_busy: false,
            time_error: None,
            time_stream_error: None,
            time_generation: 0,
            time_refresh_pending: false,
            time_stream_refreshing: false,
            clock_editor: None,
            clock_confirmation: None,
            clock_setting: false,
            time: None,
            timezone_editor: None,
            locale_loading: true,
            locale_busy: false,
            locale_error: None,
            locale_stream_error: None,
            locale_generation: 0,
            locale_refresh_pending: false,
            locale_stream_refreshing: false,
            locale: None,
            locale_editor: None,
            region_editor: None,
            locale_revert: None,
            x11_layout_editor: None,
            x11_variant_editor: None,
            x11_options_editor: None,
            x11_keyboard_revert: None,
            login_items_loading: true,
            login_item_busy: None,
            login_items_error: None,
            login_items_stream_error: None,
            login_items_generation: 0,
            login_items_refresh_pending: false,
            login_items_stream_refreshing: false,
            login_items: None,
            login_item_add: None,
            login_item_remove: None,
            sharing_loading: true,
            sharing_busy: false,
            sharing_error: None,
            sharing_stream_error: None,
            sharing: None,
            sharing_confirmation: None,
            file_sharing_confirmation: None,
            power: rmac_power::Snapshot::default(),
            display: rmac_display::Snapshot::default(),
            network: rmac_network::NetworkSnapshot::default(),
            storage: Vec::new(),
            storage_busy: false,
            storage_error: None,
            storage_stream_error: None,
            storage_generation: 0,
            storage_refresh_pending: false,
            storage_stream_refreshing: false,
            storage_action_busy: None,
            audio: rmac_audio::Snapshot::default(),
            input: rmac_input::Snapshot::default(),
            gtk_text: None,
            privacy: None,
            security_coverage: None,
            sections,
            selected,
            nav: Vec::new(),
            search,
            focus: cx.focus_handle(),
            focused_once: false,
            dragging: false,
            wifi_error: None,
            wifi_stream_error: None,
            bluetooth_error: None,
            bluetooth_stream_error: None,
            network_error: None,
            network_stream_error: None,
            vpn_error: None,
            vpn_stream_error: None,
            audio_error: None,
            audio_stream_error: None,
            power_error: None,
            power_stream_error: None,
            display_error: None,
            input_error: None,
            input_stream_error: None,
            theme_error: None,
            theme_store_stream_error: None,
            theme_portal_stream_error: None,
            shell_settings_error: None,
            shell_settings_stream_error: None,
            gtk_text_error: None,
            gtk_text_stream_error: None,
            privacy_error: None,
            privacy_stream_error: None,
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
            lock_request_busy: false,
            lock_request_error: None,

            shell_settings_loading: true,
            shell_settings_busy: false,
            shell_settings: None,
            shell_settings_revert: None,
            dock_compositor: rmac_compositor::State::default(),
            wallpaper_target: WallpaperTarget::Default,
            wallpaper_revert: None,
            wallpaper_error: None,
            wallpaper_preview: None,
            wallpaper_preview_loading: true,
            wallpaper_preview_error: None,
            wallpaper_preview_watch_error: None,
            wallpaper_preview_generation: 0,
            _wallpaper_preview_watcher: None,
            spotlight_revert: None,
            spotlight_error: None,
            recent_history_confirmation: false,
            recent_history_busy: false,
            recent_history_notice: None,
            shortcut_status_loading: true,
            shortcut_status: None,
            shortcut_status_error: None,
            shortcut_configuration_busy: false,
            shortcut_configuration_error: None,

            network_loading: true,
            network_busy: false,
            network_generation: 0,
            network_editor: None,

            vpn: rmac_network::VpnSnapshot::default(),
            vpn_loading: true,
            vpn_refreshing: false,
            vpn_busy: None,
            vpn_cancellation: None,
            vpn_generation: 0,
            vpn_import_capabilities: rmac_network::VpnImportCapabilities::default(),
            vpn_import_loading: true,
            vpn_import_busy: false,
            vpn_import_preview: None,
            vpn_editor_loading: None,
            vpn_editor_busy: false,
            vpn_editor: None,
            vpn_secret_preparing: false,
            vpn_secret_busy: false,
            vpn_secret_preview: None,
            vpn_delete_preparing: None,
            vpn_delete_busy: false,
            vpn_delete_preview: None,

            wifi_available: false,
            wifi_loading: true,
            wifi_busy: false,
            wifi_generation: 0,
            wifi_connecting: None,
            wifi_forgetting: None,
            wifi_forget_confirmation: None,
            wifi_password_prompt: None,
            wifi_enterprise_prompt: None,
            wifi_cancellation: None,
            wifi_on: false,
            wifi_interface: None,
            wifi_networks: Vec::new(),
            wifi_saved_networks: Vec::new(),

            bluetooth_available: false,
            bluetooth_loading: true,
            bluetooth_busy: false,
            bluetooth_generation: 0,
            bluetooth_discovering: false,
            bluetooth_adapter_name: None,
            bluetooth_on: false,
            bt_discoverable: false,
            bt_devices: Vec::new(),
            bluetooth_pairing: None,
            bluetooth_forget_confirmation: None,
            bluetooth_forgetting: None,

            host_appearance: rmac_appearance::Snapshot::default(),
            theme: None,
            theme_loading: true,
            theme_busy: false,
            theme_generation: 0,
            theme_refresh_pending: false,
            theme_stream_refreshing: false,
            gtk_text_loading: true,
            gtk_text_busy: false,
            gtk_text_generation: 0,
            gtk_text_refresh_pending: false,
            gtk_text_stream_refreshing: false,

            privacy_loading: true,
            privacy_busy: None,
            privacy_reset_confirmation: None,
            privacy_generation: 0,
            privacy_refresh_pending: false,
            privacy_stream_refreshing: false,
            security_coverage_loading: true,

            audio_loading: true,
            audio_busy: false,
            audio_generation: 0,
            audio_refresh_pending: false,
            output_volume_generation: 0,
            input_volume_generation: 0,
            output_balance_generation: 0,
            output_volume,
            input_volume,
            output_balance,

            power_loading: true,
            power_busy: false,
            power_generation: 0,
            power_refresh_pending: false,

            display_loading: true,
            display_busy: false,
            display_generation: 0,
            display_refresh_pending: false,
            display_confirmation: None,

            input_loading: true,
            input_busy: false,
            input_generation: 0,
            input_refresh_pending: false,
            input_stream_refreshing: false,
        }
    }

    fn apply_shell_settings_stream_update(&mut self, update: ShellSettingsStreamUpdate) -> bool {
        self.shell_settings_loading = false;
        match update {
            ShellSettingsStreamUpdate::Snapshot(snapshot) => {
                self.shell_settings_stream_error = None;
                if self.shell_settings_busy {
                    return false;
                }
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                let spotlight_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                });
                if self
                    .shell_settings
                    .as_ref()
                    .is_some_and(|current| current.settings.dock != snapshot.settings.dock)
                {
                    self.shell_settings_revert = None;
                }
                if wallpaper_changed {
                    self.wallpaper_revert = None;
                }
                if spotlight_changed {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(*snapshot);
                self.shell_settings_error = None;
                wallpaper_changed
            }
            ShellSettingsStreamUpdate::Unavailable(error) => {
                self.shell_settings_stream_error =
                    Some(format!("Live shell settings updates are unavailable: {error}").into());
                false
            }
        }
    }

    fn finish_shell_settings_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<rmac_shell_settings::DockSettings>,
    ) -> bool {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
                });
                let spotlight_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                });
                if wallpaper_changed {
                    self.wallpaper_revert = None;
                }
                if spotlight_changed {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.shell_settings_revert = previous;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
                wallpaper_changed
            }
            Err(error) => {
                self.shell_settings_error =
                    Some(format!("Could not update Desktop & Dock: {error}").into());
                false
            }
        }
    }

    fn refresh_shell_settings(&mut self, refresh_wallpaper_preview: bool, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        self.shell_settings_loading = true;
        self.shell_settings_error = None;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(|| {
                rmac_shell_settings::ShellSettingsStore::from_environment()?.load()
            })
            .await;
            let _ =
                this.update(cx, |this: &mut Settings, cx| {
                    this.shell_settings_loading = false;
                    match result {
                        Ok(snapshot) => {
                            let wallpaper_changed =
                                this.shell_settings.as_ref().is_none_or(|current| {
                                    current.settings.wallpaper != snapshot.settings.wallpaper
                                });
                            let spotlight_changed =
                                this.shell_settings.as_ref().is_none_or(|current| {
                                    SpotlightAuthority::from_settings(&current.settings)
                                        != SpotlightAuthority::from_settings(&snapshot.settings)
                                });
                            if this.shell_settings.as_ref().is_some_and(|current| {
                                current.settings.dock != snapshot.settings.dock
                            }) {
                                this.shell_settings_revert = None;
                            }
                            if wallpaper_changed {
                                this.wallpaper_revert = None;
                            }
                            if spotlight_changed {
                                this.spotlight_revert = None;
                            }
                            this.shell_settings = Some(snapshot);
                            this.shell_settings_error = None;
                            this.shell_settings_stream_error = None;
                            if wallpaper_changed || refresh_wallpaper_preview {
                                this.refresh_wallpaper_preview(cx);
                            }
                        }
                        Err(error) => {
                            this.shell_settings_error =
                                Some(format!("Could not refresh shell settings: {error}").into());
                        }
                    }
                    cx.notify();
                });
        })
        .detach();
    }

    fn apply_dock_change(&mut self, change: DockChange, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = snapshot.settings.dock.clone();
        let mut next = previous.clone();
        change.clone().apply(&mut next);
        if next == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Change(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_shell_settings_mutation(result, Some(previous)) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_dock_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.shell_settings_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.shell_settings_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Restore(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_shell_settings_mutation(result, None) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_gtk_text_update(
        &mut self,
        result: std::result::Result<rmac_gtk_settings::Snapshot, rmac_gtk_settings::Error>,
    ) {
        self.gtk_text_loading = false;
        self.gtk_text_busy = false;
        match result {
            Ok(snapshot) => {
                self.gtk_text = Some(snapshot);
                self.gtk_text_error = None;
            }
            Err(error) => {
                self.gtk_text_error =
                    Some(format!("Could not update GTK text scaling: {error}").into());
            }
        }
    }

    fn queue_gtk_text_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            self.gtk_text_refresh_pending = true;
            return;
        }
        self.gtk_text_refresh_pending = false;
        self.gtk_text_stream_refreshing = true;
        let generation = self.gtk_text_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.gtk_text_stream_refreshing = false;
                if gtk_text_stream_snapshot_is_current(
                    generation,
                    this.gtk_text_generation,
                    this.gtk_text_loading,
                    this.gtk_text_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.gtk_text = Some(snapshot);
                            this.gtk_text_error = None;
                        }
                        Err(_) => {
                            this.gtk_text_error =
                                Some("Could not refresh changed GTK text scaling".into());
                        }
                    }
                } else {
                    this.gtk_text_refresh_pending = true;
                }
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn run_pending_gtk_text_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_refresh_pending
            && !self.gtk_text_loading
            && !self.gtk_text_busy
            && !self.gtk_text_stream_refreshing
        {
            self.queue_gtk_text_stream_refresh(cx);
        }
    }

    fn refresh_gtk_text(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_gtk_text_scale(&mut self, factor: f64, cx: &mut Context<Self>) {
        if self.gtk_text_loading
            || self.gtk_text_busy
            || self.gtk_text_stream_refreshing
            || !self
                .gtk_text
                .as_ref()
                .is_some_and(|snapshot| snapshot.available && snapshot.writable)
        {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_gtk_settings::set_text_scale(factor) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

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

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus);
        }
        let settings_error = self
            .system_data_error
            .clone()
            .or_else(|| self.system_data_stream_error.clone())
            .or_else(|| self.updates_error.clone())
            .or_else(|| self.updates_stream_error.clone())
            .or_else(|| self.storage_error.clone())
            .or_else(|| self.storage_stream_error.clone())
            .or_else(|| self.time_error.clone())
            .or_else(|| self.time_stream_error.clone())
            .or_else(|| self.locale_error.clone())
            .or_else(|| self.locale_stream_error.clone())
            .or_else(|| self.login_items_error.clone())
            .or_else(|| self.login_items_stream_error.clone())
            .or_else(|| self.sharing_error.clone())
            .or_else(|| self.sharing_stream_error.clone())
            .or_else(|| self.wifi_error.clone())
            .or_else(|| self.wifi_stream_error.clone())
            .or_else(|| self.bluetooth_error.clone())
            .or_else(|| self.bluetooth_stream_error.clone())
            .or_else(|| self.network_error.clone())
            .or_else(|| self.network_stream_error.clone())
            .or_else(|| self.vpn_error.clone())
            .or_else(|| self.vpn_stream_error.clone())
            .or_else(|| self.audio_error.clone())
            .or_else(|| self.audio_stream_error.clone())
            .or_else(|| self.power_error.clone())
            .or_else(|| self.power_stream_error.clone())
            .or_else(|| self.display_error.clone())
            .or_else(|| self.input_error.clone())
            .or_else(|| self.input_stream_error.clone())
            .or_else(|| self.theme_error.clone())
            .or_else(|| self.theme_store_stream_error.clone())
            .or_else(|| self.theme_portal_stream_error.clone())
            .or_else(|| self.shell_settings_error.clone())
            .or_else(|| self.shell_settings_stream_error.clone())
            .or_else(|| self.wallpaper_error.clone())
            .or_else(|| self.spotlight_error.clone())
            .or_else(|| self.gtk_text_error.clone())
            .or_else(|| self.gtk_text_stream_error.clone())
            .or_else(|| self.privacy_error.clone())
            .or_else(|| self.privacy_stream_error.clone());
        let wifi_password_dialog = self.render_wifi_password_dialog(cx);
        let wifi_enterprise_dialog = self.render_wifi_enterprise_dialog(cx);
        let wifi_forget_dialog = self.render_wifi_forget_dialog(cx);
        let bluetooth_pairing_dialog = self.render_bluetooth_pairing_dialog(cx);
        let bluetooth_forget_dialog = self.render_bluetooth_forget_dialog(cx);
        let clock_confirmation_dialog = self.render_clock_confirmation(cx);
        let vpn_import_dialog = self.render_vpn_import_dialog(cx);
        let vpn_secret_clear_dialog = self.render_vpn_secret_clear_dialog(cx);
        let vpn_delete_dialog = self.render_vpn_delete_dialog(cx);
        let update_install_dialog = self.render_update_install_dialog(cx);
        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter"
                    && this.clock_confirmation.is_some()
                    && !this.clock_setting
                {
                    cx.stop_propagation();
                    this.confirm_clock_change(cx);
                } else if event.keystroke.key == "escape" && this.clock_confirmation.is_some() {
                    cx.stop_propagation();
                    this.cancel_clock_confirmation(cx);
                } else if event.keystroke.key == "escape"
                    && this.recent_history_confirmation
                    && !this.recent_history_busy
                {
                    cx.stop_propagation();
                    this.cancel_recent_history_clear(cx);
                } else if event.keystroke.key == "escape" && this.updates_plan.is_some() {
                    cx.stop_propagation();
                    this.cancel_update_plan(cx);
                } else if event.keystroke.key == "escape" && this.wifi_forget_confirmation.is_some()
                {
                    cx.stop_propagation();
                    this.cancel_wifi_forget(cx);
                } else if event.keystroke.key == "escape"
                    && this.bluetooth_forget_confirmation.is_some()
                {
                    cx.stop_propagation();
                    this.cancel_bluetooth_forget(cx);
                } else if event.keystroke.key == "escape" && this.vpn_cancellation.is_some() {
                    cx.stop_propagation();
                    this.cancel_vpn_activation(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_import_preview.is_some()
                    && !this.vpn_import_busy
                {
                    cx.stop_propagation();
                    this.finish_vpn_import(false, cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_secret_preview.is_some()
                    && !this.vpn_secret_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_secret_clear(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_delete_preview.is_some()
                    && !this.vpn_delete_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_delete(cx);
                } else if event.keystroke.key == "escape"
                    && this.vpn_editor.is_some()
                    && !this.vpn_editor_busy
                {
                    cx.stop_propagation();
                    this.cancel_vpn_edit(cx);
                } else if event.keystroke.key == "escape"
                    && this.network_editor.is_some()
                    && !this.network_busy
                {
                    cx.stop_propagation();
                    this.cancel_network_edit(cx);
                }
            }))
            .on_action(cx.listener(|t, _: &GoBack, _, cx| t.go_back(cx)))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                if this.clock_setting {
                    return;
                }
                if this.recent_history_busy {
                    return;
                }
                if this.recent_history_confirmation {
                    this.cancel_recent_history_clear(cx);
                    return;
                }
                if this.clock_confirmation.take().is_some() {
                    cx.notify();
                    return;
                }
                if this.clock_editor.take().is_some() {
                    this.time_error = None;
                    cx.notify();
                    return;
                }
                if this.updates_installing {
                    return;
                }
                if this.updates_preparing {
                    this.cancel_update_operation(cx);
                    return;
                }
                if this.updates_plan.take().is_some() {
                    cx.notify();
                    return;
                }
                if let Some(cancellation) = &this.vpn_cancellation {
                    cancellation.cancel();
                    return;
                }
                if this.vpn_import_preview.is_some() {
                    if !this.vpn_import_busy {
                        this.finish_vpn_import(false, cx);
                    }
                    return;
                }
                if this.vpn_import_busy {
                    return;
                }
                if this.vpn_delete_preparing.is_some() || this.vpn_delete_busy {
                    return;
                }
                if this.vpn_delete_preview.take().is_some() {
                    window.remove_window();
                    return;
                }
                if this.vpn_secret_preparing || this.vpn_secret_busy {
                    return;
                }
                if this.vpn_secret_preview.take().is_some() {
                    window.remove_window();
                    return;
                }
                if this.wifi_forgetting.is_some()
                    || this.bluetooth_forgetting.is_some()
                    || this.network_busy
                    || this.vpn_editor_busy
                {
                    return;
                }
                if this.vpn_busy.is_some() {
                    return;
                }
                if let Some(cancellation) = &this.wifi_cancellation {
                    cancellation.cancel();
                }
                if let Some(pairing) = &this.bluetooth_pairing {
                    pairing.session.cancel();
                }
                window.remove_window();
            }))
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
                            this.system_data_error = None;
                            this.system_data_stream_error = None;
                            this.updates_error = None;
                            this.updates_stream_error = None;
                            this.storage_error = None;
                            this.storage_stream_error = None;
                            this.time_error = None;
                            this.time_stream_error = None;
                            this.locale_error = None;
                            this.locale_stream_error = None;
                            this.login_items_error = None;
                            this.login_items_stream_error = None;
                            this.sharing_error = None;
                            this.sharing_stream_error = None;
                            this.wifi_error = None;
                            this.wifi_stream_error = None;
                            this.bluetooth_error = None;
                            this.bluetooth_stream_error = None;
                            this.network_error = None;
                            this.network_stream_error = None;
                            this.vpn_error = None;
                            this.vpn_stream_error = None;
                            this.audio_error = None;
                            this.audio_stream_error = None;
                            this.power_error = None;
                            this.power_stream_error = None;
                            this.display_error = None;
                            this.input_error = None;
                            this.input_stream_error = None;
                            this.theme_error = None;
                            this.theme_store_stream_error = None;
                            this.theme_portal_stream_error = None;
                            this.shell_settings_error = None;
                            this.shell_settings_stream_error = None;
                            this.wallpaper_error = None;
                            this.spotlight_error = None;
                            this.gtk_text_error = None;
                            this.gtk_text_stream_error = None;
                            this.privacy_error = None;
                            this.privacy_stream_error = None;
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
            .when_some(wifi_password_dialog, |root, dialog| root.child(dialog))
            .when_some(wifi_enterprise_dialog, |root, dialog| root.child(dialog))
            .when_some(wifi_forget_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_pairing_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_forget_dialog, |root, dialog| root.child(dialog))
            .when_some(clock_confirmation_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_import_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_secret_clear_dialog, |root, dialog| root.child(dialog))
            .when_some(vpn_delete_dialog, |root, dialog| root.child(dialog))
            .when_some(update_install_dialog, |root, dialog| root.child(dialog))
    }
}

const SIDEBAR_W: f32 = 248.0;

// ---- row / control builders ----------------------------------------------

fn network_ip_method_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    selected: &rmac_network::IpMethod,
    enabled: bool,
) -> AnyElement {
    let options = match family {
        rmac_network::IpFamily::V4 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
        rmac_network::IpFamily::V6 => vec![
            ("Automatic", rmac_network::IpMethod::Automatic),
            ("DHCP", rmac_network::IpMethod::Dhcp),
            ("Manual", rmac_network::IpMethod::Manual),
            ("Link-Local", rmac_network::IpMethod::LinkLocal),
            ("Off", rmac_network::IpMethod::Disabled),
        ],
    };
    let mut control = div().flex().gap_1().w(px(390.0));
    for (index, (label, method)) in options.into_iter().enumerate() {
        let method_view = view.clone();
        let chosen = method.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!(
                    "network-{}-method-{index}",
                    match family {
                        rmac_network::IpFamily::V4 => "ipv4",
                        rmac_network::IpFamily::V6 => "ipv6",
                    }
                ))),
                label,
            )
            .flex_1()
            .selected(*selected == method)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                method_view.update(cx, |settings, cx| {
                    settings.set_network_ip_method(family, chosen.clone(), window, cx)
                });
            }),
        );
    }
    row_base()
        .child(text_block(
            "Configure".into(),
            Some("Choose how this connection receives addresses".into()),
        ))
        .child(control)
        .into_any_element()
}

fn network_field_row(
    title: &'static str,
    subtitle: &'static str,
    editor: &Entity<InputState>,
    enabled: bool,
) -> AnyElement {
    row_base()
        .child(text_block(title.into(), Some(subtitle.into())))
        .child(
            div()
                .w(px(390.0))
                .child(TextField::new(editor).small().disabled(!enabled)),
        )
        .into_any_element()
}

fn network_dns_policy_row(
    view: Entity<Settings>,
    family: rmac_network::IpFamily,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let id = match family {
        rmac_network::IpFamily::V4 => "network-ipv4-ignore-auto-dns",
        rmac_network::IpFamily::V6 => "network-ipv6-ignore-auto-dns",
    };
    row_base()
        .child(text_block(
            "Use only these DNS servers".into(),
            Some("Ignore DNS supplied automatically by the network".into()),
        ))
        .child(
            Toggle::new(id)
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_ignore_auto_dns(family, *value, cx)
                    });
                }),
        )
        .into_any_element()
}

fn network_proxy_method_row(
    view: Entity<Settings>,
    selected: rmac_network::ProxyMethod,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(390.0));
    for (index, (label, method)) in [
        ("Off", rmac_network::ProxyMethod::None),
        ("Automatic", rmac_network::ProxyMethod::Automatic),
    ]
    .into_iter()
    .enumerate()
    {
        let method_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("network-proxy-method-{index}"))),
                label,
            )
            .flex_1()
            .selected(selected == method)
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                method_view.update(cx, |settings, cx| {
                    settings.set_network_proxy_method(method, window, cx)
                });
            }),
        );
    }
    row_base()
        .child(text_block(
            "Configure".into(),
            Some("Use a proxy auto-configuration source".into()),
        ))
        .child(control)
        .into_any_element()
}

fn network_proxy_browser_row(view: Entity<Settings>, checked: bool, enabled: bool) -> AnyElement {
    row_base()
        .child(text_block(
            "Web browsers only".into(),
            Some("Other applications may ignore this proxy configuration".into()),
        ))
        .child(
            Toggle::new("network-proxy-browser-only")
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, _, cx| {
                    view.update(cx, |settings, cx| {
                        settings.set_network_proxy_browser_only(*value, cx)
                    });
                }),
        )
        .into_any_element()
}

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
    let mut b = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(s) = sub {
        b = b.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary())
                .child(s),
        );
    }
    b
}

/// A plain card-section label row (no control).
fn label_row(title: &'static str, value: Option<SharedString>) -> Div {
    let mut r = row_base().child(
        div()
            .flex_1()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(v) = value {
        r = r.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
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
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

fn locale_format(snapshot: &rmac_locale::Snapshot, key: &str) -> String {
    snapshot.effective_format_locale(key).to_owned()
}

fn locale_preview_row(
    icon: &'static str,
    title: &'static str,
    source: String,
    example: Option<&str>,
) -> AnyElement {
    row_base()
        .child(tile(icon, secondary(), 22.0))
        .child(text_block(
            title.into(),
            Some(format!("Locale: {source}").into()),
        ))
        .child(
            div()
                .max_w(px(260.0))
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(example.unwrap_or("Uses locale convention").to_owned()),
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
                .text_size(rmac_ui::text_px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
        )
}

/// A section header above a card (gray small caps-ish title).
fn section_header(title: impl Into<SharedString>) -> Div {
    div()
        .px_1()
        .pt_2()
        .pb_1()
        .text_size(rmac_ui::text_px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .text_color(secondary())
        .child(title.into())
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
                .text_size(rmac_ui::text_px(13.0))
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
                .text_size(rmac_ui::text_px(12.0))
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
) -> Stateful<Div> {
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
        .text_size(rmac_ui::text_px(12.0))
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
        .text_size(rmac_ui::text_px(18.0))
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
        .text_size(rmac_ui::text_px(18.0))
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
                        .text_size(rmac_ui::text_px(12.0))
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

fn bluetooth_device_row(
    view: &Entity<Settings>,
    device: &rmac_bluetooth::Device,
    busy: bool,
    forgetting: bool,
) -> AnyElement {
    let mut details = Vec::new();
    if !device.kind.is_empty() {
        details.push(device.kind.clone());
    }
    if !device.address.is_empty() {
        details.push(device.address.clone());
    }
    if device.paired {
        details.push(if device.trusted {
            "Trusted".into()
        } else {
            "Not trusted".into()
        });
    }
    let subtitle = (!details.is_empty()).then(|| details.join(" · ").into());
    let connect_id = device.id.clone();
    let pair_id = device.id.clone();
    let pair_name = SharedString::from(device.name.clone());
    let connect = !device.connected;
    let connect_view = view.clone();
    let pair_view = view.clone();
    let forget_id = device.id.clone();
    let forget_name = SharedString::from(device.name.clone());
    let forget_view = view.clone();
    let action = if device.connected {
        "Disconnect"
    } else if device.paired {
        "Connect"
    } else {
        "Pair"
    };
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
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(
                        SharedString::from(format!("bluetooth-device-action-{connect_id}")),
                        action,
                    )
                    .xsmall()
                    .disabled(busy)
                    .when(device.paired, |button| {
                        button.on_click(move |_, _, cx| {
                            connect_view.update(cx, |settings, cx| {
                                settings.set_bluetooth_device_connected(
                                    connect_id.clone(),
                                    connect,
                                    cx,
                                );
                            });
                        })
                    })
                    .when(!device.paired, |button| {
                        button.on_click(move |_, window, cx| {
                            pair_view.update(cx, |settings, cx| {
                                settings.begin_bluetooth_pairing(
                                    pair_id.clone(),
                                    pair_name.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                    }),
                )
                .when(device.paired, |actions| {
                    actions.child(
                        Button::new(
                            SharedString::from(format!("bluetooth-device-forget-{forget_id}")),
                            "Forget…",
                        )
                        .xsmall()
                        .busy(forgetting)
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            forget_view.update(cx, |settings, cx| {
                                settings.request_bluetooth_forget(
                                    forget_id.clone(),
                                    forget_name.clone(),
                                    cx,
                                );
                            });
                        }),
                    )
                }),
        )
        .into_any_element()
}

fn display_layout_preview(outputs: &[rmac_display::Output]) -> Div {
    let enabled = outputs
        .iter()
        .filter_map(|output| Some((output, output.logical.as_ref()?)))
        .collect::<Vec<_>>();
    let Some(min_x) = enabled.iter().map(|(_, logical)| logical.x).min() else {
        return note_card("No enabled display layout is available.");
    };
    let min_y = enabled
        .iter()
        .map(|(_, logical)| logical.y)
        .min()
        .unwrap_or_default();
    let max_x = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.x) + i64::from(logical.width))
        .max()
        .unwrap_or(1);
    let max_y = enabled
        .iter()
        .map(|(_, logical)| i64::from(logical.y) + i64::from(logical.height))
        .max()
        .unwrap_or(1);
    let span_x = (max_x - i64::from(min_x)).max(1) as f32;
    let span_y = (max_y - i64::from(min_y)).max(1) as f32;
    let scale = (440.0 / span_x).min(130.0 / span_y);
    let mut canvas = div()
        .relative()
        .w_full()
        .h(px(150.0))
        .rounded(px(9.0))
        .bg(rmac_ui::mac::control_fill())
        .border_1()
        .border_color(sep())
        .overflow_hidden();
    for (index, (output, logical)) in enabled.into_iter().enumerate() {
        let left = 10.0 + (logical.x - min_x) as f32 * scale;
        let top = 10.0 + (logical.y - min_y) as f32 * scale;
        let width = (logical.width as f32 * scale).max(24.0);
        let height = (logical.height as f32 * scale).max(18.0);
        let title = if output.primary {
            format!("{} · Main", index + 1)
        } else {
            format!("{} · {}", index + 1, output.name)
        };
        canvas = canvas.child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .flex()
                .items_center()
                .justify_center()
                .px_1()
                .rounded(px(5.0))
                .border_2()
                .border_color(if output.primary { accent() } else { sep() })
                .bg(if output.primary { accent() } else { card_bg() })
                .text_size(rmac_ui::text_px(11.0))
                .text_color(if output.primary {
                    hsl(0xffffff)
                } else {
                    label()
                })
                .overflow_hidden()
                .child(title),
        );
    }
    div()
        .p_2()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(canvas)
}

/// A slider row (state held in its own SliderState entity).
fn slider_row(title: &'static str, state: &Entity<SliderState>, value: SharedString) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(div().flex_1().child(Slider::new(state).w_full()))
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(value),
        )
}

fn balance_slider_row(state: &Entity<SliderState>) -> Div {
    row_base()
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("Balance"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("L"),
                )
                .child(div().flex_1().child(Slider::new(state).w_full()))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child("R"),
                ),
        )
        .child(div().w(px(44.0)).flex_none())
}

type GtkTextScaleOption = (&'static str, f64);

const GTK_TEXT_SCALE_OPTIONS: [GtkTextScaleOption; 3] =
    [("Standard", 1.0), ("Large", 1.2), ("Extra Large", 1.3)];

fn dock_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [DockOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().cloned().enumerate() {
        let option_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("{id}-{index}"))),
                option_label,
            )
            .flex_1()
            .selected(selected == Some(index))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                option_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(change.clone(), cx)
                });
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn dock_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: Option<SharedString>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> DockChange,
) -> AnyElement {
    let toggle_view = view.clone();
    let toggle = Toggle::new(id)
        .checked(checked)
        .disabled(!enabled)
        .on_click(move |value, _, cx| {
            toggle_view.update(cx, |settings, cx| {
                settings.apply_dock_change(change(*value), cx)
            });
        });
    row_base()
        .child(text_block(title.into(), subtitle))
        .child(toggle)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn dock_output_row(
    view: &Entity<Settings>,
    id: &str,
    title: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    enabled: bool,
    scope: rmac_shell_settings::OutputScope,
) -> AnyElement {
    let output_view = view.clone();
    row_base()
        .child(text_block(title, subtitle))
        .child(
            Button::new(
                ElementId::from(SharedString::from(format!("dock-output-{id}"))),
                if selected { "Selected" } else { "Use" },
            )
            .selected(selected)
            .disabled(!enabled || selected)
            .on_click(move |_, _, cx| {
                output_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(DockChange::Outputs(scope.clone()), cx)
                });
            }),
        )
        .into_any_element()
}

fn wallpaper_fit_row(
    view: Entity<Settings>,
    selected: rmac_shell_settings::WallpaperFit,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(380.0));
    for (index, (label, fit)) in WALLPAPER_FIT_OPTIONS.iter().copied().enumerate() {
        let fit_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("wallpaper-fit-{index}"))),
                label,
            )
            .flex_1()
            .selected(selected == fit)
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                fit_view.update(cx, |settings, cx| {
                    let target = settings.wallpaper_target.clone();
                    settings.apply_wallpaper_change(target, WallpaperChange::Fit(fit), cx);
                });
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("Display mode"),
        )
        .child(control)
        .into_any_element()
}

fn shortcut_configuration_available(status: Option<&rmac_shortcuts::BackendStatus>) -> bool {
    matches!(
        status,
        Some(rmac_shortcuts::BackendStatus::Portal {
            version,
            can_configure: true,
        }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION
    )
}

fn spotlight_provider_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let toggle_view = view.clone();
    row_base()
        .child(text_block(title.into(), Some(subtitle.into())))
        .child(
            Toggle::new(ElementId::from(SharedString::from(format!(
                "spotlight-provider-{id}"
            ))))
            .checked(checked)
            .disabled(!enabled)
            .on_click(move |value, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.apply_spotlight_change(
                        SpotlightChange::ProviderEnabled {
                            id: id.into(),
                            enabled: *value,
                        },
                        cx,
                    )
                });
            }),
        )
        .into_any_element()
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
                .text_size(rmac_ui::text_px(11.0))
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
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

fn gtk_text_scale_row(
    view: Entity<Settings>,
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, factor)) in GTK_TEXT_SCALE_OPTIONS.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "gtk-text-scale-{index}"
                ))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .text_size(rmac_ui::text_px(11.0))
                .when(selected == Some(index), |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(selected != Some(index), |element| {
                    element.bg(rmac_ui::mac::control_fill()).text_color(label())
                })
                .when(enabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::control_fill_hover()))
                        .on_click(move |_, _, cx| {
                            option_view
                                .update(cx, |settings, cx| settings.set_gtk_text_scale(factor, cx));
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
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("GTK application text"),
        )
        .child(control)
        .into_any_element()
}

fn input_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [InputOption],
    selected: Option<usize>,
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
                .text_size(rmac_ui::text_px(11.0))
                .when(selected == Some(index), |element| {
                    element.bg(accent()).text_color(on_accent())
                })
                .when(selected != Some(index), |element| {
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
                .text_size(rmac_ui::text_px(13.0))
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
        r = r.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
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

fn battery_history_card(points: &[rmac_power::BatteryHistoryPoint]) -> Div {
    let samples = sample_battery_history(points, 48);
    let minimum = points
        .iter()
        .map(|point| point.percentage)
        .min()
        .unwrap_or_default();
    let maximum = points
        .iter()
        .map(|point| point.percentage)
        .max()
        .unwrap_or_default();
    let latest = points
        .last()
        .map(|point| point.percentage)
        .unwrap_or_default();
    let bars = samples.into_iter().map(|point| {
        let color = if matches!(
            point.state,
            rmac_power::BatteryState::Charging | rmac_power::BatteryState::PendingCharge
        ) {
            hsl(0x34c759)
        } else {
            accent()
        };
        div()
            .flex_1()
            .min_w(px(2.0))
            .h(px(4.0 + f32::from(point.percentage) * 0.72))
            .rounded(px(2.0))
            .bg(color)
    });
    div()
        .v_flex()
        .mb_3()
        .gap_2()
        .p_3()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep())
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(format!(
                    "Last 24 hours · {minimum}% minimum · {maximum}% maximum · {latest}% latest"
                )),
        )
        .child(
            div()
                .h(px(80.0))
                .flex()
                .items_end()
                .gap(px(2.0))
                .children(bars),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(rmac_ui::text_px(10.5))
                .text_color(rmac_ui::mac::text_tertiary())
                .child("24 hours ago")
                .child("Now"),
        )
}

/// Format bytes as decimal GB (matching macOS storage display).
fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

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
