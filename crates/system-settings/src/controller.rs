//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · local account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive and backed by typed Linux/macOS services. Unsupported
//! mutations are explicitly unavailable rather than represented by local state.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Read-only panes use real platform state rather than fabricated values.

mod bluetooth;
mod chrome;
mod date_time;
mod desktop_dock;
mod detail;
mod locale;
mod lock_screen;
mod login_items;
mod navigation_state;
mod network;
mod sharing;
mod software_updates;
mod storage;
mod system_info;
mod vpn;
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

    fn refresh_wallpaper_preview(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.shell_settings.as_ref() else {
            self.wallpaper_preview_loading = false;
            self.wallpaper_preview_error = Some("Wallpaper settings are unavailable".into());
            return;
        };
        let (selection, _) =
            wallpaper_selection(&snapshot.settings.wallpaper, &self.wallpaper_target);
        let watched_paths = rmac_wallpaper::parse_source(selection.source.as_deref())
            .ok()
            .and_then(|source| rmac_wallpaper::file_path(&source).map(PathBuf::from))
            .into_iter()
            .collect::<Vec<_>>();
        self.wallpaper_preview_generation = self.wallpaper_preview_generation.wrapping_add(1);
        let generation = self.wallpaper_preview_generation;
        self.wallpaper_preview_loading = true;
        self.wallpaper_preview_error = None;
        self.wallpaper_preview_watch_error = None;
        self._wallpaper_preview_watcher = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (events_tx, events_rx) = async_channel::bounded(1);
            let (result, watcher) = blocking::unblock(move || {
                let watcher = rmac_wallpaper_image::watch_files(&watched_paths, events_tx);
                (render_wallpaper_preview(&selection), watcher)
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.wallpaper_preview_generation != generation {
                    return;
                }
                this.wallpaper_preview_loading = false;
                match result {
                    Ok(preview) => {
                        this.wallpaper_preview = Some(preview);
                        this.wallpaper_preview_error = None;
                    }
                    Err(error) => {
                        this.wallpaper_preview_error =
                            Some(format!("Could not preview this wallpaper: {error}").into());
                    }
                }
                let watching = match watcher {
                    Ok(watcher) => {
                        let watching = watcher.is_some();
                        this._wallpaper_preview_watcher = watcher;
                        this.wallpaper_preview_watch_error = None;
                        watching
                    }
                    Err(_) => {
                        this.wallpaper_preview_watch_error = Some(
                            "Live updates for the selected wallpaper file are unavailable".into(),
                        );
                        false
                    }
                };
                if watching {
                    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                        let event = events_rx.recv().await;
                        let _ = this.update(cx, |this: &mut Settings, cx| {
                            if this.wallpaper_preview_generation != generation {
                                return;
                            }
                            match event {
                                Ok(rmac_wallpaper_image::FileWatchEvent::Changed) => {
                                    this.refresh_wallpaper_preview(cx);
                                }
                                Ok(rmac_wallpaper_image::FileWatchEvent::Failed { .. })
                                | Err(_) => {
                                    this._wallpaper_preview_watcher = None;
                                    this.wallpaper_preview_watch_error = Some(
                                        "Live updates for the selected wallpaper file stopped"
                                            .into(),
                                    );
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .detach();
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_wallpaper_target(&mut self, target: WallpaperTarget, cx: &mut Context<Self>) {
        if self.shell_settings_busy || self.wallpaper_target == target {
            return;
        }
        self.wallpaper_target = target;
        self.wallpaper_error = None;
        self.refresh_wallpaper_preview(cx);
    }

    fn apply_wallpaper_change(
        &mut self,
        target: WallpaperTarget,
        change: WallpaperChange,
        cx: &mut Context<Self>,
    ) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = snapshot.settings.wallpaper.clone();
        let mut next = previous.clone();
        change.clone().apply(&target, &mut next);
        if next == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Wallpaper { target, change })
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, Some(previous));
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_wallpaper_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<rmac_shell_settings::WallpaperSettings>,
    ) {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                if self
                    .shell_settings
                    .as_ref()
                    .is_some_and(|current| current.settings.dock != snapshot.settings.dock)
                {
                    self.shell_settings_revert = None;
                }
                if self.shell_settings.as_ref().is_some_and(|current| {
                    SpotlightAuthority::from_settings(&current.settings)
                        != SpotlightAuthority::from_settings(&snapshot.settings)
                }) {
                    self.spotlight_revert = None;
                }
                self.shell_settings = Some(snapshot);
                self.wallpaper_revert = previous;
                self.wallpaper_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
            }
            Err(error) => {
                self.wallpaper_error = Some(format!("Could not update Wallpaper: {error}").into());
            }
        }
    }

    fn choose_wallpaper_file(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let target = self.wallpaper_target.clone();
        let fit = wallpaper_selection(&snapshot.settings.wallpaper, &target)
            .0
            .fit;
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_wallpaper_file().await;
            let validated = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            blocking::unblock(move || validate_wallpaper_choice(path, fit)).await
                        })
                        .await,
                ),
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(source)) => this.apply_wallpaper_change(
                        target,
                        WallpaperChange::Source(Some(source)),
                        cx,
                    ),
                    Some(Err(error)) => {
                        this.wallpaper_error = Some(error.into());
                    }
                    None => this.refresh_shell_settings(true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_wallpaper_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.wallpaper_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.wallpaper_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreWallpaper(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wallpaper_mutation(result, None);
                this.refresh_wallpaper_preview(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_spotlight_change(&mut self, change: SpotlightChange, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            return;
        };
        let previous = SpotlightAuthority::from_settings(&snapshot.settings);
        let mut next = snapshot.settings.clone();
        change.clone().apply(&mut next);
        if SpotlightAuthority::from_settings(&next) == previous {
            return;
        }

        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::Spotlight(change))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, Some(previous)) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_spotlight_mutation(
        &mut self,
        result: std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error>,
        previous: Option<SpotlightAuthority>,
    ) -> bool {
        self.shell_settings_loading = false;
        self.shell_settings_busy = false;
        match result {
            Ok(snapshot) => {
                let wallpaper_changed = self.shell_settings.as_ref().is_none_or(|current| {
                    current.settings.wallpaper != snapshot.settings.wallpaper
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
                self.shell_settings = Some(snapshot);
                self.spotlight_revert = previous;
                self.spotlight_error = None;
                self.shell_settings_error = None;
                self.shell_settings_stream_error = None;
                wallpaper_changed
            }
            Err(error) => {
                self.spotlight_error = Some(format!("Could not update Spotlight: {error}").into());
                false
            }
        }
    }

    fn choose_search_exclusion(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_search_exclusion().await;
            let validated = match choice {
                Ok(Some(path)) => {
                    Some(blocking::unblock(move || validate_search_exclusion(path)).await)
                }
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shell_settings_busy = false;
                match validated {
                    Some(Ok(path)) => {
                        this.apply_spotlight_change(SpotlightChange::AddExclusion(path), cx)
                    }
                    Some(Err(error)) => this.spotlight_error = Some(error.into()),
                    None => this.refresh_shell_settings(false, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_spotlight_change(&mut self, cx: &mut Context<Self>) {
        if self.shell_settings_loading || self.shell_settings_busy {
            return;
        }
        let Some(previous) = self.spotlight_revert.clone() else {
            return;
        };
        self.shell_settings_busy = true;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                persist_shell_settings_mutation(ShellSettingsMutation::RestoreSpotlight(previous))
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.finish_spotlight_mutation(result, None) {
                    this.refresh_wallpaper_preview(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn request_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy {
            return;
        }
        self.recent_history_confirmation = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
    }

    fn cancel_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if !self.recent_history_busy && self.recent_history_confirmation {
            self.recent_history_confirmation = false;
            cx.notify();
        }
    }

    fn confirm_recent_history_clear(&mut self, cx: &mut Context<Self>) {
        if self.recent_history_busy || !self.recent_history_confirmation {
            return;
        }
        self.recent_history_confirmation = false;
        self.recent_history_busy = true;
        self.recent_history_notice = None;
        self.spotlight_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_recent_documents::Store::from_environment().and_then(|store| store.clear())
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.recent_history_busy = false;
                match result {
                    Ok(_) => {
                        this.recent_history_notice =
                            Some("Recent document history was cleared for rmac Search.".into());
                    }
                    Err(_) => {
                        this.spotlight_error =
                            Some("Could not clear recent document history.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_shortcut_status_update(
        &mut self,
        result: std::result::Result<rmac_shortcuts::BackendStatus, rmac_shortcuts::Error>,
    ) {
        self.shortcut_status_loading = false;
        match result {
            Ok(status) => {
                self.shortcut_status = Some(status);
                self.shortcut_status_error = None;
            }
            Err(_) => {
                self.shortcut_status_error =
                    Some("The session shortcut broker has not reported its backend".into());
            }
        }
    }

    fn refresh_shortcut_status(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_status_loading {
            return;
        }
        self.shortcut_status_loading = true;
        self.shortcut_status_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_shortcuts::backend_status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shortcut_status_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn configure_global_shortcuts(&mut self, cx: &mut Context<Self>) {
        if self.shortcut_configuration_busy
            || !shortcut_configuration_available(self.shortcut_status.as_ref())
        {
            return;
        }
        self.shortcut_configuration_busy = true;
        self.shortcut_configuration_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(rmac_shortcuts::request_shortcut_configuration).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.shortcut_configuration_busy = false;
                if result.is_err() {
                    this.shortcut_configuration_error = Some(
                        "Could not open global shortcut configuration. The live session broker or portal may be unavailable."
                            .into(),
                    );
                }
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
        let refresh_pending = std::mem::take(&mut self.audio_refresh_pending);
        self.audio_loading = false;
        self.audio_busy = false;
        match result {
            Ok(snapshot) => {
                self.replace_audio_snapshot(snapshot, cx);
                self.audio_error = None;
                self.audio_stream_error = None;
            }
            Err(error) => {
                self.audio_error = Some(format!("Could not update Sound: {error}").into());
            }
        }
        if refresh_pending {
            self.refresh_audio(cx);
        }
    }

    fn replace_audio_snapshot(&mut self, snapshot: rmac_audio::Snapshot, cx: &mut Context<Self>) {
        self.output_volume_generation = self.output_volume_generation.wrapping_add(1);
        self.input_volume_generation = self.input_volume_generation.wrapping_add(1);
        self.output_balance_generation = self.output_balance_generation.wrapping_add(1);
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
        let balance = snapshot
            .outputs
            .iter()
            .find(|device| device.is_default)
            .and_then(|device| device.balance.as_ref())
            .map_or(0.0, |balance| f32::from(balance.value));
        self.output_balance = Self::audio_balance_slider(cx, balance);
        self.audio = snapshot;
    }

    fn finish_audio_stream_update(
        &mut self,
        result: std::result::Result<rmac_audio::Snapshot, rmac_audio::Error>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(snapshot) => {
                self.replace_audio_snapshot(snapshot, cx);
                self.audio_stream_error = None;
            }
            Err(_) => {
                self.audio_stream_error =
                    Some("Live audio state could not be refreshed from PipeWire".into());
            }
        }
    }

    fn refresh_audio(&mut self, cx: &mut Context<Self>) {
        if self.audio_loading || self.audio_busy {
            return;
        }
        self.audio_generation = self.audio_generation.wrapping_add(1);
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
        device: rmac_audio::Device,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading
            || self.audio_busy
            || !self.audio.available
            || !self.audio.can_set_default
        {
            return;
        }
        self.apply_audio_change(AudioChange::DefaultDevice(kind, device), cx);
    }

    fn set_audio_profile(
        &mut self,
        device: rmac_audio::HardwareDevice,
        profile: rmac_audio::Profile,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Profile(device, profile), cx);
    }

    fn set_audio_route(
        &mut self,
        kind: rmac_audio::DeviceKind,
        device: rmac_audio::Device,
        route: rmac_audio::Route,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Route(kind, device, route), cx);
    }

    fn schedule_audio_balance(&mut self, value: f32, cx: &mut Context<Self>) {
        if self.audio_loading || !self.audio.available || !self.audio.has_output {
            return;
        }
        self.output_balance_generation = self.output_balance_generation.wrapping_add(1);
        let generation = self.output_balance_generation;
        let value = value.round().clamp(-100.0, 100.0) as i8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.output_balance_generation != generation || this.audio_busy {
                    return;
                }
                let Some(device) = this
                    .audio
                    .outputs
                    .iter()
                    .find(|device| device.is_default && device.balance.is_some())
                    .cloned()
                else {
                    return;
                };
                this.apply_audio_change(AudioChange::Balance(device, value), cx);
            });
        })
        .detach();
    }

    fn apply_audio_change(&mut self, change: AudioChange, cx: &mut Context<Self>) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        self.audio_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { change.apply() })
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
        cx: &mut Context<Self>,
    ) {
        let refresh_pending = std::mem::take(&mut self.power_refresh_pending);
        self.power_loading = false;
        self.power_busy = false;
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_error = None;
                self.power_stream_error = None;
            }
            Err(error) => {
                self.power_error = Some(format!("Could not update Battery: {error}").into());
            }
        }
        if refresh_pending {
            self.refresh_power(cx);
        }
    }

    fn finish_power_stream_update(
        &mut self,
        result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.power = snapshot;
                self.power_stream_error = None;
            }
            Err(_) => {
                self.power_stream_error =
                    Some("Live battery state could not be refreshed from UPower".into());
            }
        }
    }

    fn refresh_power(&mut self, cx: &mut Context<Self>) {
        if self.power_loading || self.power_busy {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
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
    }

    fn set_power_profile(&mut self, profile: rmac_power::PowerProfile, cx: &mut Context<Self>) {
        if self.power_loading
            || self.power_busy
            || !self.power.profiles.available
            || !self.power.profiles.supported.contains(&profile)
        {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { apply_power_profile(profile) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_charge_threshold(
        &mut self,
        threshold: rmac_power::ChargeThreshold,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let is_current = self.power.battery.as_ref().is_some_and(|battery| {
            battery.charge_threshold == threshold && battery.charge_threshold.can_change()
        });
        if self.power_loading || self.power_busy || !is_current || threshold.enabled == enabled {
            return;
        }
        self.power_generation = self.power_generation.wrapping_add(1);
        self.power_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let update = cx
                .background_executor()
                .spawn(async move { apply_charge_threshold(&threshold, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(snapshot) = update.recovery {
                    this.power = snapshot;
                }
                this.finish_power_update(update.result, cx);
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

    fn request_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            self.display_refresh_pending = true;
            return;
        }
        self.display_refresh_pending = false;
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.display_generation == generation
                    && !this.display_loading
                    && !this.display_busy
                    && this.display_confirmation.is_none()
                {
                    this.finish_display_update(result);
                    cx.notify();
                } else {
                    this.display_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    fn flush_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_refresh_pending
            && !self.display_loading
            && !self.display_busy
            && self.display_confirmation.is_none()
        {
            self.request_display_stream_refresh(cx);
        }
    }

    fn refresh_displays(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            return;
        }
        self.display_refresh_pending = false;
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
                    this.display_generation = this.display_generation.wrapping_add(1);
                    this.display_confirmation = None;
                }
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_display_change(&mut self, change: DisplayChange, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_configure
        {
            return;
        }
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { change.apply() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(applied) => {
                        let confirmed = applied.snapshot.clone();
                        this.finish_display_update(Ok(applied.snapshot));
                        this.begin_display_confirmation(applied.baseline, confirmed, cx);
                    }
                    Err(error) => {
                        this.finish_display_update(Err(error));
                        this.flush_display_stream_refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn begin_display_confirmation(
        &mut self,
        baseline: rmac_display::Snapshot,
        applied: rmac_display::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        self.display_confirmation = Some(DisplayConfirmation {
            baseline,
            applied,
            generation,
            seconds_remaining: DISPLAY_CONFIRMATION_SECONDS,
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            for remaining in (0..DISPLAY_CONFIRMATION_SECONDS).rev() {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let update = this.update(cx, |this: &mut Settings, cx| {
                    let current = this.display_confirmation.as_ref().is_some_and(|pending| {
                        pending.generation == generation && this.display_generation == generation
                    });
                    if !current {
                        return false;
                    }
                    if remaining == 0 {
                        this.revert_display_change(cx);
                    } else if let Some(pending) = &mut this.display_confirmation {
                        pending.seconds_remaining = remaining;
                        cx.notify();
                    }
                    true
                });
                if !matches!(update, Ok(true)) || remaining == 0 {
                    break;
                }
            }
        })
        .detach();
    }

    fn keep_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_persist {
            return;
        }
        let Some(pending) = self.display_confirmation.clone() else {
            return;
        };
        let Some(primary) = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .or_else(|| {
                self.display
                    .outputs
                    .iter()
                    .find(|output| output.logical.is_some())
            })
            .map(|output| output.id.clone())
        else {
            self.display_error = Some("No enabled display can be saved as Main".into());
            cx.notify();
            return;
        };
        let layout = match rmac_display::current_layout(&self.display, &primary) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error =
                    Some(format!("Could not prepare display layout: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_confirmation = None;
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(snapshot) => {
                        this.finish_display_update(Ok(snapshot));
                        this.flush_display_stream_refresh(cx);
                    }
                    Err(error) => {
                        this.display_busy = false;
                        this.display_error =
                            Some(format!("Could not save display layout: {error}").into());
                        this.begin_display_confirmation(pending.baseline, pending.applied, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_primary_display(&mut self, output: String, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_persist
        {
            return;
        }
        let layout = match rmac_display::current_layout(&self.display, &output) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error = Some(format!("Could not select Main display: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        let Some(pending) = self.display_confirmation.take() else {
            return;
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_display::restore_snapshot(&pending.baseline, &pending.applied)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
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

    fn finish_privacy_update(
        &mut self,
        result: std::result::Result<rmac_privacy::Snapshot, rmac_privacy_linux::Error>,
    ) {
        self.privacy_loading = false;
        self.privacy_busy = None;
        match result {
            Ok(snapshot) => {
                self.privacy = Some(snapshot);
                self.privacy_error = None;
            }
            Err(error) => {
                self.privacy_error =
                    Some(format!("Could not update portal permissions: {error}").into());
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
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
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
    }

    fn apply_theme_change(&mut self, change: ThemeChange, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        let Some(theme) = &self.theme else {
            return;
        };
        let expected = theme.preferences.clone();
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(apply_theme_change_authoritatively(change, expected))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn queue_theme_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            self.theme_refresh_pending = true;
            return;
        }
        self.theme_refresh_pending = false;
        self.theme_stream_refreshing = true;
        let generation = self.theme_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.theme_stream_refreshing = false;
                if theme_stream_snapshot_is_current(
                    generation,
                    this.theme_generation,
                    this.theme_loading,
                    this.theme_busy,
                ) {
                    match result {
                        Ok(load) => this.finish_theme_update(Ok(load)),
                        Err(_) => {
                            this.theme_error =
                                Some("Could not refresh changed appearance preferences".into());
                        }
                    }
                } else {
                    this.theme_refresh_pending = true;
                }
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn run_pending_theme_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_refresh_pending
            && !self.theme_loading
            && !self.theme_busy
            && !self.theme_stream_refreshing
        {
            self.queue_theme_stream_refresh(cx);
        }
    }

    fn refresh_input(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
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
    }

    fn request_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_stream_refreshing = true;
        self.input_generation = self.input_generation.wrapping_add(1);
        let generation = self.input_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.input_stream_refreshing = false;
                if input_stream_snapshot_is_current(
                    generation,
                    this.input_generation,
                    this.input_loading,
                    this.input_busy,
                ) {
                    this.finish_input_update(result);
                    this.flush_input_stream_refresh(cx);
                    cx.notify();
                } else {
                    this.input_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    fn flush_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_refresh_pending
            && !self.input_loading
            && !self.input_busy
            && !self.input_stream_refreshing
        {
            self.request_input_stream_refresh(cx);
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

    fn queue_privacy_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            self.privacy_refresh_pending = true;
            return;
        }
        self.privacy_refresh_pending = false;
        self.privacy_stream_refreshing = true;
        let generation = self.privacy_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.privacy_stream_refreshing = false;
                if privacy_stream_snapshot_is_current(
                    generation,
                    this.privacy_generation,
                    this.privacy_loading,
                    this.privacy_busy.is_some(),
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.finish_privacy_update(Ok(snapshot));
                            this.privacy_stream_error = None;
                        }
                        Err(error) => this.finish_privacy_update(Err(error)),
                    }
                } else {
                    this.privacy_refresh_pending = true;
                }
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn run_pending_privacy_refresh(&mut self, cx: &mut Context<Self>) {
        if self.privacy_refresh_pending
            && !self.privacy_loading
            && self.privacy_busy.is_none()
            && !self.privacy_stream_refreshing
        {
            self.queue_privacy_stream_refresh(cx);
        }
    }

    fn refresh_privacy(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            return;
        }
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        self.privacy_loading = true;
        cx.notify();
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
    }

    fn refresh_security_coverage(&mut self, cx: &mut Context<Self>) {
        if self.security_coverage_loading {
            return;
        }
        self.security_coverage_loading = true;
        cx.notify();
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
    }

    fn request_privacy_reset(
        &mut self,
        decision: rmac_privacy::PortalDecision,
        cx: &mut Context<Self>,
    ) {
        if self.privacy_busy.is_none()
            && !self.privacy_loading
            && !self.privacy_stream_refreshing
            && self.privacy.as_ref().is_some_and(|snapshot| {
                snapshot.can_reset && snapshot.decisions.contains(&decision)
            })
        {
            self.privacy_reset_confirmation = Some(decision);
            cx.notify();
        }
    }

    fn cancel_privacy_reset(&mut self, cx: &mut Context<Self>) {
        if self.privacy_reset_confirmation.take().is_some() {
            cx.notify();
        }
    }

    fn confirm_privacy_reset(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            return;
        }
        let Some(decision) = self.privacy_reset_confirmation.take() else {
            return;
        };
        let resource = decision.resource;
        let app_id = decision.app_id.clone();
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        self.privacy_busy = Some((resource, app_id.clone()));
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_privacy_linux::reset_decision(&decision) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
                this.run_pending_privacy_refresh(cx);
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
        change.apply(&mut settings);
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_input::save(&settings) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- explicit shell-owned panes ----------------------------------

    fn render_spotlight(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let choose_view = view.clone();
        let configure_shortcuts_view = view.clone();
        let clear_history_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("rmac Search"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("spotlight-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.spotlight_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_spotlight_change(cx)
                                });
                            }),
                    )
                    .child(
                        Button::new(
                            "spotlight-refresh",
                            if self.shell_settings_busy {
                                "Applying…"
                            } else if self.shell_settings_loading || self.shortcut_status_loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                        )
                        .disabled(self.shell_settings_loading || self.shell_settings_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| {
                                settings.refresh_shell_settings(false, cx);
                                settings.refresh_shortcut_status(cx);
                            });
                        }),
                    ),
            )];

        cards.push(section_header("Recent documents"));
        cards.push(card(vec![row_base()
            .child(text_block(
                "rmac recent history".into(),
                Some(
                    "Used by Files and Launcher alongside newer desktop XBEL entries; clearing never deletes a document"
                        .into(),
                ),
            ))
            .child(
                Button::new(
                    "spotlight-clear-history",
                    if self.recent_history_busy {
                        "Clearing…"
                    } else {
                        "Clear…"
                    },
                )
                .disabled(self.recent_history_busy || self.recent_history_confirmation)
                .on_click(move |_, _, cx| {
                    clear_history_view.update(cx, |settings, cx| {
                        settings.request_recent_history_clear(cx)
                    });
                }),
            )
            .into_any_element()]));
        if let Some(notice) = &self.recent_history_notice {
            cards.push(note_card(notice.clone()));
        }
        if self.recent_history_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(
                "Clear recent documents from rmac Search? Files are not deleted. rmac will hide existing desktop-history entries, while other applications continue to manage and display their own history.",
            ));
            cards.push(card(vec![row_base()
                .child(div().flex_1())
                .child(
                    Button::new("spotlight-clear-history-cancel", "Cancel").on_click(
                        move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.cancel_recent_history_clear(cx)
                            });
                        },
                    ),
                )
                .child(
                    rmac_ui::dialog_button(
                        "spotlight-clear-history-confirm",
                        "Clear History",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(self.recent_history_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view
                            .update(cx, |settings, cx| settings.confirm_recent_history_clear(cx));
                    }),
                )
                .into_any_element()]));
        }

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading authoritative search preferences…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. Search preferences remain unchanged.",
            ));
            return self.pane(cards);
        };
        let settings = &snapshot.settings;
        let enabled = !self.shell_settings_busy;
        let applications =
            spotlight_provider_policy(settings, rmac_launcher_providers::APPLICATIONS_PROVIDER);
        let settings_provider =
            spotlight_provider_policy(settings, rmac_launcher_providers::SETTINGS_PROVIDER);
        let files = spotlight_provider_policy(settings, rmac_launcher_providers::FILES_PROVIDER);
        let calculator =
            spotlight_provider_policy(settings, rmac_launcher_providers::CALCULATOR_PROVIDER);

        cards.push(section_header("Search results"));
        cards.push(card(vec![
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::APPLICATIONS_PROVIDER,
                "Applications",
                "Installed desktop applications",
                applications.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::SETTINGS_PROVIDER,
                "System Settings",
                "Destinations and Linux-relevant setting keywords",
                settings_provider.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::FILES_PROVIDER,
                "Files",
                if files.allow_private_content {
                    "On-demand filenames and recent documents"
                } else {
                    "Private-content permission is required"
                },
                files.enabled,
                enabled,
            ),
            spotlight_provider_row(
                view.clone(),
                rmac_launcher_providers::CALCULATOR_PROVIDER,
                "Calculator",
                "Local bounded arithmetic; no scripts or network",
                calculator.enabled,
                enabled,
            ),
        ]));

        cards.push(section_header("File privacy and scope"));
        let private_view = view.clone();
        let removable_view = view.clone();
        cards.push(card(vec![
            row_base()
                .child(text_block(
                    "Allow private file results".into(),
                    Some("Admit local filenames and recent-document paths to Search".into()),
                ))
                .child(
                    Toggle::new("spotlight-private-files")
                        .checked(files.allow_private_content)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            private_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::ProviderPrivateContent {
                                        id: rmac_launcher_providers::FILES_PROVIDER.into(),
                                        allowed: *value,
                                    },
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
            row_base()
                .child(text_block(
                    "Include removable mounts".into(),
                    Some("Allow on-demand file search to cross filesystem boundaries".into()),
                ))
                .child(
                    Toggle::new("spotlight-removable-mounts")
                        .checked(settings.spotlight.include_removable_mounts)
                        .disabled(!enabled)
                        .on_click(move |value, _, cx| {
                            removable_view.update(cx, |settings, cx| {
                                settings.apply_spotlight_change(
                                    SpotlightChange::IncludeRemovableMounts(*value),
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
        ]));
        cards.push(note_card(
            "File search is local and on demand. rmac does not build a perpetual content index, and no built-in provider requests network access.",
        ));

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
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child("Excluded folders"),
                )
                .child(
                    Button::new("spotlight-add-exclusion", "Add Folder…")
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            choose_view
                                .update(cx, |settings, cx| settings.choose_search_exclusion(cx));
                        }),
                ),
        );
        if settings.spotlight.excluded_paths.is_empty() {
            cards.push(note_card(
                "No folders are excluded. Add a folder to prune it before filename traversal and recent-document admission.",
            ));
        } else {
            let exclusion_rows = settings
                .spotlight
                .excluded_paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let remove_view = view.clone();
                    let remove_path = path.clone();
                    row_base()
                        .child(text_block(
                            PathBuf::from(path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.clone())
                                .into(),
                            Some(path.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "spotlight-remove-exclusion-{index}"
                                ))),
                                "Remove",
                            )
                            .disabled(!enabled)
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |settings, cx| {
                                    settings.apply_spotlight_change(
                                        SpotlightChange::RemoveExclusion(remove_path.clone()),
                                        cx,
                                    )
                                });
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(exclusion_rows));
        }

        cards.push(section_header("Indexing"));
        cards.push(card(vec![
            value_row(
                "icons/search.svg",
                accent(),
                "Search mode".into(),
                "On demand".into(),
            ),
            value_row(
                "icons/hard-drive.svg",
                secondary(),
                "Filesystem scope".into(),
                if settings.spotlight.include_removable_mounts {
                    "Home and removable mounts".into()
                } else {
                    "Home filesystem only".into()
                },
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Background content index".into(),
                "Not used".into(),
            ),
        ]));

        let launcher_shortcut = rmac_shortcuts::default_shortcuts()
            .into_iter()
            .find(|shortcut| shortcut.id.0 == "launcher")
            .expect("the stable launcher shortcut is registered");
        let shortcut_status: SharedString = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure,
            }) => format!(
                "Portal v{version}{}",
                if *can_configure && *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION {
                    " · configurable"
                } else {
                    ""
                }
            )
            .into(),
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "niri fallback required".into()
            }
            None if self.shortcut_status_loading => "Loading…".into(),
            None => "Not reported".into(),
        };
        let shortcut_configuration_enabled =
            shortcut_configuration_available(self.shortcut_status.as_ref());
        let shortcut_configuration_detail = match self.shortcut_status.as_ref() {
            Some(rmac_shortcuts::BackendStatus::Portal {
                version,
                can_configure: true,
            }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION => {
                "Open the portal UI for every shortcut in the live rmac session"
            }
            Some(rmac_shortcuts::BackendStatus::Portal { .. }) => {
                "The active portal is older than GlobalShortcuts version 2"
            }
            Some(rmac_shortcuts::BackendStatus::FallbackRequired { .. }) => {
                "The generated niri fallback remains the shortcut authority"
            }
            None if self.shortcut_status_loading => "Waiting for the session broker",
            None => "The session broker has not reported its shortcut backend",
        };
        cards.push(section_header("Keyboard shortcut"));
        cards.push(card(vec![
            value_row(
                "icons/keyboard.svg",
                accent(),
                "Active backend".into(),
                shortcut_status,
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "Portal preference".into(),
                launcher_shortcut.preferred_trigger.into(),
            ),
            value_row(
                "icons/keyboard.svg",
                secondary(),
                "niri fallback".into(),
                launcher_shortcut.niri_trigger.into(),
            ),
            row_base()
                .child(text_block(
                    "Global shortcuts".into(),
                    Some(shortcut_configuration_detail.into()),
                ))
                .child(
                    Button::new(
                        "spotlight-configure-shortcuts",
                        if self.shortcut_configuration_busy {
                            "Opening…"
                        } else {
                            "Configure…"
                        },
                    )
                    .disabled(self.shortcut_configuration_busy || !shortcut_configuration_enabled)
                    .on_click(move |_, _, cx| {
                        configure_shortcuts_view.update(cx, |settings, cx| {
                            settings.configure_global_shortcuts(cx);
                        });
                    }),
                )
                .into_any_element(),
        ]));
        if let Some(error) = self.shortcut_status_error.clone() {
            cards.push(note_card(error));
        }
        if let Some(error) = self.shortcut_configuration_error.clone() {
            cards.push(note_card(error));
        }
        cards.push(note_card(
            "The portal owns user consent and the actual trigger. Configure opens its UI through the broker's existing session; it never creates a second binding authority. The fallback is enabled only when the broker reports it is required, so one shortcut backend owns Logo/Mod+Space at a time.",
        ));
        cards.push(note_card(
            "These preferences are consumed by the launcher provider/runtime foundations. The centered GPUI overlay and full live session wiring remain D7/D8 release gates.",
        ));
        self.pane(cards)
    }

    fn render_wallpaper(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let choose_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("Desktop wallpaper"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("wallpaper-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.wallpaper_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_wallpaper_change(cx)
                                });
                            }),
                    )
                    .child(
                        Button::new("wallpaper-choose", "Choose Image…")
                            .disabled(self.shell_settings_loading || self.shell_settings_busy)
                            .on_click(move |_, _, cx| {
                                choose_view
                                    .update(cx, |settings, cx| settings.choose_wallpaper_file(cx));
                            }),
                    )
                    .child(
                        Button::new(
                            "wallpaper-refresh",
                            if self.shell_settings_busy {
                                "Applying…"
                            } else if self.shell_settings_loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                        )
                        .disabled(self.shell_settings_loading || self.shell_settings_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| {
                                settings.refresh_shell_settings(true, cx)
                            });
                        }),
                    ),
            )];

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative wallpaper settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. Wallpaper choices remain unchanged.",
            ));
            return self.pane(cards);
        };
        let wallpaper = &snapshot.settings.wallpaper;
        let (selection, owns_selection) = wallpaper_selection(wallpaper, &self.wallpaper_target);
        let enabled = !self.shell_settings_busy;

        cards.push(section_header("Apply to"));
        let mut targets = div().flex().flex_wrap().gap_2().mb_3();
        let default_view = view.clone();
        targets = targets.child(
            Button::new("wallpaper-target-default", "All displays (default)")
                .selected(self.wallpaper_target == WallpaperTarget::Default)
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    default_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(WallpaperTarget::Default, cx)
                    });
                }),
        );
        let mut output_ids = std::collections::BTreeSet::new();
        output_ids.extend(wallpaper.per_output.keys().cloned());
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            output_ids.insert(output.clone());
        }
        output_ids.extend(
            self.dock_compositor
                .outputs
                .values()
                .filter(|output| output.enabled())
                .map(|output| output.id.0.clone()),
        );
        for output_id in output_ids {
            let target = WallpaperTarget::Output(output_id.clone());
            let selected = self.wallpaper_target == target;
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|output| output.enabled() && output.id.0 == output_id);
            let label = self
                .dock_compositor
                .outputs
                .get(&rmac_compositor::OutputId(output_id.clone()))
                .map(|output| {
                    format!("{} {}", output.make, output.model)
                        .trim()
                        .to_owned()
                })
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| output_id.clone());
            let target_view = view.clone();
            targets = targets.child(
                Button::new(
                    ElementId::from(SharedString::from(format!("wallpaper-target-{output_id}"))),
                    if live {
                        label
                    } else {
                        format!("{label} · offline")
                    },
                )
                .selected(selected)
                .disabled(!enabled)
                .on_click(move |_, _, cx| {
                    target_view.update(cx, |settings, cx| {
                        settings.select_wallpaper_target(target.clone(), cx)
                    });
                }),
            );
        }
        cards.push(targets);

        cards.push(section_header("Preview"));
        let preview = div()
            .w(px(480.0))
            .h(px(270.0))
            .mx_auto()
            .mb_3()
            .rounded(px(12.0))
            .overflow_hidden()
            .bg(hsl(0x1e1e20))
            .border_1()
            .border_color(sep())
            .when_some(self.wallpaper_preview.clone(), |element, preview| {
                element.child(img(preview).w_full().h_full().object_fit(ObjectFit::Fill))
            })
            .when(self.wallpaper_preview.is_none(), |element| {
                element.flex().items_center().justify_center().child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(white())
                        .child(if self.wallpaper_preview_loading {
                            "Preparing preview…"
                        } else {
                            "Preview unavailable"
                        }),
                )
            });
        cards.push(preview);
        if self.wallpaper_preview_loading && self.wallpaper_preview.is_some() {
            cards.push(note_card(
                "Refreshing the preview from the selected source…",
            ));
        }
        if let Some(error) = self.wallpaper_preview_error.clone() {
            cards.push(note_card(error));
        }
        if let Some(error) = self.wallpaper_preview_watch_error.clone() {
            cards.push(note_card(error));
        }

        cards.push(section_header("Image"));
        let aurora_view = view.clone();
        let use_default_view = view.clone();
        let source_name = wallpaper_source_name(&selection);
        let using_aurora = matches!(
            rmac_wallpaper::parse_source(selection.source.as_deref()),
            Ok(rmac_wallpaper::Source::BuiltIn(_))
        );
        let mut source_rows = vec![row_base()
            .child(text_block(
                "Current image".into(),
                Some(match &self.wallpaper_target {
                    WallpaperTarget::Default => "Default for every display".into(),
                    WallpaperTarget::Output(_) if owns_selection => {
                        "Custom choice for this output".into()
                    }
                    WallpaperTarget::Output(_) => "Inherited from the default".into(),
                }),
            ))
            .child(
                div()
                    .max_w(px(190.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child(source_name),
            )
            .into_any_element()];
        source_rows.push(
            row_base()
                .child(text_block(
                    "Original Aurora".into(),
                    Some("Procedural rmac artwork; no third-party file".into()),
                ))
                .child(
                    Button::new("wallpaper-use-aurora", "Use")
                        .disabled(!enabled || using_aurora)
                        .on_click(move |_, _, cx| {
                            aurora_view.update(cx, |settings, cx| {
                                let target = settings.wallpaper_target.clone();
                                settings.apply_wallpaper_change(
                                    target,
                                    WallpaperChange::Source(None),
                                    cx,
                                )
                            });
                        }),
                )
                .into_any_element(),
        );
        if matches!(self.wallpaper_target, WallpaperTarget::Output(_)) {
            source_rows.push(
                row_base()
                    .child(text_block(
                        "Use default wallpaper".into(),
                        Some("Remove this output's saved override".into()),
                    ))
                    .child(
                        Button::new("wallpaper-use-default", "Use Default")
                            .disabled(!enabled || !owns_selection)
                            .on_click(move |_, _, cx| {
                                use_default_view.update(cx, |settings, cx| {
                                    let target = settings.wallpaper_target.clone();
                                    settings.apply_wallpaper_change(
                                        target,
                                        WallpaperChange::UseDefault,
                                        cx,
                                    )
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(source_rows));

        cards.push(section_header("Fit"));
        cards.push(card(vec![wallpaper_fit_row(
            view.clone(),
            selection.fit,
            enabled,
        )]));

        let connection = match self.dock_compositor.connection {
            rmac_compositor::ConnectionState::Connected => "Connected",
            rmac_compositor::ConnectionState::Connecting => "Connecting",
            rmac_compositor::ConnectionState::Reconnecting => "Reconnecting",
            rmac_compositor::ConnectionState::Disconnected => "Unavailable",
        };
        cards.push(section_header("Authority"));
        cards.push(card(vec![
            value_row(
                "icons/settings.svg",
                accent(),
                "Saved preferences".into(),
                "C4 shell settings".into(),
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "niri output stream".into(),
                connection.into(),
            ),
            value_row(
                "icons/image.svg",
                secondary(),
                "Accepted image types".into(),
                "PNG, JPEG, WebP".into(),
            ),
        ]));
        if let WallpaperTarget::Output(output) = &self.wallpaper_target {
            let live = self
                .dock_compositor
                .outputs
                .values()
                .any(|candidate| candidate.enabled() && candidate.id.0 == *output);
            if !live {
                cards.push(note_card(
                    "This output is currently unplugged or disabled. Its override remains authoritative and will return when the same stable niri output ID reappears.",
                ));
            }
        }
        if self.dock_compositor.connection != rmac_compositor::ConnectionState::Connected {
            cards.push(note_card(
                "niri is not connected in this process. Saved per-output choices remain editable, but live output availability cannot be confirmed.",
            ));
        }
        cards.push(note_card(
            "The preview uses the same bounded PNG/JPEG/WebP decoder and exact Fill, Fit, Stretch, Center, or Tile geometry as the wallpaper runtime. The Wayland background surface itself remains a separate D9 release gate.",
        ));
        self.pane(cards)
    }

    // ---- Wi-Fi --------------------------------------------------------

    fn render_wifi(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let power_subtitle = if self.wifi_loading {
            Some("Reading system state…".into())
        } else if self.wifi_forgetting.is_some() {
            Some("Forgetting saved network…".into())
        } else if self.wifi_connecting.is_some() {
            Some("Connecting to network…".into())
        } else if self.wifi_busy {
            Some("Applying change…".into())
        } else {
            self.wifi_interface
                .as_ref()
                .map(|interface| format!("NetworkManager · {interface}").into())
        };
        let power_view = view.clone();
        let power = Toggle::new("wifi-power")
            .checked(self.wifi_on)
            .disabled(self.wifi_loading || self.wifi_busy || !self.wifi_available)
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
            let refresh = Button::new("wifi-refresh", "Refresh")
                .small()
                .ghost()
                .busy(self.wifi_busy)
                .disabled(self.wifi_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_wifi(cx));
                });
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
                            .text_size(rmac_ui::text_px(12.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(secondary())
                            .child("Networks"),
                    )
                    .child(refresh),
            );

            let rows = if self.wifi_networks.is_empty() {
                vec![EmptyState::new("No networks found")
                    .message("Refresh to scan again")
                    .into_any_element()]
            } else {
                self.wifi_networks
                    .iter()
                    .enumerate()
                    .map(|(index, network)| {
                        let connecting = self.wifi_connecting.as_ref() == Some(&network.id);
                        let forgetting = self.wifi_forgetting.as_ref() == Some(&network.id);
                        let status = if forgetting {
                            "Forgetting…".to_string()
                        } else if connecting {
                            "Connecting…".to_string()
                        } else if network.connected {
                            format!("Connected · {}%", network.strength)
                        } else if network.known {
                            format!("Known Network · {}%", network.strength)
                        } else {
                            let security = match network.security {
                                rmac_network::WifiSecurity::Open => "Open Network",
                                rmac_network::WifiSecurity::EnhancedOpen => "Enhanced Open",
                                rmac_network::WifiSecurity::Personal(_) => "Password Required",
                                rmac_network::WifiSecurity::Enterprise => {
                                    "Enterprise Setup Required"
                                }
                                rmac_network::WifiSecurity::Legacy => "Unsupported Legacy Security",
                                rmac_network::WifiSecurity::Protected => "Unsupported Security",
                            };
                            format!("{security} · {}%", network.strength)
                        };
                        let can_connect = !self.wifi_busy && network.can_connect();
                        let selected_network = network.clone();
                        let network_view = view.clone();
                        ListRow::new(
                            SharedString::from(format!("wifi-network-{index}")),
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(tile(
                                    "icons/wifi.svg",
                                    if network.connected || connecting || forgetting {
                                        accent()
                                    } else {
                                        secondary()
                                    },
                                    22.0,
                                ))
                                .child(text_block(network.ssid.clone().into(), None))
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .text_color(secondary())
                                        .child(status),
                                ),
                        )
                        .h(px(50.0))
                        .px_3()
                        .disabled(!can_connect)
                        .on_activate(move |_, window, cx| {
                            network_view.update(cx, |settings, cx| {
                                settings.select_wifi_network(selected_network.clone(), window, cx)
                            });
                        })
                        .into_any_element()
                    })
                    .collect()
            };
            cards.push(card(rows));
            cards.push(note_card(
                "Select a network to connect. New WPA Personal and SAE networks ask for their password. Enterprise setup supports certificate-verified PEAP with MSCHAPv2; certificate, smart-card, and other EAP methods remain unavailable. Legacy security is not supported.",
            ));
        }

        if !self.wifi_saved_networks.is_empty() {
            cards.push(section_header("Known Networks"));
            let rows = self
                .wifi_saved_networks
                .iter()
                .enumerate()
                .map(|(index, network)| {
                    let forgetting = self.wifi_forgetting.as_ref() == Some(&network.id);
                    let security = match network.id.security() {
                        rmac_network::WifiSecurity::Open => "Saved Open Network",
                        rmac_network::WifiSecurity::EnhancedOpen => "Saved Enhanced Open Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Psk,
                        ) => "Saved WPA Personal Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Sae,
                        ) => "Saved SAE Network",
                        rmac_network::WifiSecurity::Personal(
                            rmac_network::WifiPersonalMode::Transition,
                        ) => "Saved WPA/SAE Network",
                        rmac_network::WifiSecurity::Enterprise => "Saved Enterprise Network",
                        rmac_network::WifiSecurity::Legacy => "Saved Legacy Network",
                        rmac_network::WifiSecurity::Protected => "Saved Protected Network",
                    };
                    let forget_network = network.id.clone();
                    let forget_ssid = SharedString::from(network.ssid.clone());
                    let forget_view = view.clone();
                    row_base()
                        .child(tile(
                            "icons/wifi.svg",
                            if forgetting { accent() } else { secondary() },
                            22.0,
                        ))
                        .child(text_block(
                            network.ssid.clone().into(),
                            Some(security.into()),
                        ))
                        .child(
                            Button::new(
                                SharedString::from(format!("wifi-saved-forget-{index}")),
                                "Forget…",
                            )
                            .xsmall()
                            .busy(forgetting)
                            .disabled(self.wifi_busy)
                            .on_click(move |_, _, cx| {
                                forget_view.update(cx, |settings, cx| {
                                    settings.request_wifi_forget(
                                        forget_network.clone(),
                                        forget_ssid.clone(),
                                        cx,
                                    )
                                });
                            }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
            cards.push(note_card(
                "Forgetting removes every accessible saved profile with the exact network identity. If it is active, this computer disconnects first.",
            ));
        }

        self.pane(cards)
    }

    fn render_wifi_password_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_password_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };

        let content = div()
            .w(px(380.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enter the password for this Wi-Fi network."),
                    ),
            )
            .child(TextField::new(&prompt.editor).disabled(busy).w_full())
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Connecting securely…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-password-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_password(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-password-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_password(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-password-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_password(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_password(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_wifi_enterprise_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_enterprise_prompt.as_ref()?;
        let busy = self.wifi_busy;
        let cancel_label = if busy { "Stop" } else { "Cancel" };
        let field = |label_text: &'static str, editor: &Entity<InputState>| {
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child(label_text),
                )
                .child(TextField::new(editor).disabled(busy).w_full())
        };

        let content = div()
            .w(px(430.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Join “{}”", prompt.ssid)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Enterprise Wi-Fi · PEAP with MSCHAPv2"),
                    ),
            )
            .child(field("Identity", &prompt.identity))
            .child(field(
                "Anonymous outer identity (optional)",
                &prompt.anonymous_identity,
            ))
            .child(field(
                "Server certificate domain",
                &prompt.certificate_domain,
            ))
            .child(field("Password", &prompt.password))
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child(
                        "The authentication server must chain to the system trust store and its certificate must match this domain. An anonymous outer identity can avoid exposing the login identity before the secure tunnel forms. Ask your network administrator for both values.",
                    ),
            )
            .when_some(prompt.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Verifying and connecting…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "wifi-enterprise-cancel",
                            cancel_label,
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_enterprise(cx))),
                    )
                    .when(!busy, |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "wifi-enterprise-submit",
                                "Join",
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| this.submit_wifi_enterprise(window, cx),
                            )),
                        )
                    }),
            );

        Some(
            rmac_ui::dialog("wifi-enterprise-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_wifi_enterprise(cx);
                        }
                        "enter" if !this.wifi_busy => {
                            cx.stop_propagation();
                            this.submit_wifi_enterprise(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
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
                            .text_size(rmac_ui::text_px(12.0))
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
                            .text_size(rmac_ui::text_px(12.0))
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
                .map(|device| {
                    bluetooth_device_row(
                        &view,
                        device,
                        self.bluetooth_busy,
                        self.bluetooth_forgetting.as_deref() == Some(device.id.as_str()),
                    )
                })
                .collect();
            if !connected.is_empty() {
                cards.push(section_header("Connected"));
                cards.push(card(connected));
            }

            let known: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.paired && !device.connected)
                .map(|device| {
                    bluetooth_device_row(
                        &view,
                        device,
                        self.bluetooth_busy,
                        self.bluetooth_forgetting.as_deref() == Some(device.id.as_str()),
                    )
                })
                .collect();
            if !known.is_empty() {
                cards.push(section_header("Known Devices"));
                cards.push(card(known));
            }

            let nearby: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| !device.paired && !device.connected)
                .map(|device| bluetooth_device_row(&view, device, self.bluetooth_busy, false))
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
                "Pairing uses a one-transaction confirmation agent. Confirm that displayed codes match; paired devices become trusted only after BlueZ reports success.",
            ));
        }
        self.pane(cards)
    }

    fn render_bluetooth_pairing_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let pairing = self.bluetooth_pairing.as_ref()?;
        let prompt = pairing.prompt.as_ref();
        let requires_input = prompt.is_some_and(|prompt| {
            matches!(
                prompt.kind,
                rmac_bluetooth::PairingPromptKind::EnterPinCode
                    | rmac_bluetooth::PairingPromptKind::EnterPasskey
            )
        });
        let (instruction, code, primary_label) = if let Some(prompt) = prompt {
            match &prompt.kind {
                rmac_bluetooth::PairingPromptKind::ConfirmPasskey { passkey } => (
                    "Make sure this code is also shown on the Bluetooth device.",
                    Some(format!("{passkey:06}")),
                    "Pair",
                ),
                rmac_bluetooth::PairingPromptKind::EnterPinCode => (
                    "Enter the 1–16 character PIN supplied by the device.",
                    None,
                    "Continue",
                ),
                rmac_bluetooth::PairingPromptKind::EnterPasskey => (
                    "Enter the six-digit passkey shown on the device.",
                    None,
                    "Continue",
                ),
                rmac_bluetooth::PairingPromptKind::AuthorizePairing => (
                    "Allow this device to pair with this computer?",
                    None,
                    "Pair",
                ),
                rmac_bluetooth::PairingPromptKind::AuthorizeService { .. } => (
                    "Allow this paired device to use its requested Bluetooth service?",
                    None,
                    "Allow",
                ),
            }
        } else if let Some(display) = &pairing.display {
            match display {
                BluetoothPairingDisplay::PinCode(pin_code) => (
                    "Type this code on the Bluetooth device, then finish there.",
                    Some(pin_code.clone()),
                    "",
                ),
                BluetoothPairingDisplay::Passkey { passkey, entered } => (
                    "Type this code on the Bluetooth device, then finish there.",
                    Some(format!("{passkey:06} · {entered}/6 entered")),
                    "",
                ),
            }
        } else {
            ("Keep the device nearby and ready to pair.", None, "")
        };

        let content = div()
            .w(px(400.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Connect to “{}”?", pairing.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(instruction),
                    ),
            )
            .when_some(code, |dialog, code| {
                dialog.child(
                    div()
                        .w_full()
                        .text_center()
                        .text_size(rmac_ui::text_px(25.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(label())
                        .child(code),
                )
            })
            .when(requires_input, |dialog| {
                dialog.child(TextField::new(&pairing.editor).w_full())
            })
            .when_some(pairing.validation_error.clone(), |dialog, error| {
                dialog.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .when(prompt.is_none(), |dialog| {
                dialog.child(Progress::indeterminate().label(if pairing.stopping {
                    "Ending pairing…"
                } else {
                    "Pairing securely…"
                }))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "bluetooth-pairing-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(pairing.stopping)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_bluetooth_pairing(cx))),
                    )
                    .when(prompt.is_some(), |buttons| {
                        buttons
                            .child(
                                rmac_ui::dialog_button(
                                    "bluetooth-pairing-reject",
                                    "Don’t Pair",
                                    rmac_ui::DialogButtonKind::Normal,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.reject_bluetooth_pairing_prompt(cx),
                                )),
                            )
                            .child(
                                rmac_ui::dialog_button(
                                    "bluetooth-pairing-submit",
                                    primary_label,
                                    rmac_ui::DialogButtonKind::Primary,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.submit_bluetooth_pairing_prompt(window, cx)
                                    },
                                )),
                            )
                    }),
            );

        Some(
            rmac_ui::dialog("bluetooth-pairing-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_bluetooth_pairing(cx);
                        }
                        "enter"
                            if this
                                .bluetooth_pairing
                                .as_ref()
                                .is_some_and(|pairing| pairing.prompt.is_some()) =>
                        {
                            cx.stop_propagation();
                            this.submit_bluetooth_pairing_prompt(window, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    // ---- Date & Time -------------------------------------------------

    fn render_date_time(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-date-time", "Refresh")
            .busy(self.time_busy || self.time_stream_refreshing)
            .disabled(self.time_loading || self.time_busy || self.time_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_time(cx));
            });
        let Some(snapshot) = &self.time else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/clock.svg", secondary(), 22.0))
                    .child(text_block(
                        "System date and time".into(),
                        Some("systemd-timedated".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.time_loading {
                    "Reading authoritative date and time state from systemd-timedated…"
                } else {
                    "The system date and time service is unavailable. No local fallback controls are shown."
                }),
            ]);
        };

        let timezone_row = if let Some(editor) = &self.timezone_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Time zone".into(),
                    Some("Enter an exact system zone such as Asia/Kolkata".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("timezone-cancel", "Cancel")
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_timezone_edit(cx));
                        }),
                )
                .child(
                    Button::new("timezone-save", "Save")
                        .primary()
                        .busy(self.time_busy)
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_timezone(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Time zone".into(),
                    Some("Validated against zones installed on this system".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(snapshot.timezone.clone()),
                )
                .child(
                    Button::new("timezone-edit", "Edit")
                        .disabled(
                            self.time_busy
                                || self.clock_editor.is_some()
                                || self.clock_confirmation.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_timezone_edit(window, cx)
                            });
                        }),
                )
                .into_any_element()
        };

        let clock_row = if let Some(editor) = &self.clock_editor {
            let review_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/clock.svg", accent(), 22.0))
                .child(text_block(
                    "Set date and time".into(),
                    Some("Use YYYY-MM-DD HH:MM:SS ±HH:MM".into()),
                ))
                .child(div().w(px(250.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("clock-edit-cancel", "Cancel")
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_clock_edit(cx));
                        }),
                )
                .child(
                    Button::new("clock-edit-review", "Review…")
                        .primary()
                        .disabled(self.time_busy)
                        .on_click(move |_, _, cx| {
                            review_view
                                .update(cx, |settings, cx| settings.prepare_clock_change(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/clock.svg", accent(), 22.0))
                .child(text_block(
                    "Set date and time".into(),
                    Some(
                        if snapshot.ntp_enabled {
                            "Turn off automatic time to edit the system clock"
                        } else {
                            "Authorization may be required"
                        }
                        .into(),
                    ),
                ))
                .child(
                    Button::new("clock-edit", "Edit")
                        .disabled(
                            self.time_busy
                                || snapshot.ntp_enabled
                                || self.timezone_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view
                                .update(cx, |settings, cx| settings.start_clock_edit(window, cx));
                        }),
                )
                .into_any_element()
        };

        let ntp_view = view.clone();
        let automatic = Toggle::new("automatic-time")
            .checked(snapshot.ntp_enabled)
            .disabled(self.time_busy || !snapshot.can_ntp)
            .on_click(move |enabled, _, cx| {
                ntp_view.update(cx, |settings, cx| settings.set_automatic_time(*enabled, cx));
            });
        let synchronization = if !snapshot.can_ntp {
            "No synchronization service"
        } else if snapshot.synchronized {
            "Synchronized"
        } else if snapshot.ntp_enabled {
            "Synchronizing"
        } else {
            "Off"
        };
        let mut cards = vec![
            card(vec![
                value_row(
                    "icons/clock.svg",
                    secondary(),
                    "Current time".into(),
                    snapshot
                        .formatted_local_time_at(
                            current_system_time_usec().unwrap_or(snapshot.time_usec),
                        )
                        .into(),
                ),
                value_row(
                    "icons/refresh-cw.svg",
                    if snapshot.synchronized {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    "Synchronization".into(),
                    synchronization.into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                    .child(text_block(
                        "Set time automatically".into(),
                        Some(if snapshot.can_ntp {
                            "Use the system network time service".into()
                        } else {
                            "No compatible network time service is installed".into()
                        }),
                    ))
                    .child(automatic)
                    .into_any_element(),
                clock_row,
            ]),
            card(vec![timezone_row]),
            card(vec![
                value_row(
                    "icons/settings.svg",
                    secondary(),
                    "Hardware clock".into(),
                    if snapshot.local_rtc {
                        "Local time"
                    } else {
                        "UTC"
                    }
                    .into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Authoritative state".into(),
                        Some("Live timedated changes · refresh on demand".into()),
                    ))
                    .child(refresh)
                    .into_any_element(),
            ]),
        ];
        if snapshot.timezones_truncated {
            cards.push(note_card(
                "The installed time-zone inventory exceeded the bounded validation list.",
            ));
        }
        cards.push(note_card(
            "Manual input includes a numeric UTC offset so daylight-saving transitions are never guessed. The hardware-clock mode remains read-only; UTC is the recommended Linux configuration.",
        ));
        self.pane(cards)
    }

    // ---- Language & Region -------------------------------------------

    fn render_language_region(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-language-region", "Refresh")
            .busy(self.locale_busy || self.locale_stream_refreshing)
            .disabled(self.locale_loading || self.locale_busy || self.locale_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_locale(cx));
            });
        let Some(snapshot) = &self.locale else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/languages.svg", secondary(), 22.0))
                    .child(text_block(
                        "System language and formats".into(),
                        Some("systemd-localed".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.locale_loading {
                    "Reading authoritative locale state and installed locales…"
                } else {
                    "The system locale service is unavailable. No local fallback controls are shown."
                }),
            ]);
        };

        let language_row = if let Some(editor) = &self.locale_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), 22.0))
                .child(text_block(
                    "Language".into(),
                    Some("Enter an exact locale installed on this computer".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("locale-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_locale_edit(cx));
                        }),
                )
                .child(
                    Button::new("locale-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_locale(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), 22.0))
                .child(text_block(
                    "Language".into(),
                    Some("Validated against the system's installed locales".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(snapshot.language().to_owned()),
                )
                .child(
                    Button::new("locale-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.region_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_locale_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let region_row = if let Some(editor) = &self.region_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Region".into(),
                    Some("Sets date, number, currency, and regional formats".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("region-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_region_edit(cx));
                        }),
                )
                .child(
                    Button::new("region-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_region(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            let value = if snapshot.formats_are_mixed() {
                format!("Mixed · {}", snapshot.region_locale())
            } else {
                snapshot.region_locale().to_owned()
            };
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Region".into(),
                    Some("System-wide formats, independent of display language".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(value),
                )
                .child(
                    Button::new("region-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.locale_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_region_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let mut cards = vec![card(vec![language_row, region_row])];
        if let Some(editor) = &self.locale_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_language(value.trim()) {
                Ok(preview) => {
                    cards.push(section_header("Assignments applied to the system"));
                    cards.push(card(
                        preview
                            .iter()
                            .map(|assignment| {
                                value_row(
                                    "icons/settings.svg",
                                    secondary(),
                                    assignment.key.clone().into(),
                                    assignment.value.clone().into(),
                                )
                            })
                            .collect(),
                    ));
                    if preview
                        .iter()
                        .any(|assignment| assignment.key.starts_with("LC_"))
                    {
                        cards.push(note_card(
                            "Existing LC_* format overrides are preserved. Applying changes LANG only.",
                        ));
                    }
                }
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }
        if let Some(editor) = &self.region_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_region(value.trim()) {
                Ok(_) => cards.push(note_card(
                    "Applying changes regional date, number, currency, paper, address, telephone, and measurement formats without changing the display language or message locale.",
                )),
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }

        cards.push(section_header("Format examples"));
        let preview = snapshot.format_preview.as_ref();
        cards.push(card(vec![
            locale_preview_row(
                "icons/clock.svg",
                "Dates and times",
                locale_format(snapshot, "LC_TIME"),
                preview.map(|preview| preview.date_time.as_str()),
            ),
            locale_preview_row(
                "icons/info.svg",
                "Numbers",
                locale_format(snapshot, "LC_NUMERIC"),
                preview.map(|preview| preview.number.as_str()),
            ),
            locale_preview_row(
                "icons/database.svg",
                "Currency",
                locale_format(snapshot, "LC_MONETARY"),
                preview.map(|preview| preview.currency.as_str()),
            ),
            locale_preview_row(
                "icons/settings.svg",
                "Measurement",
                locale_format(snapshot, "LC_MEASUREMENT"),
                None,
            ),
        ]));
        if let Some(error) = &snapshot.format_preview_error {
            cards.push(note_card(format!(
                "Format examples are unavailable: {error}. Locale assignments remain authoritative."
            )));
        }

        let keyboard = if snapshot.x11_layout.is_empty() {
            "Not reported by systemd-localed".to_owned()
        } else {
            let mut value = snapshot.x11_layout.clone();
            if !snapshot.x11_variant.is_empty() {
                value.push_str(" · ");
                value.push_str(&snapshot.x11_variant);
            }
            value
        };
        let layout_authority = self.input.keyboard_layout_authority;
        let can_edit_layout = layout_authority
            == rmac_input::KeyboardLayoutAuthority::SystemLocaled
            && snapshot.x11_layouts_error.is_none()
            && !snapshot.installed_x11_layouts.is_empty();
        cards.push(section_header("Input sources"));
        let editing_keyboard = self.x11_layout_editor.is_some();
        let mut keyboard_rows = if let (Some(layout), Some(variant), Some(options)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
        ) {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            vec![
                row_base()
                    .child(tile("icons/keyboard.svg", accent(), 22.0))
                    .child(text_block(
                        "XKB layouts".into(),
                        Some("Comma-separated installed names, in switch order".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(layout).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), 22.0))
                    .child(text_block(
                        "Variants".into(),
                        Some("One entry per layout; empty entries are allowed".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(variant).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), 22.0))
                    .child(text_block(
                        "Switching options".into(),
                        Some("For example grp:ctrl_space_toggle".into()),
                    ))
                    .child(div().w(px(150.0)).child(TextField::new(options).small()))
                    .child(
                        Button::new("x11-keyboard-cancel", "Cancel")
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                cancel_view.update(cx, |settings, cx| {
                                    settings.cancel_x11_keyboard_edit(cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("x11-keyboard-apply", "Apply")
                            .primary()
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                apply_view.update(cx, |settings, cx| {
                                    settings.submit_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            ]
        } else {
            let edit_view = view.clone();
            vec![row_base()
                .child(tile("icons/keyboard.svg", accent(), 22.0))
                .child(text_block(
                    "Keyboard layouts".into(),
                    Some("systemd-localed default and switch order".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(keyboard),
                )
                .child(
                    Button::new("x11-keyboard-edit", "Edit")
                        .disabled(self.locale_busy || !can_edit_layout)
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_x11_keyboard_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()]
        };
        if !editing_keyboard {
            keyboard_rows.push(value_row(
                "icons/settings.svg",
                secondary(),
                "Switching options".into(),
                if snapshot.x11_options.is_empty() {
                    "Not configured".into()
                } else {
                    snapshot.x11_options.clone().into()
                },
            ));
        }
        keyboard_rows.push(value_row(
            "icons/keyboard.svg",
            secondary(),
            "Console keymap".into(),
            if snapshot.console_keymap.is_empty() {
                "Not configured".into()
            } else {
                snapshot.console_keymap.clone().into()
            },
        ));
        if self.x11_keyboard_revert.is_some() {
            let revert_view = view.clone();
            keyboard_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Previous keyboard layout".into(),
                        Some("Exact model, layouts, variants, and options".into()),
                    ))
                    .child(
                        Button::new("x11-keyboard-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(keyboard_rows));
        cards.push(note_card(match layout_authority {
            rmac_input::KeyboardLayoutAuthority::SystemLocaled => {
                "niri follows this systemd-localed layout because no explicit XKB override is present. Use the configured XKB switching option to move between multiple layouts."
            }
            rmac_input::KeyboardLayoutAuthority::NiriConfig => {
                "The niri config has an explicit XKB block, so it—not systemd-localed—owns this session's keyboard layout. The system default is read-only here to avoid overriding that choice."
            }
            rmac_input::KeyboardLayoutAuthority::IncludedConfig => {
                "A traversed niri include owns explicit XKB settings, so the systemd-localed default remains read-only here instead of competing with that configuration."
            }
            rmac_input::KeyboardLayoutAuthority::Unavailable => {
                "The active niri keyboard-layout authority could not be verified. The system default remains read-only."
            }
        }));
        if let Some(error) = &snapshot.x11_layouts_error {
            cards.push(note_card(format!(
                "Keyboard layout editing is unavailable: {error}."
            )));
        }
        if snapshot.installed_x11_layouts_truncated {
            cards.push(note_card(
                "The installed XKB layout inventory exceeded the bounded validation list.",
            ));
        }

        let mut authority_rows = vec![row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Authoritative state".into(),
                Some(
                    format!(
                        "Live localed changes · {} installed locales",
                        snapshot.installed_locales.len()
                    )
                    .into(),
                ),
            ))
            .child(refresh)
            .into_any_element()];
        if self.locale_revert.is_some() {
            let revert_view = view.clone();
            authority_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Previous locale assignments".into(),
                        Some("Reverts only if the complete applied state is still current".into()),
                    ))
                    .child(
                        Button::new("locale-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| settings.revert_locale(cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(authority_rows));
        if snapshot.installed_locales_truncated {
            cards.push(note_card(
                "The installed locale inventory exceeded the bounded validation list.",
            ));
        }
        cards.push(note_card(
            "New applications and services use an applied locale immediately. Sign out and back in before judging the current desktop session.",
        ));
        self.pane(cards)
    }

    // ---- Login Items -------------------------------------------------

    fn render_login_items(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-login-items", "Refresh")
            .busy(
                self.login_item_busy.as_deref() == Some("refresh")
                    || self.login_items_stream_refreshing,
            )
            .disabled(
                self.login_items_loading
                    || self.login_item_busy.is_some()
                    || self.login_items_stream_refreshing,
            )
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_login_items(cx));
            });
        let Some(snapshot) = &self.login_items else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/app-window.svg", secondary(), 22.0))
                    .child(text_block(
                        "Open at login".into(),
                        Some("XDG autostart directories".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.login_items_loading {
                    "Reading effective XDG autostart entries…"
                } else {
                    "Autostart entries are unavailable. No private fallback toggles are shown."
                }),
            ]);
        };

        let choose_view = view.clone();
        let mut cards = vec![section_header("Open at login")];
        cards.push(card(vec![row_base()
            .child(tile("icons/app-window.svg", accent(), 22.0))
            .child(text_block(
                "Add application entry".into(),
                Some("Choose a local .desktop file to review".into()),
            ))
            .child(
                Button::new("choose-login-item", "Add…")
                    .busy(self.login_item_busy.as_deref() == Some("choose"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        choose_view.update(cx, |settings, cx| settings.choose_login_item(cx));
                    }),
            )
            .into_any_element()]));
        if let Some(preview) = &self.login_item_add {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Review “{}” ({}). This command will be allowed to run at sign-in. {}",
                preview.name,
                preview.id,
                if preview.replacing {
                    "A user entry with this filename exists and will be replaced only after confirmation."
                } else {
                    "The validated entry will be copied into your user autostart directory."
                }
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Command at sign-in".into(),
                    Some(preview.command.clone().into()),
                ))
                .into_any_element()]));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    if preview.replacing {
                        "Replace existing login item"
                    } else {
                        "Add login item"
                    }
                    .into(),
                    Some("The installed copy will start enabled".into()),
                ))
                .child(
                    Button::new("cancel-add-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_add = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-add-login-item",
                        if preview.replacing { "Replace" } else { "Add" },
                    )
                    .primary()
                    .busy(self.login_item_busy.as_deref() == Some("add"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_add_login_item(cx));
                    }),
                )
                .into_any_element()]));
        }
        if snapshot.items.is_empty() {
            cards.push(note_card("No effective XDG autostart entries were found."));
        } else {
            let rows = snapshot
                .items
                .iter()
                .map(|item| {
                    let id = item.id.clone();
                    let reveal_id = item.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let remove_view = view.clone();
                    let remove_id = item.id.clone();
                    let busy = self.login_item_busy.as_deref() == Some(item.id.as_str());
                    let remove_key = format!("prepare-remove:{}", item.id);
                    let preparing_remove =
                        self.login_item_busy.as_deref() == Some(remove_key.as_str());
                    let reveal_key = format!("reveal:{}", item.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = item.session_detail.clone().unwrap_or_else(|| {
                        if item.user_owned {
                            "User autostart entry".into()
                        } else {
                            "System autostart entry".into()
                        }
                    });
                    row_base()
                        .child(tile("icons/app-window.svg", accent(), 22.0))
                        .child(text_block(item.name.clone().into(), Some(subtitle.into())))
                        .when(item.user_owned && !item.managed_override, |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "remove-login-item-{}",
                                        item.id
                                    ))),
                                    "Remove…",
                                )
                                .busy(preparing_remove)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    remove_view.update(cx, |settings, cx| {
                                        settings.request_remove_login_item(remove_id.clone(), cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "reveal-login-item-{}",
                                    item.id
                                ))),
                                "Show in Files",
                            )
                            .busy(revealing)
                            .disabled(self.login_item_busy.is_some())
                            .on_click(move |_, _, cx| {
                                reveal_view.update(cx, |settings, cx| {
                                    settings.reveal_login_item(reveal_id.clone(), false, cx);
                                });
                            }),
                        )
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "login-item-{}",
                                item.id
                            ))))
                            .checked(item.enabled)
                            .disabled(self.login_item_busy.is_some() || !item.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_login_item_enabled(id.clone(), *enabled, cx);
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(preview) = &self.login_item_remove {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Remove “{}”? Its user-owned desktop entry will be moved to Trash. If a system entry with the same filename exists, it will remain visible but disabled.",
                preview.name
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Confirm removal".into(),
                    Some("This does not delete the application itself".into()),
                ))
                .child(
                    Button::new("cancel-remove-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_remove = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new("confirm-remove-login-item", "Move to Trash")
                        .primary()
                        .busy(self.login_item_busy.as_deref() == Some("remove"))
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            confirm_view.update(cx, |settings, cx| {
                                settings.confirm_remove_login_item(cx);
                            });
                        }),
                )
                .into_any_element()]));
        }

        cards.push(section_header("Allow in background"));
        if snapshot.background_services.is_empty() {
            cards.push(note_card(if snapshot.background_services_error.is_some() {
                "The systemd user manager is unavailable. XDG application login items remain usable."
            } else {
                "No enabled or user-installed systemd background services were found."
            }));
        } else {
            let rows = snapshot
                .background_services
                .iter()
                .map(|service| {
                    let id = service.id.clone();
                    let reveal_id = service.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let busy_key = format!("systemd:{}", service.id);
                    let busy = self.login_item_busy.as_deref() == Some(busy_key.as_str());
                    let reveal_key = format!("reveal:{}", service.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = format!("{} · {}", service.detail, service.state.label());
                    row_base()
                        .child(tile("icons/settings.svg", secondary(), 22.0))
                        .child(text_block(
                            service.name.clone().into(),
                            Some(subtitle.into()),
                        ))
                        .when(service.source.is_some(), |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "reveal-background-service-{}",
                                        service.id
                                    ))),
                                    "Show in Files",
                                )
                                .busy(revealing)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    reveal_view.update(cx, |settings, cx| {
                                        settings.reveal_login_item(reveal_id.clone(), true, cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "background-service-{}",
                                service.id
                            ))))
                            .checked(service.enabled)
                            .disabled(self.login_item_busy.is_some() || !service.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_background_service_enabled(
                                        id.clone(),
                                        *enabled,
                                        cx,
                                    );
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(error) = &snapshot.background_services_error {
            cards.push(note_card(format!(
                "Background service status is unavailable: {error}"
            )));
        }
        if snapshot.background_services_truncated {
            cards.push(note_card(
                "The systemd user service inventory exceeded the bounded display limit.",
            ));
        }

        if !snapshot.issues.is_empty() {
            cards.push(section_header("Entries needing attention"));
            cards.push(card(
                snapshot
                    .issues
                    .iter()
                    .map(|issue| {
                        row_base()
                            .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                            .child(text_block(
                                issue.file.clone().into(),
                                Some(issue.detail.clone().into()),
                            ))
                            .into_any_element()
                    })
                    .collect(),
            ));
        }
        let refresh_row = row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Authoritative state".into(),
                Some("Live XDG files · systemd user unit changes".into()),
            ))
            .child(refresh)
            .into_any_element();
        cards.push(card(vec![refresh_row]));
        if snapshot.truncated {
            cards.push(note_card(
                "The autostart inventory exceeded the bounded display limit.",
            ));
        }
        cards.push(note_card(
            "Changes to systemd user services take effect at the next sign-in; this pane does not start or stop running services. Adding or removing systemd unit files remains an administrator workflow.",
        ));
        self.pane(cards)
    }

    // ---- Sharing -----------------------------------------------------

    fn render_sharing(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-sharing", "Refresh")
            .busy(self.sharing_busy)
            .disabled(self.sharing_loading || self.sharing_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_sharing(cx));
            });
        let Some(snapshot) = &self.sharing else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/globe.svg", secondary(), 22.0))
                    .child(text_block(
                        "Host sharing services".into(),
                        Some("systemd and firewall authority".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.sharing_loading {
                    "Reading authoritative sharing capabilities…"
                } else {
                    "Sharing services are unavailable. No local fallback toggles are shown."
                }),
            ]);
        };

        let remote = &snapshot.remote_login;
        let toggle_view = view.clone();
        let remote_toggle = Toggle::new("remote-login")
            .checked(remote.active && remote.enabled_at_boot)
            .disabled(self.sharing_busy || !remote.available)
            .on_click(move |enabled, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.sharing_confirmation = Some(*enabled);
                    settings.file_sharing_confirmation = None;
                    cx.notify();
                });
            });
        let mut cards = vec![
            section_header("Remote Login"),
            card(vec![
                row_base()
                    .child(tile("icons/key.svg", accent(), 22.0))
                    .child(text_block(
                        "Remote Login (SSH)".into(),
                        Some(if remote.available {
                            format!(
                                "{} · {}",
                                remote.service_state.as_deref().unwrap_or("unknown"),
                                if remote.enabled_at_boot {
                                    "starts at boot"
                                } else {
                                    "disabled at boot"
                                }
                            )
                            .into()
                        } else {
                            "OpenSSH server is not installed".into()
                        }),
                    ))
                    .child(remote_toggle)
                    .into_any_element(),
                value_row(
                    "icons/shield.svg",
                    if remote.firewall == rmac_sharing::FirewallState::Allows {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    "Firewall".into(),
                    remote.firewall.label("SSH").into(),
                ),
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Authoritative state".into(),
                        Some("Live ssh.service · systemd · UFW files".into()),
                    ))
                    .child(refresh)
                    .into_any_element(),
            ]),
        ];

        if let Some(enabled) = self.sharing_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(if enabled {
                "Turn on Remote Login? This enables and starts the system SSH service after administrator authorization. It does not change firewall rules or authentication policy."
            } else {
                "Turn off Remote Login? Existing SSH sessions may be disconnected, and remote access can be lost. This stops and disables the system SSH service after administrator authorization."
            }));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    if enabled {
                        "Confirm enabling Remote Login"
                    } else {
                        "Confirm disabling Remote Login"
                    }
                    .into(),
                    Some("Administrator authorization may be requested".into()),
                ))
                .child(
                    Button::new("cancel-remote-login", "Cancel")
                        .disabled(self.sharing_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.sharing_confirmation = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-remote-login",
                        if enabled { "Turn On" } else { "Turn Off" },
                    )
                    .primary()
                    .busy(self.sharing_busy)
                    .disabled(self.sharing_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_remote_login(cx));
                    }),
                )
                .into_any_element()]));
        }
        if remote.firewall != rmac_sharing::FirewallState::Allows {
            cards.push(note_card(
                remote.firewall_detail.clone().unwrap_or_else(|| {
                    "A running SSH service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
                }),
            ));
        }
        cards.push(section_header("File Sharing"));
        let file = &snapshot.file_sharing;
        let file_toggle_view = view.clone();
        let file_toggle = Toggle::new("file-sharing")
            .checked(file.active && file.enabled_at_boot)
            .disabled(self.sharing_busy || !file.available)
            .on_click(move |enabled, _, cx| {
                file_toggle_view.update(cx, |settings, cx| {
                    settings.file_sharing_confirmation = Some(*enabled);
                    settings.sharing_confirmation = None;
                    cx.notify();
                });
            });
        cards.push(card(vec![
            row_base()
                .child(tile("icons/hard-drive.svg", accent(), 22.0))
                .child(text_block(
                    "SMB File Sharing".into(),
                    Some(if file.available {
                        format!(
                            "{} · {}",
                            file.service_state.as_deref().unwrap_or("unknown"),
                            if file.enabled_at_boot {
                                "starts at boot"
                            } else {
                                "disabled at boot"
                            }
                        )
                        .into()
                    } else {
                        "Samba file server is not installed".into()
                    }),
                ))
                .child(file_toggle)
                .into_any_element(),
            value_row(
                "icons/shield.svg",
                if file.firewall == rmac_sharing::FirewallState::Allows {
                    hsl(0x34c759)
                } else {
                    secondary()
                },
                "Firewall".into(),
                file.firewall.label("Samba").into(),
            ),
            value_row(
                "icons/folder-symlink.svg",
                secondary(),
                "Effective shares".into(),
                if file.shares.is_empty() {
                    "None reported".into()
                } else {
                    format!("{} configured", file.shares.len()).into()
                },
            ),
        ]));
        if !file.shares.is_empty() {
            cards.push(card(
                file.shares
                    .iter()
                    .map(|share| {
                        value_row(
                            "icons/folder-symlink.svg",
                            secondary(),
                            share.name.clone().into(),
                            "SMB share".into(),
                        )
                    })
                    .collect(),
            ));
        }
        if let Some(enabled) = self.file_sharing_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(if enabled {
                "Turn on File Sharing? This enables and starts Samba after administrator authorization. Every effective configured share may become reachable under its existing access policy. Firewall rules, share definitions, file permissions, and credentials are not changed."
            } else {
                "Turn off File Sharing? Connected SMB clients may lose access immediately. This stops and disables Samba after administrator authorization without deleting share definitions."
            }));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    if enabled {
                        "Confirm enabling File Sharing"
                    } else {
                        "Confirm disabling File Sharing"
                    }
                    .into(),
                    Some("Administrator authorization may be requested".into()),
                ))
                .child(
                    Button::new("cancel-file-sharing", "Cancel")
                        .disabled(self.sharing_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.file_sharing_confirmation = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-file-sharing",
                        if enabled { "Turn On" } else { "Turn Off" },
                    )
                    .primary()
                    .busy(self.sharing_busy)
                    .disabled(self.sharing_busy)
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_file_sharing(cx));
                    }),
                )
                .into_any_element()]));
        }
        if file.shares_truncated {
            cards.push(note_card(
                "The effective Samba share inventory exceeded the bounded display limit.",
            ));
        }
        if let Some(error) = &file.configuration_error {
            cards.push(note_card(error.clone()));
        }
        if file.firewall != rmac_sharing::FirewallState::Allows {
            cards.push(note_card(file.firewall_detail.clone().unwrap_or_else(|| {
                "A running SMB service does not prove that other computers can reach it. Network and router firewalls remain separate authorities.".into()
            })));
        }
        cards.push(note_card(
            "The switch controls only smbd.service runtime and boot state. Share definitions, permissions, credentials, and firewall policy remain separate authorities. rmac does not present AirDrop because Linux has no compatible local authority.",
        ));
        self.pane(cards)
    }

    // ---- Accessibility -----------------------------------------------

    fn render_accessibility(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let gtk_refresh_view = view.clone();
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/accessibility.svg", accent(), 22.0))
            .child(text_block(
                "Visual preferences".into(),
                Some("Live across rmac apps and shell surfaces".into()),
            ))
            .child(
                Button::new("accessibility-refresh", "Refresh")
                    .busy(self.theme_busy || self.theme_stream_refreshing)
                    .disabled(self.theme_loading || self.theme_busy || self.theme_stream_refreshing)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_theme(cx));
                    }),
            )
            .into_any_element()])];
        if self.theme_loading {
            cards.push(note_card("Loading accessibility preferences…"));
            return self.pane(cards);
        }
        if let Some(theme) = &self.theme {
            let preferences = &theme.preferences;
            cards.push(section_header("Vision"));
            cards.push(card(vec![
                theme_segment_row(
                    view.clone(),
                    "accessibility-contrast",
                    "Display contrast",
                    &THEME_CONTRAST_OPTIONS,
                    match preferences.contrast {
                        rmac_theme::ContrastPreference::Automatic => 0,
                        rmac_theme::ContrastPreference::Normal => 1,
                        rmac_theme::ContrastPreference::Higher => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                theme_segment_row(
                    view.clone(),
                    "accessibility-motion",
                    "Interface motion",
                    &THEME_MOTION_OPTIONS,
                    match preferences.motion {
                        rmac_theme::MotionPreferenceSetting::Automatic => 0,
                        rmac_theme::MotionPreferenceSetting::Full => 1,
                        rmac_theme::MotionPreferenceSetting::Reduced => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                theme_segment_row(
                    view.clone(),
                    "accessibility-text-scale",
                    "Text size",
                    &THEME_TEXT_SCALE_OPTIONS,
                    match preferences.text_scale {
                        rmac_theme::TextScalePreference::Standard => 0,
                        rmac_theme::TextScalePreference::Large => 1,
                        rmac_theme::TextScalePreference::ExtraLarge => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                value_row(
                    "icons/info.svg",
                    secondary(),
                    "Effective visual mode".into(),
                    format!(
                        "{} contrast · {} motion · {}% text",
                        match theme.effective.contrast {
                            rmac_appearance::Contrast::Normal => "Normal",
                            rmac_appearance::Contrast::Higher => "Higher",
                        },
                        match theme.effective.motion {
                            rmac_appearance::MotionPreference::Full => "Full",
                            rmac_appearance::MotionPreference::Reduced => "Reduced",
                        },
                        (theme.effective.text_scale.factor() * 100.0).round() as u16,
                    )
                    .into(),
                ),
            ]));
        } else {
            cards.push(note_card(
                "The rmac visual accessibility preference service is unavailable.",
            ));
        }

        cards.push(section_header("GTK Application Text"));
        cards.push(card(vec![row_base()
            .child(tile("icons/app-window.svg", secondary(), 22.0))
            .child(text_block(
                "GTK text scaling".into(),
                Some("GNOME interface authority; separate from rmac and display scale".into()),
            ))
            .child(
                Button::new("gtk-text-refresh", "Refresh")
                    .busy(self.gtk_text_busy || self.gtk_text_stream_refreshing)
                    .disabled(
                        self.gtk_text_loading
                            || self.gtk_text_busy
                            || self.gtk_text_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        gtk_refresh_view.update(cx, |settings, cx| settings.refresh_gtk_text(cx));
                    }),
            )
            .into_any_element()]));
        if self.gtk_text_loading {
            cards.push(note_card("Loading GTK text scaling from GSettings…"));
        } else if let Some(snapshot) = &self.gtk_text {
            if snapshot.available {
                let selected = GTK_TEXT_SCALE_OPTIONS
                    .iter()
                    .position(|(_, factor)| (snapshot.factor - factor).abs() < 0.001);
                cards.push(card(vec![
                    gtk_text_scale_row(
                        view.clone(),
                        selected,
                        snapshot.writable
                            && !self.gtk_text_busy
                            && !self.gtk_text_stream_refreshing,
                    ),
                    value_row(
                        "icons/app-window.svg",
                        secondary(),
                        "Effective GTK text".into(),
                        format!("{}%", (snapshot.factor * 100.0).round() as u16).into(),
                    ),
                ]));
                if let Some(detail) = &snapshot.detail {
                    cards.push(note_card(detail.clone()));
                }
            } else {
                cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                    "The GNOME interface text-scaling authority is unavailable.".into()
                })));
            }
        }

        let keyboard_view = view.clone();
        let mouse_view = view.clone();
        let screen_reader_refresh_view = view.clone();
        let trackpad_view = view;
        cards.push(section_header("Motor"));
        cards.push(card(vec![
            row_base()
                .child(tile("icons/keyboard.svg", secondary(), 22.0))
                .child(text_block(
                    "Keyboard".into(),
                    Some("Repeat timing and layout controls backed by niri".into()),
                ))
                .child(
                    Button::new("accessibility-keyboard", "Open").on_click(move |_, _, cx| {
                        keyboard_view
                            .update(cx, |settings, cx| settings.select_category("Keyboard", cx));
                    }),
                )
                .into_any_element(),
            row_base()
                .child(tile("icons/mouse.svg", secondary(), 22.0))
                .child(text_block(
                    "Pointer".into(),
                    Some("Speed, acceleration, handedness, and scroll controls".into()),
                ))
                .child(
                    Button::new("accessibility-mouse", "Mouse").on_click(move |_, _, cx| {
                        mouse_view.update(cx, |settings, cx| settings.select_category("Mouse", cx));
                    }),
                )
                .child(Button::new("accessibility-trackpad", "Trackpad").on_click(
                    move |_, _, cx| {
                        trackpad_view
                            .update(cx, |settings, cx| settings.select_category("Trackpad", cx));
                    },
                ))
                .into_any_element(),
        ]));

        if !self.input_loading {
            let keyboard = &self.input.settings.keyboard;
            let selected_preset = KEYBOARD_RESPONSE_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::KeyboardRepeatPreset { delay_ms, rate }
                        if *delay_ms == keyboard.repeat_delay_ms && *rate == keyboard.repeat_rate
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-key-response",
                    "Key repeat preset",
                    &KEYBOARD_RESPONSE_PRESETS,
                    selected_preset,
                    self.input.can_configure && !self.input_busy,
                ),
                value_row(
                    "icons/keyboard.svg",
                    secondary(),
                    "Effective key repeat".into(),
                    format!(
                        "{} ms delay · {} characters/s",
                        keyboard.repeat_delay_ms, keyboard.repeat_rate
                    )
                    .into(),
                ),
            ]));
            let mouse = &self.input.settings.mouse;
            let mouse_writable = self.input.can_configure && mouse.enabled && !self.input_busy;
            let selected_pointer_preset = MOUSE_PRECISION_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::MousePrecisionPreset { speed, profile }
                        if *speed == mouse.accel_speed && *profile == mouse.accel_profile
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-pointer-precision",
                    "Mouse precision",
                    &MOUSE_PRECISION_PRESETS,
                    selected_pointer_preset,
                    mouse_writable,
                ),
                input_switch_row(
                    cx.entity(),
                    "accessibility-middle-emulation",
                    "icons/mouse.svg",
                    "Middle-button emulation",
                    Some("Press the left and right mouse buttons together"),
                    mouse.middle_emulation,
                    mouse_writable,
                    InputChange::MouseMiddleEmulation,
                ),
                value_row(
                    "icons/mouse.svg",
                    secondary(),
                    "Effective mouse response".into(),
                    format!(
                        "{} acceleration · speed {}",
                        mouse.accel_profile.label(),
                        mouse.accel_speed
                    )
                    .into(),
                ),
            ]));
            if let Some(detail) = self
                .input
                .detail
                .clone()
                .filter(|_| !self.input.can_configure)
            {
                cards.push(note_card(detail));
            }
        } else {
            cards.push(note_card(
                "Loading keyboard accessibility settings from niri…",
            ));
        }
        cards.push(note_card(
            "Niri currently provides repeat timing but no compositor authority for Sticky Keys, Slow Keys, or Bounce Keys. Those controls remain unavailable instead of being simulated inside individual apps.",
        ));
        cards.push(note_card(
            "Mouse precision and middle-button emulation are applied by niri through libinput. Niri does not currently provide Mouse Keys, dwell click, or a session-wide double-click timing authority, so those controls remain unavailable.",
        ));

        cards.push(section_header("Text & Screen Reader"));
        cards.push(note_card(
            "Text size applies live to shared controls and app-owned interface text across the current rmac apps. It does not change GTK, browser, editor or terminal content fonts, display scaling, or compositor scaling.",
        ));
        let screen_reader = &self.screen_reader;
        let enabled_output = self
            .dock_compositor
            .outputs
            .values()
            .any(rmac_compositor::Output::enabled);
        let prerequisites_present =
            screen_reader.prerequisites_present(enabled_output) && !self.screen_reader_loading;
        cards.push(card(vec![
            row_base()
                .child(tile(
                    "icons/accessibility.svg",
                    if prerequisites_present {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    22.0,
                ))
                .child(text_block(
                    "Niri/Orca prerequisites".into(),
                    Some(if self.screen_reader_loading {
                        "Checking…".into()
                    } else if prerequisites_present {
                        "Detected".into()
                    } else {
                        "Incomplete".into()
                    }),
                ))
                .child(
                    Button::new("refresh-screen-reader", "Refresh")
                        .busy(self.screen_reader_loading)
                        .disabled(self.screen_reader_loading)
                        .on_click(move |_, _, cx| {
                            screen_reader_refresh_view
                                .update(cx, |settings, cx| settings.refresh_screen_reader(cx));
                        }),
                )
                .into_any_element(),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Desktop session".into(),
                if screen_reader.niri_session {
                    "Full niri session".into()
                } else {
                    "Not a full niri session".into()
                },
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Enabled display".into(),
                if enabled_output {
                    "Detected from niri".into()
                } else {
                    "Not detected".into()
                },
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Xwayland display for Orca".into(),
                if screen_reader.x11_display {
                    "DISPLAY exported".into()
                } else {
                    "Unavailable".into()
                },
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "xwayland-satellite".into(),
                if screen_reader.xwayland_satellite_installed {
                    "Executable in PATH".into()
                } else if screen_reader.x11_display {
                    "Custom X11 path exported".into()
                } else {
                    "Not found in PATH".into()
                },
            ),
            value_row(
                "icons/accessibility.svg",
                secondary(),
                "Orca".into(),
                if screen_reader.orca_installed {
                    "Installed".into()
                } else {
                    "Not found in PATH".into()
                },
            ),
            value_row(
                "icons/keyboard.svg",
                accent(),
                "Niri default shortcut".into(),
                "Super–Alt–S".into(),
            ),
        ]));
        if self.screen_reader_loading {
            cards.push(note_card("Checking niri and Orca prerequisites…"));
        } else if let Some(limitation) = screen_reader.limitation(enabled_output) {
            cards.push(note_card(limitation));
        } else {
            cards.push(note_card(
                "The detectable prerequisites are present. Environment checks cannot prove working EGL, speech output, the configured shortcut, or application semantics; test all four on the Linux PC.",
            ));
        }
        cards.push(note_card(
            "Niri does not currently provide built-in desktop zoom or a screen curtain. Those controls remain unavailable instead of being simulated by rmac.",
        ));
        cards.push(note_card(
            "This readiness check covers niri and Orca only. rmac application roles, names, states, actions, focus, and announcements still require Linux AT-SPI/Orca runtime evidence before accessibility can be claimed.",
        ));
        self.pane(cards)
    }

    // ---- Privacy & Security -----------------------------------------

    fn render_privacy_security(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let updates_view = view.clone();
        let coverage_view = view.clone();
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/shield.svg", accent(), 22.0))
            .child(text_block(
                "Portal permission decisions".into(),
                Some("Camera and microphone decisions stored by XDG portals".into()),
            ))
            .child(
                Button::new("privacy-refresh", "Refresh")
                    .busy(self.privacy_loading || self.privacy_stream_refreshing)
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_privacy(cx));
                    }),
            )
            .into_any_element()])];

        if self.privacy_loading {
            cards.push(note_card(
                "Loading decisions from the portal PermissionStore…",
            ));
        } else if let Some(snapshot) = &self.privacy {
            if snapshot.available {
                cards.push(card(vec![value_row(
                    "icons/info.svg",
                    secondary(),
                    "PermissionStore interface".into(),
                    format!("Version {}", snapshot.version).into(),
                )]));
                for resource in [
                    rmac_privacy::PortalResource::Camera,
                    rmac_privacy::PortalResource::Microphone,
                ] {
                    cards.push(section_header(resource.label()));
                    let decisions = snapshot
                        .decisions
                        .iter()
                        .filter(|decision| decision.resource == resource)
                        .cloned()
                        .collect::<Vec<_>>();
                    if decisions.is_empty() {
                        cards.push(note_card(format!(
                            "No stored {} decisions. This does not prove that native or already-running applications lack access.",
                            resource.label().to_lowercase()
                        )));
                        continue;
                    }
                    let rows = decisions
                        .into_iter()
                        .map(|decision| {
                            let identity = self.application_identity(&decision.app_id);
                            let display_name = identity
                                .map(|application| application.name.as_str())
                                .unwrap_or(&decision.app_id)
                                .to_owned();
                            let detail = format!(
                                "{} · Stored tokens: {}",
                                decision.app_id,
                                decision.summary()
                            );
                            let reset_view = view.clone();
                            let reset_decision = decision.clone();
                            let busy = self.privacy_busy.as_ref().is_some_and(
                                |(busy_resource, busy_app)| {
                                    *busy_resource == decision.resource
                                        && busy_app == &decision.app_id
                                },
                            );
                            row_base()
                                .child(tile("icons/app-window.svg", secondary(), 22.0))
                                .child(text_block(display_name.into(), Some(detail.into())))
                                .child(
                                    Button::new(
                                        SharedString::from(format!(
                                            "privacy-reset-{}-{}",
                                            decision.resource.id(),
                                            decision.app_id
                                        )),
                                        "Reset",
                                    )
                                    .busy(busy)
                                    .disabled(
                                        !snapshot.can_reset
                                            || self.privacy_busy.is_some()
                                            || self.privacy_stream_refreshing,
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            reset_view.update(cx, |settings, cx| {
                                                settings.request_privacy_reset(
                                                    reset_decision.clone(),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                )
                                .into_any_element()
                        })
                        .collect();
                    cards.push(card(rows));
                }
                if let Some(detail) = &snapshot.detail {
                    cards.push(note_card(detail.clone()));
                }
            } else {
                cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                    "The portal PermissionStore is unavailable in this session.".into()
                })));
            }
        }

        if let Some(decision) = &self.privacy_reset_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Reset the stored {} decision for {}? The next portal request may ask again. This does not terminate active access or change permissions for native applications.",
                decision.resource.label().to_lowercase(),
                decision.app_id
            )));
            cards.push(card(vec![row_base()
                .child(div().flex_1())
                .child(
                    Button::new("privacy-reset-cancel", "Cancel").on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| settings.cancel_privacy_reset(cx));
                    }),
                )
                .child(
                    rmac_ui::dialog_button(
                        "privacy-reset-confirm",
                        "Reset Decision",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_privacy_reset(cx));
                    }),
                )
                .into_any_element()]));
        }

        cards.push(section_header("Security Updates"));
        let security_status = if self.updates_loading && self.updates.is_none() {
            "Loading cached PackageKit status…".to_string()
        } else if let Some(snapshot) = &self.updates {
            let count = snapshot.security_count();
            if count == 0 {
                "No cached security updates".to_string()
            } else if count == 1 {
                "1 security update available".to_string()
            } else {
                format!("{count} security updates available")
            }
        } else {
            "PackageKit security status unavailable".to_string()
        };
        cards.push(card(vec![row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Available security updates".into(),
                Some(security_status.into()),
            ))
            .child(
                Button::new("privacy-open-updates", "Open").on_click(move |_, _, cx| {
                    updates_view.update(cx, |settings, cx| {
                        settings.push(SubPage::SoftwareUpdate, cx);
                    });
                }),
            )
            .into_any_element()]));

        cards.push(section_header("Ubuntu Security Coverage"));
        cards.push(card(vec![row_base()
            .child(tile("icons/shield.svg", accent(), 22.0))
            .child(text_block(
                "Installed package security".into(),
                Some("Ubuntu Pro Client · local machine-readable authorities".into()),
            ))
            .child(
                Button::new("privacy-refresh-coverage", "Refresh")
                    .busy(self.security_coverage_loading)
                    .disabled(self.security_coverage_loading)
                    .on_click(move |_, _, cx| {
                        coverage_view.update(cx, |settings, cx| {
                            settings.refresh_security_coverage(cx);
                        });
                    }),
            )
            .into_any_element()]));
        if self.security_coverage_loading && self.security_coverage.is_none() {
            cards.push(note_card(
                "Reading package origins, Ubuntu Pro services, and unattended-upgrades status…",
            ));
        } else if let Some(coverage) = &self.security_coverage {
            if let Some(release) = &coverage.release_support {
                let status = if release.days_remaining > 0 {
                    format!(
                        "Standard support · {} days remaining",
                        release.days_remaining
                    )
                } else if release.days_remaining == 0 {
                    "Standard support ends today".to_string()
                } else {
                    format!(
                        "Standard support ended {} days ago",
                        release.days_remaining.unsigned_abs()
                    )
                };
                cards.push(card(vec![value_row(
                    "icons/shield.svg",
                    if release.supported() {
                        accent()
                    } else {
                        secondary()
                    },
                    format!("Ubuntu {} lifecycle", release.series).into(),
                    status.into(),
                )]));
            }
            if let Some(sources) = &coverage.package_sources {
                cards.push(card(vec![
                    value_row(
                        "icons/info.svg",
                        secondary(),
                        "Installed APT packages".into(),
                        sources.installed.to_string().into(),
                    ),
                    value_row(
                        "icons/shield.svg",
                        accent(),
                        "Ubuntu archive".into(),
                        format!(
                            "{} Main/Restricted · {} Universe/Multiverse",
                            sources.main.saturating_add(sources.restricted),
                            sources.universe.saturating_add(sources.multiverse)
                        )
                        .into(),
                    ),
                    value_row(
                        "icons/shield.svg",
                        accent(),
                        "Ubuntu Pro archives".into(),
                        format!(
                            "{} ESM Infra · {} ESM Apps",
                            sources.esm_infra, sources.esm_apps
                        )
                        .into(),
                    ),
                    value_row(
                        "icons/app-window.svg",
                        secondary(),
                        "Other package origins".into(),
                        format!(
                            "{} third-party · {} unknown",
                            sources.third_party, sources.unknown
                        )
                        .into(),
                    ),
                ]));
            }
            if let Some(pro) = &coverage.pro {
                let contract = if pro.contract_valid {
                    format!(
                        "Valid · {} days remaining",
                        pro.contract_remaining_days.max(0)
                    )
                } else if pro.attached {
                    format!(
                        "Attached but not valid{}",
                        pro.contract_status
                            .as_deref()
                            .map(|status| format!(" · {status}"))
                            .unwrap_or_default()
                    )
                } else {
                    "Not attached".to_string()
                };
                let services = if pro.enabled_services.is_empty() {
                    "No Ubuntu Pro services enabled".to_string()
                } else {
                    format!("Enabled: {}", pro.enabled_services.join(", "))
                };
                cards.push(card(vec![value_row(
                    "icons/shield.svg",
                    if pro.contract_valid {
                        accent()
                    } else {
                        secondary()
                    },
                    "Ubuntu Pro contract".into(),
                    format!("{contract} · {services}").into(),
                )]));
            }
            if let Some(automatic) = &coverage.automatic_updates {
                let status = if automatic.fully_enabled() {
                    format!(
                        "Enabled · every {} day(s)",
                        automatic.upgrade_frequency_days
                    )
                } else {
                    automatic
                        .disabled_reason
                        .clone()
                        .unwrap_or_else(|| "Not fully enabled".into())
                };
                cards.push(card(vec![value_row(
                    "icons/refresh-cw.svg",
                    if automatic.fully_enabled() {
                        accent()
                    } else {
                        secondary()
                    },
                    "Automatic security updates".into(),
                    status.into(),
                )]));
                cards.push(note_card(format!(
                    "Allowed unattended-upgrade origins: {}. APT timer: {} · periodic job: {} · package-list refresh: every {} day(s){}.",
                    if automatic.allowed_origins.is_empty() {
                        "none reported".to_string()
                    } else {
                        automatic.allowed_origins.join(", ")
                    },
                    if automatic.apt_timer_enabled { "enabled" } else { "disabled" },
                    if automatic.periodic_job_enabled { "enabled" } else { "disabled" },
                    automatic.package_list_frequency_days,
                    automatic
                        .last_run
                        .as_deref()
                        .map(|last_run| format!(" · last run {last_run}"))
                        .unwrap_or_default()
                )));
            }
            for issue in &coverage.issues {
                cards.push(note_card(issue.clone()));
            }
            if !coverage.pro_client_available {
                cards.push(note_card(
                    "Ubuntu Pro Client is unavailable or does not provide the required offline API endpoints on this system.",
                ));
            }
        }
        cards.push(section_header("Desktop Application Sources"));
        let application_sources = rmac_apps::source_inventory(&self.app_catalog);
        cards.push(card(vec![
            value_row(
                "icons/app-window.svg",
                accent(),
                "Desktop-visible applications".into(),
                application_sources.total().to_string().into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Sandbox package exports".into(),
                format!(
                    "{} Flatpak · {} Snap",
                    application_sources.flatpak, application_sources.snap
                )
                .into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Portable applications".into(),
                format!("{} AppImage", application_sources.appimage).into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Unattributed desktop entries".into(),
                format!(
                    "{} system · {} user · {} other",
                    application_sources.system_desktop_entries,
                    application_sources.user_desktop_entries,
                    application_sources.other_desktop_entries
                )
                .into(),
            ),
        ]));
        cards.push(note_card(
            "Application source counts cover the live desktop-entry catalog. Flatpak and Snap use their exported desktop-entry paths; AppImage uses integration IDs or the launch executable. System and user desktop entries are not claimed to be APT-owned, and command-line-only packages are outside this inventory.",
        ));
        cards.push(note_card(
            "Reset revalidates the selected version-2 application/resource tokens immediately before DeletePermission and proves absence afterward. PermissionStore has no atomic compare-and-delete operation, so a change after that preflight cannot be excluded. Tokens remain uninterpreted because the store does not define their meaning.",
        ));
        cards.push(note_card(
            "Ubuntu coverage and automatic-update values are read-only until a polkit-aware, rollback-safe APT policy editor is reviewed. Package and application source counts describe provenance signals, not repository trust, vulnerability status, coverage, or the security of an individual application.",
        ));
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
                    .text_size(rmac_ui::text_px(12.0))
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
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.theme_busy || self.theme_stream_refreshing {
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
        let enabled = !self.theme_busy && !self.theme_stream_refreshing;
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
                            .text_size(rmac_ui::text_px(12.0))
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
                .text_size(rmac_ui::text_px(11.0))
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
                            .text_size(rmac_ui::text_px(10.0))
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
                    .text_size(rmac_ui::text_px(12.0))
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
                "banners",
                "Show notification banners",
                Some("Show a banner when this application sends a notification"),
                policy.banners,
                busy || !policy.enabled,
                NotificationPolicyChange::Banners,
            ),
            notification_toggle_row(
                &view,
                app_id,
                "sounds",
                "Play sounds for notifications",
                Some("Allow this application to play its notification sound"),
                policy.sounds,
                busy || !policy.enabled,
                NotificationPolicyChange::Sounds,
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
            notification_toggle_row(
                &view,
                app_id,
                "urgent-through-focus",
                "Allow urgent notifications through Focus",
                Some("Only notifications marked urgent may bypass an active Focus"),
                policy.urgent_through_focus,
                busy || !policy.enabled,
                NotificationPolicyChange::UrgentThroughFocus,
            ),
        ]))
        .child(note_card(
            "Lock Screen preview controls stay hidden because the shipping swaylock provider cannot securely render notification content.",
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
                    .text_size(rmac_ui::text_px(12.0))
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
            let mut status = state
                .projection
                .mode_name
                .clone()
                .unwrap_or_else(|| "Off".into());
            if matches!(
                state.source,
                Some(rmac_focus::ActivationSource::Schedule(_))
            ) {
                status.push_str(" · Scheduled");
            }
            let mut current = row_base()
                .child(tile(
                    "icons/moon.svg",
                    if active { accent() } else { secondary() },
                    22.0,
                ))
                .child(text_block("Current Focus".into(), Some(status.into())));
            match focus_current_action(state.source.as_ref()) {
                FocusCurrentAction::TurnOffManual => {
                    let turn_off_view = view.clone();
                    current = current.child(
                        div()
                            .id("focus-turn-off")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                turn_off_view.update(cx, |settings, cx| settings.disable_focus(cx));
                            })
                            .child("Turn Off"),
                    );
                }
                FocusCurrentAction::EditSchedule(schedule_id) => {
                    let edit_view = view.clone();
                    let schedule_id = schedule_id.as_str().to_owned();
                    current = current.child(
                        div()
                            .id("focus-edit-active-schedule")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                edit_view.update(cx, |settings, cx| {
                                    settings.push(
                                        SubPage::FocusSchedule {
                                            schedule_id: schedule_id.clone(),
                                        },
                                        cx,
                                    );
                                });
                            })
                            .child("Edit Schedule"),
                    );
                }
                FocusCurrentAction::None => {}
            }
            cards.push(card(vec![current.into_any_element()]));
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
                        .text_size(rmac_ui::text_px(12.0))
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
                .text_size(rmac_ui::text_px(12.0))
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
                .text_size(rmac_ui::text_px(12.0))
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
                .text_size(rmac_ui::text_px(12.0))
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
        if let Some(error) = &self.lock_request_error {
            cards.push(note_card(error.clone()));
        }
        let lock_view = view.clone();
        cards.push(card(vec![row_base()
            .child(tile("icons/lock.svg", accent(), 22.0))
            .child(text_block(
                "Test Lock Screen".into(),
                Some(
                    "Securely hide this session now, then authenticate to return to Settings"
                        .into(),
                ),
            ))
            .child(
                Button::new(
                    "lock-screen-now",
                    if self.lock_request_busy {
                        "Locking…"
                    } else {
                        "Lock Now"
                    },
                )
                .disabled(self.lock_request_busy)
                .on_click(move |_, _, cx| {
                    lock_view.update(cx, |settings, cx| settings.request_lock_screen(cx));
                }),
            )
            .into_any_element()]));
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
                value_row(
                    "icons/bell.svg",
                    secondary(),
                    "Notification Previews".into(),
                    "Hidden".into(),
                ),
            ]));
            cards.push(note_card(
                "The current PAM-enabled swaylock provider cannot render notification content, so previews stay hidden even when an application policy would allow them. Lid close and suspend follow the system’s supported logind policy and always pass through the pre-sleep lock boundary.",
            ));
        }
        self.pane(cards)
    }

    // ---- Sound --------------------------------------------------------

    fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let out = self.output_volume.read(cx).value().start().round() as i32;
        let input = self.input_volume.read(cx).value().start().round() as i32;
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
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
                    .text_size(rmac_ui::text_px(12.0))
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
            if let Some(detail) = &self.audio.configuration_error {
                cards.push(note_card(detail.clone()));
            }
            if self.audio.has_output {
                let output_view = view.clone();
                let output_mute = Toggle::new("audio-output-mute")
                    .checked(self.audio.output.muted)
                    .on_click(move |muted, _, cx| {
                        output_view.update(cx, |settings, cx| {
                            settings.set_audio_muted(rmac_audio::DeviceKind::Output, *muted, cx)
                        });
                    });
                let mut output_card = div()
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
                    ));
                if self
                    .audio
                    .outputs
                    .iter()
                    .any(|device| device.is_default && device.balance.is_some())
                {
                    output_card = output_card
                        .child(div().h(px(1.0)).bg(sep()).mx_3())
                        .child(balance_slider_row(&self.output_balance));
                }
                output_card = output_card.child(div().h(px(1.0)).bg(sep()).mx_3()).child(
                    row_base()
                        .child(tile("icons/volume-2.svg", secondary(), 22.0))
                        .child(text_block("Mute output".into(), None))
                        .child(output_mute),
                );
                cards.push(output_card);
            } else {
                cards.push(note_card("No output device is currently active."));
            }

            if self.audio.has_input {
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
            } else {
                cards.push(note_card("No input device is currently active."));
            }
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
            cards.push(self.audio_route_card(
                "Output Ports",
                &self.audio.outputs,
                rmac_audio::DeviceKind::Output,
                cx,
            ));
            cards.push(self.audio_route_card(
                "Input Ports",
                &self.audio.inputs,
                rmac_audio::DeviceKind::Input,
                cx,
            ));
            cards.push(self.audio_profile_card(cx));
        }

        cards.push(note_card(
            "Output, microphone, defaults, ports, and device profiles use the system audio service. Session alert sounds and interface effects stay hidden until the rmac sound policy service exists.",
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
                let device = d.clone();
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
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(secondary())
                                .child("Default"),
                        )
                        .child(glyph("icons/check.svg", 13.0, accent()))
                    })
                    .when(self.audio.can_set_default && !d.is_default, |el| {
                        el.cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .on_click(move |_, _, cx| {
                                let device = device.clone();
                                device_view.update(cx, |settings, cx| {
                                    settings.set_default_audio_device(kind, device, cx)
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

    fn audio_route_card(
        &self,
        title: &'static str,
        devices: &[rmac_audio::Device],
        kind: rmac_audio::DeviceKind,
        cx: &Context<Self>,
    ) -> Div {
        let view = cx.entity();
        let rows: Vec<AnyElement> = devices
            .iter()
            .flat_map(|device| {
                device.routes.iter().map(|route| {
                    let row_device = device.clone();
                    let row_route = route.clone();
                    let route_view = view.clone();
                    let detail: SharedString = if route.availability.can_select() {
                        device.name.clone().into()
                    } else {
                        format!("{} · Unavailable", device.name).into()
                    };
                    row_base()
                        .id(ElementId::from(SharedString::from(format!(
                            "audio-route-{title}-{}-{}",
                            device.id, route.index
                        ))))
                        .child(tile("icons/volume-2.svg", accent(), 22.0))
                        .child(text_block(route.name.clone().into(), Some(detail)))
                        .when(route.is_active, |el| {
                            el.child(
                                div()
                                    .text_size(rmac_ui::text_px(12.0))
                                    .text_color(secondary())
                                    .child("Active"),
                            )
                            .child(glyph(
                                "icons/check.svg",
                                13.0,
                                accent(),
                            ))
                        })
                        .when(
                            audio_choice_is_actionable(
                                route.is_active,
                                route.availability,
                                self.audio_busy,
                            ),
                            |el| {
                                el.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let device = row_device.clone();
                                        let route = row_route.clone();
                                        route_view.update(cx, |settings, cx| {
                                            settings.set_audio_route(kind, device, route, cx)
                                        });
                                    })
                            },
                        )
                        .into_any_element()
                })
            })
            .collect();
        if rows.is_empty() {
            div()
        } else {
            div()
                .v_flex()
                .child(section_header(title))
                .child(card(rows))
        }
    }

    fn audio_profile_card(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let rows: Vec<AnyElement> = self
            .audio
            .hardware_devices
            .iter()
            .flat_map(|device| {
                device.profiles.iter().map(|profile| {
                    let row_device = device.clone();
                    let row_profile = profile.clone();
                    let profile_view = view.clone();
                    let detail: SharedString = if profile.availability.can_select() {
                        device.name.clone().into()
                    } else {
                        format!("{} · Unavailable", device.name).into()
                    };
                    row_base()
                        .id(ElementId::from(SharedString::from(format!(
                            "audio-profile-{}-{}",
                            device.id, profile.index
                        ))))
                        .child(tile("icons/settings.svg", accent(), 22.0))
                        .child(text_block(profile.name.clone().into(), Some(detail)))
                        .when(profile.is_active, |el| {
                            el.child(
                                div()
                                    .text_size(rmac_ui::text_px(12.0))
                                    .text_color(secondary())
                                    .child("Active"),
                            )
                            .child(glyph(
                                "icons/check.svg",
                                13.0,
                                accent(),
                            ))
                        })
                        .when(
                            audio_choice_is_actionable(
                                profile.is_active,
                                profile.availability,
                                self.audio_busy,
                            ),
                            |el| {
                                el.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let device = row_device.clone();
                                        let profile = row_profile.clone();
                                        profile_view.update(cx, |settings, cx| {
                                            settings.set_audio_profile(device, profile, cx)
                                        });
                                    })
                            },
                        )
                        .into_any_element()
                })
            })
            .collect();
        if rows.is_empty() {
            div()
        } else {
            div()
                .v_flex()
                .child(section_header("Device Profiles"))
                .child(card(rows))
        }
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
                    .text_size(rmac_ui::text_px(12.0))
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
                    .text_size(rmac_ui::text_px(12.0))
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

    fn input_devices_card(&self, kinds: &[rmac_input::DeviceKind], empty: &'static str) -> Div {
        let rows = self
            .input
            .devices
            .iter()
            .filter(|device| kinds.contains(&device.kind))
            .map(|device| {
                value_row(
                    match device.kind {
                        rmac_input::DeviceKind::Keyboard => "icons/keyboard.svg",
                        rmac_input::DeviceKind::Touchpad => "icons/touchpad.svg",
                        _ => "icons/mouse.svg",
                    },
                    secondary(),
                    device.name.clone().into(),
                    device.kind.label().into(),
                )
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            note_card(empty)
        } else {
            card(rows)
        }
    }

    fn has_input_devices(&self, kinds: &[rmac_input::DeviceKind]) -> bool {
        self.input
            .devices
            .iter()
            .any(|device| kinds.contains(&device.kind))
    }

    fn render_keyboard(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        cards.push(section_header("Connected Keyboards"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Keyboard],
            "No connected keyboard was reported by the Linux input subsystem.",
        ));
        let other_kinds = [
            rmac_input::DeviceKind::Tablet,
            rmac_input::DeviceKind::Touchscreen,
            rmac_input::DeviceKind::Other,
        ];
        if self.has_input_devices(&other_kinds) {
            cards.push(section_header("Other Input Devices"));
            cards.push(self.input_devices_card(&other_kinds, ""));
            cards.push(note_card(
                "Tablets, touchscreens, and unclassified kernel input devices are listed for live inventory; this page does not apply keyboard settings to them.",
            ));
        }
        let settings = &self.input.settings.keyboard;
        let selected_delay = KEYBOARD_DELAYS.iter().position(|(_, change)| {
            matches!(change, InputChange::KeyboardRepeatDelay(value) if *value == settings.repeat_delay_ms)
        });
        let selected_rate = KEYBOARD_RATES.iter().position(|(_, change)| {
            matches!(change, InputChange::KeyboardRepeatRate(value) if *value == settings.repeat_rate)
        });
        cards.push(section_header("Key Repeat"));
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-delay",
                "Delay until repeat",
                &KEYBOARD_DELAYS,
                selected_delay,
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-rate",
                "Key repeat rate",
                &KEYBOARD_RATES,
                selected_rate,
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
            "Changes are resolved across the positional include graph, candidate-validated, saved to an isolated final rmac include, and applied by niri's live reload.",
        ));
        if self.input.included_files > 0 {
            cards.push(note_card(format!(
                "Effective input values include {} recursively loaded niri configuration file(s).",
                self.input.included_files
            )));
        }
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
        self.pane(cards)
    }

    fn render_mouse(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.mouse;
        let writable = self.input.can_configure && settings.enabled && !self.input_busy;
        cards.push(section_header("Connected Mice"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Mouse],
            "No connected mouse was reported by the Linux input subsystem.",
        ));
        let other_pointer_kinds = [
            rmac_input::DeviceKind::Trackpoint,
            rmac_input::DeviceKind::Trackball,
        ];
        if self.has_input_devices(&other_pointer_kinds) {
            cards.push(section_header("Other Pointing Devices"));
            cards.push(self.input_devices_card(&other_pointer_kinds, ""));
            cards.push(note_card(
                "Pointing sticks and trackballs use separate niri device-type sections. They are shown for live inventory and are not changed by the Mouse controls below.",
            ));
        }
        if !settings.enabled {
            cards.push(note_card(
                "The niri mouse section is explicitly off. Its effective settings are preserved but cannot affect devices until that type is enabled in the configuration.",
            ));
        }
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "mouse-tracking",
                "Tracking speed",
                &MOUSE_SPEEDS,
                Some(speed_index(settings.accel_speed)),
                writable,
            ),
            input_segment_row(
                cx.entity(),
                "mouse-acceleration",
                "Acceleration",
                &MOUSE_PROFILES,
                Some(usize::from(
                    settings.accel_profile == rmac_input::AccelProfile::Flat,
                )),
                writable,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-natural-scroll",
                "icons/mouse.svg",
                "Natural scrolling",
                Some("Move content in the direction your finger travels"),
                settings.natural_scroll,
                writable,
                InputChange::MouseNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-left-handed",
                "icons/mouse.svg",
                "Primary button on right",
                Some("Swap the left and right mouse buttons"),
                settings.left_handed,
                writable,
                InputChange::MouseLeftHanded,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-middle-emulation",
                "icons/mouse.svg",
                "Middle-button emulation",
                Some("Press the left and right buttons together for middle click"),
                settings.middle_emulation,
                writable,
                InputChange::MouseMiddleEmulation,
            ),
        ]));
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
        self.pane(cards)
    }

    fn render_trackpad(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.touchpad;
        let writable = self.input.can_configure && settings.pointer.enabled && !self.input_busy;
        cards.push(section_header("Connected Trackpads"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Touchpad],
            "No connected trackpad was reported by the Linux input subsystem.",
        ));
        if !settings.pointer.enabled {
            cards.push(note_card(
                "The niri touchpad section is explicitly off. Its effective settings are preserved but cannot affect devices until that type is enabled in the configuration.",
            ));
        }
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "touchpad-tracking",
                "Tracking speed",
                &TOUCHPAD_SPEEDS,
                Some(speed_index(settings.pointer.accel_speed)),
                writable,
            ),
            input_segment_row(
                cx.entity(),
                "touchpad-acceleration",
                "Acceleration",
                &TOUCHPAD_PROFILES,
                Some(usize::from(
                    settings.pointer.accel_profile == rmac_input::AccelProfile::Flat,
                )),
                writable,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-tap",
                "icons/touchpad.svg",
                "Tap to click",
                None,
                settings.tap_to_click,
                writable,
                InputChange::TouchpadTap,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-natural-scroll",
                "icons/touchpad.svg",
                "Natural scrolling",
                Some("Move content in the direction your fingers travel"),
                settings.pointer.natural_scroll,
                writable,
                InputChange::TouchpadNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-dwt",
                "icons/keyboard.svg",
                "Ignore while typing",
                Some("Prevent accidental pointer movement while typing"),
                settings.disable_while_typing,
                writable,
                InputChange::TouchpadDwt,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-drag-lock",
                "icons/touchpad.svg",
                "Drag lock",
                Some("Keep dragging briefly after lifting your finger"),
                settings.drag_lock,
                writable,
                InputChange::TouchpadDragLock,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-left-handed",
                "icons/touchpad.svg",
                "Primary click on right",
                None,
                settings.pointer.left_handed,
                writable,
                InputChange::TouchpadLeftHanded,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-middle-emulation",
                "icons/touchpad.svg",
                "Middle-click emulation",
                Some("Press the left and right click areas together"),
                settings.pointer.middle_emulation,
                writable,
                InputChange::TouchpadMiddleEmulation,
            ),
        ]));
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
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
                    .text_size(rmac_ui::text_px(12.0))
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
                    .text_size(rmac_ui::text_px(12.0))
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

            cards.push(section_header("Charging"));
            match battery.charge_threshold.availability {
                rmac_power::ChargeThresholdAvailability::Available => {
                    let threshold = battery.charge_threshold.clone();
                    let threshold_view = view.clone();
                    let threshold_for_action = threshold.clone();
                    let toggle = Toggle::new("battery-charge-threshold")
                        .checked(threshold.enabled)
                        .disabled(self.power_busy || !threshold.can_change())
                        .on_click(move |enabled, _, cx| {
                            threshold_view.update(cx, |settings, cx| {
                                settings.set_charge_threshold(
                                    threshold_for_action.clone(),
                                    *enabled,
                                    cx,
                                );
                            });
                        });
                    cards.push(card(vec![row_base()
                        .child(tile("icons/battery-charging.svg", hsl(0x34c759), 22.0))
                        .child(text_block(
                            "Optimized Charging".into(),
                            Some(charge_threshold_description(&threshold).into()),
                        ))
                        .child(toggle)
                        .into_any_element()]));
                }
                rmac_power::ChargeThresholdAvailability::MultipleBatteries => {
                    cards.push(note_card(
                        "Optimized charging is unavailable for multiple system batteries until each battery can be controlled separately.",
                    ));
                }
                rmac_power::ChargeThresholdAvailability::Unsupported => {
                    cards.push(note_card(
                        "Optimized charging is unavailable because UPower reports no writable charge limit for this battery.",
                    ));
                }
            }

            cards.push(section_header("Battery Level"));
            match battery.history.availability {
                rmac_power::BatteryHistoryAvailability::Available
                    if battery.history.points.is_empty() =>
                {
                    cards.push(note_card(
                        "UPower supports battery history, but it has not recorded any charge samples in the last 24 hours.",
                    ));
                }
                rmac_power::BatteryHistoryAvailability::Available => {
                    cards.push(battery_history_card(&battery.history.points));
                }
                rmac_power::BatteryHistoryAvailability::TemporarilyUnavailable => {
                    cards.push(note_card(
                        "Recent battery history could not be read from UPower. Current battery state remains authoritative.",
                    ));
                }
                rmac_power::BatteryHistoryAvailability::Unsupported => {
                    cards.push(note_card(
                        "Recent battery history is unavailable because UPower does not provide it for this battery.",
                    ));
                }
            }
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
        let keep_view = view.clone();
        let confirmation_seconds = self
            .display_confirmation
            .as_ref()
            .map(|pending| pending.seconds_remaining);
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
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
                    .when_some(confirmation_seconds, |actions, seconds| {
                        actions
                            .child(
                                div()
                                    .id("display-keep")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.0))
                                    .text_size(rmac_ui::text_px(12.0))
                                    .text_color(accent())
                                    .cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .child(format!("Keep Changes ({seconds}s)"))
                                    .on_click(move |_, _, cx| {
                                        keep_view.update(cx, |settings, cx| {
                                            settings.keep_display_change(cx)
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .id("display-revert")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.0))
                                    .text_size(rmac_ui::text_px(12.0))
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
                            .text_size(rmac_ui::text_px(12.0))
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

        let enabled_outputs = self
            .display
            .outputs
            .iter()
            .filter(|output| output.logical.is_some())
            .count();
        if enabled_outputs > 1 {
            cards.push(section_header("Arrange"));
            cards.push(display_layout_preview(&self.display.outputs));
            if !self.display.mirror_supported {
                cards.push(note_card(
                    "niri does not provide native display mirroring. Displays remain extended; wl-mirror can mirror content without pretending it is a compositor layout mode.",
                ));
            }
        }
        let main_output = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .cloned();

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
                    .text_size(rmac_ui::text_px(12.0))
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
            if output.primary {
                rows.push(value_row(
                    "icons/monitor.svg",
                    accent(),
                    "Main Display".into(),
                    "This display".into(),
                ));
            } else if output.logical.is_some() && self.display.can_persist {
                let output_id = output.id.clone();
                let primary_view = view.clone();
                rows.push(
                    row_base()
                        .id(SharedString::from(format!("display-primary-{output_id}")))
                        .child(text_block(
                            "Use as Main Display".into(),
                            Some("Anchors rmac shell surfaces and niri startup focus".into()),
                        ))
                        .child(glyph("icons/chevron-right.svg", 14.0, secondary()))
                        .when(
                            !self.display_busy && self.display_confirmation.is_none(),
                            |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        primary_view.update(cx, |settings, cx| {
                                            settings.set_primary_display(output_id.clone(), cx)
                                        });
                                    })
                            },
                        )
                        .into_any_element(),
                );
            }
            cards.push(card(rows));

            let Some(logical) = output.logical.as_ref() else {
                continue;
            };
            if !self.display.can_configure {
                continue;
            }

            if !output.primary {
                if let (Some(main), Some(moving)) = (main_output.as_ref(), output.logical.as_ref())
                {
                    if let Some(anchor) = main.logical.as_ref() {
                        cards.push(section_header("Arrange relative to Main"));
                        let placement_rows = [
                            DisplayPlacement::Left,
                            DisplayPlacement::Right,
                            DisplayPlacement::Above,
                            DisplayPlacement::Below,
                        ]
                        .into_iter()
                        .filter_map(|placement| {
                            let (x, y) = relative_display_position(moving, anchor, placement)?;
                            let selected = moving.x == x && moving.y == y;
                            let output_id = output.id.clone();
                            let expected_output = output.clone();
                            let placement_view = view.clone();
                            Some(
                                row_base()
                                    .id(SharedString::from(format!(
                                        "display-position-{output_id}-{}",
                                        placement.label()
                                    )))
                                    .child(text_block(placement.label().into(), None))
                                    .when(selected, |row| {
                                        row.child(glyph("icons/check.svg", 14.0, accent()))
                                    })
                                    .when(!selected, |row| {
                                        row.cursor_pointer()
                                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                            .on_click(move |_, _, cx| {
                                                let change = DisplayChange::Position {
                                                    output: expected_output.clone(),
                                                    x,
                                                    y,
                                                };
                                                placement_view.update(cx, |settings, cx| {
                                                    settings.apply_display_change(change, cx)
                                                });
                                            })
                                    })
                                    .into_any_element(),
                            )
                        })
                        .collect();
                        cards.push(card(placement_rows));
                    }
                }
            }

            if (0.5..=4.0).contains(&logical.scale) {
                cards.push(section_header("Scale"));
                let scale_rows = [1.0, 1.25, 1.5, 1.75, 2.0]
                    .into_iter()
                    .map(|scale| {
                        let selected = (logical.scale - scale).abs() < 0.001;
                        let output_id = output.id.clone();
                        let expected_output = output.clone();
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
                                            output: expected_output.clone(),
                                            scale,
                                        };
                                        scale_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
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
                        let expected_output = output.clone();
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
                                            output: expected_output.clone(),
                                            transform: transform.clone(),
                                        };
                                        rotation_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(rotation_rows));
            }

            if output.current_mode().is_some() {
                cards.push(section_header("Resolution"));
                let mode_rows = output
                    .modes
                    .iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        let mode = *mode;
                        let selected = output.current_mode == Some(index);
                        let output_id = output.id.clone();
                        let expected_output = output.clone();
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
                                            output: expected_output.clone(),
                                            mode,
                                        };
                                        mode_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(mode_rows));
            }
        }

        if let Some(graphics) = &self.sysinfo.graphics {
            cards.push(section_header("Graphics"));
            cards.push(card(vec![value_row(
                "icons/settings.svg",
                secondary(),
                "Chipset".into(),
                graphics.clone().into(),
            )]));
        }
        if self.display.can_configure {
            if self.display.can_persist {
                cards.push(note_card(
                    "Mode, scale, rotation, and arrangement changes remain temporary until you choose Keep Changes. rmac then validates an owned niri include with the complete live layout before saving it.",
                ));
            } else if let Some(detail) = &self.display.persistence_detail {
                cards.push(note_card(detail.clone()));
            }
        }
        self.pane(cards)
    }

    // ---- Network ------------------------------------------------------

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
                        .text_size(rmac_ui::text_px(12.0))
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
                        .text_size(rmac_ui::text_px(12.0))
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

        if self.network_editor.is_some() {
            cards.extend(self.render_network_editor(cx));
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
                    .text_size(rmac_ui::text_px(12.0))
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
            if let Some(configuration) = &device.configuration {
                let edit_view = view.clone();
                let edit_interface = device.interface.clone();
                let edit_configuration = configuration.clone();
                let enabled = configuration.editable && !self.network_busy;
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            configuration
                                .limitation
                                .as_ref()
                                .map(|limitation| limitation.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-{}",
                                    device.interface
                                ))),
                                if configuration.editable {
                                    "Details…"
                                } else {
                                    "Read Only"
                                },
                            )
                            .disabled(!enabled)
                            .on_click(move |_, window, cx| {
                                edit_view.update(cx, |settings, cx| {
                                    settings.start_network_edit(
                                        edit_interface.clone(),
                                        edit_configuration.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            }),
                        )
                        .into_any_element(),
                );
            } else if let Some(error) = &device.configuration_error {
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            Some(error.clone().into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-unavailable-{}",
                                    device.interface
                                ))),
                                "Unavailable",
                            )
                            .disabled(true),
                        )
                        .into_any_element(),
                );
            } else {
                rows.push(
                    row_base()
                        .child(text_block(
                            "Connection Details".into(),
                            Some("Connect this interface to edit its active profile".into()),
                        ))
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "network-edit-inactive-{}",
                                    device.interface
                                ))),
                                "Unavailable",
                            )
                            .disabled(true),
                        )
                        .into_any_element(),
                );
            }
            cards.push(card(rows));
        }
        self.pane(cards)
    }

    fn render_network_editor(&self, cx: &Context<Self>) -> Vec<Div> {
        let Some(editor) = &self.network_editor else {
            return Vec::new();
        };
        let view = cx.entity();
        let busy = self.network_busy;
        let ipv4_values_enabled = !busy
            && !matches!(
                editor.ipv4_method,
                rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
            );
        let ipv6_values_enabled = !busy
            && !matches!(
                editor.ipv6_method,
                rmac_network::IpMethod::Disabled | rmac_network::IpMethod::LinkLocal
            );
        let proxy_values_enabled =
            !busy && editor.proxy_method == rmac_network::ProxyMethod::Automatic;
        let cancel_view = view.clone();
        let save_view = view.clone();
        let mut sections = vec![
            section_header(format!(
                "{} · {}",
                editor.configuration.name, editor.interface
            )),
            note_card(
                "IP and DNS changes are staged on the active connection and verified before the complete profile is saved. Proxy-only changes are saved atomically. On failure, rmac restores the previous authority only when no newer external edit would be overwritten.",
            ),
            section_header("IPv4"),
            card(vec![
                network_ip_method_row(
                    view.clone(),
                    rmac_network::IpFamily::V4,
                    &editor.ipv4_method,
                    !busy,
                ),
                network_field_row(
                    "Addresses",
                    "Comma-separated addresses with prefixes",
                    &editor.ipv4_addresses,
                    ipv4_values_enabled,
                ),
                network_field_row(
                    "Router",
                    "Optional default gateway",
                    &editor.ipv4_gateway,
                    ipv4_values_enabled,
                ),
                network_field_row(
                    "DNS Servers",
                    "Comma-separated IPv4 addresses",
                    &editor.ipv4_dns,
                    ipv4_values_enabled,
                ),
                network_dns_policy_row(
                    view.clone(),
                    rmac_network::IpFamily::V4,
                    editor.ipv4_ignore_auto_dns,
                    ipv4_values_enabled,
                ),
            ]),
            section_header("IPv6"),
            card(vec![
                network_ip_method_row(
                    view.clone(),
                    rmac_network::IpFamily::V6,
                    &editor.ipv6_method,
                    !busy,
                ),
                network_field_row(
                    "Addresses",
                    "Comma-separated addresses with prefixes",
                    &editor.ipv6_addresses,
                    ipv6_values_enabled,
                ),
                network_field_row(
                    "Router",
                    "Optional default gateway",
                    &editor.ipv6_gateway,
                    ipv6_values_enabled,
                ),
                network_field_row(
                    "DNS Servers",
                    "Comma-separated IPv6 addresses",
                    &editor.ipv6_dns,
                    ipv6_values_enabled,
                ),
                network_dns_policy_row(
                    view.clone(),
                    rmac_network::IpFamily::V6,
                    editor.ipv6_ignore_auto_dns,
                    ipv6_values_enabled,
                ),
            ]),
            section_header("Proxy"),
            card(vec![
                network_proxy_method_row(view.clone(), editor.proxy_method, !busy),
                network_field_row(
                    "Configuration URL",
                    "HTTP, HTTPS, or absolute file URL",
                    &editor.proxy_url,
                    proxy_values_enabled,
                ),
                network_proxy_browser_row(
                    view.clone(),
                    editor.proxy_browser_only,
                    proxy_values_enabled,
                ),
            ]),
        ];
        if let Some(error) = &editor.validation_error {
            sections.push(note_card(error.clone()));
        }
        sections.push(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .mb_3()
                .child(
                    Button::new("network-edit-cancel", "Cancel")
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_network_edit(cx));
                        }),
                )
                .child(
                    Button::new(
                        "network-edit-save",
                        if busy { "Applying…" } else { "Apply" },
                    )
                    .primary()
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |settings, cx| settings.submit_network_edit(cx));
                    }),
                ),
        );
        sections
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
        let refresh_label = if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            "Deleting…"
        } else if self.vpn_secret_busy {
            "Forgetting…"
        } else if self.vpn_secret_preparing {
            "Preparing…"
        } else if self.vpn_editor_busy {
            "Saving…"
        } else if self.vpn_editor_loading.is_some() {
            "Opening…"
        } else if self.vpn_editor.is_some() {
            "Editing…"
        } else if self.vpn_import_busy {
            "Importing…"
        } else if self.vpn_busy.is_some() {
            "Updating…"
        } else if self.vpn_refreshing {
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
                        .text_size(rmac_ui::text_px(12.0))
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
                        .text_size(rmac_ui::text_px(12.0))
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
        if self.vpn_editor_loading.is_some() {
            cards.push(note_card("Opening current VPN profile details…"));
        }
        if self.vpn_editor.is_some() {
            cards.extend(self.render_vpn_editor(cx));
        }
        if self.vpn.profiles.is_empty() {
            cards.push(note_card(
                "No VPN configurations are installed. Import a configuration below.",
            ));
        } else {
            let rows = self
                .vpn
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| {
                    let id = profile.id.clone();
                    let switch_id = id.clone();
                    let profile_view = view.clone();
                    let delete_id = id.clone();
                    let delete_view = view.clone();
                    let edit_id = id.clone();
                    let edit_view = view.clone();
                    let applying = self.vpn_busy.as_ref() == Some(&id);
                    let connecting = applying && self.vpn_cancellation.is_some();
                    let preparing_delete = self.vpn_delete_preparing.as_ref() == Some(&id);
                    let subtitle = if connecting {
                        format!("{} · Connecting…", profile.service)
                    } else if applying {
                        format!("{} · Disconnecting…", profile.service)
                    } else {
                        format!("{} · {}", profile.service, profile.state.label())
                    };
                    let control = if connecting {
                        Button::new(("vpn-stop", index), "Stop")
                            .on_click(move |_, _, cx| {
                                profile_view
                                    .update(cx, |settings, cx| settings.cancel_vpn_activation(cx));
                            })
                            .into_any_element()
                    } else {
                        Toggle::new(ElementId::from(SharedString::from(format!(
                            "vpn-profile-{index}"
                        ))))
                        .checked(profile.state.is_enabled())
                        .disabled(
                            self.vpn_busy.is_some()
                                || self.vpn_refreshing
                                || self.vpn_import_busy
                                || self.vpn_import_preview.is_some()
                                || self.vpn_editor_loading.is_some()
                                || self.vpn_editor_busy
                                || self.vpn_editor.is_some()
                                || self.vpn_delete_preparing.is_some()
                                || self.vpn_delete_busy
                                || self.vpn_delete_preview.is_some(),
                        )
                        .on_click(move |enabled, _, cx| {
                            let id = switch_id.clone();
                            profile_view.update(cx, |settings, cx| {
                                settings.set_vpn_enabled(id, *enabled, cx)
                            });
                        })
                        .into_any_element()
                    };
                    let controls = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(cfg!(target_os = "linux"), |controls| {
                            controls
                                .child(
                                    Button::new(
                                        ("vpn-edit", index),
                                        if self.vpn_editor_loading.as_ref() == Some(&id) {
                                            "Opening…"
                                        } else {
                                            "Details…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            let id = edit_id.clone();
                                            edit_view.update(cx, |settings, cx| {
                                                settings.start_vpn_edit(id, window, cx)
                                            });
                                        },
                                    ),
                                )
                                .child(
                                    Button::new(
                                        ("vpn-delete", index),
                                        if preparing_delete {
                                            "Preparing…"
                                        } else {
                                            "Delete…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            let id = delete_id.clone();
                                            delete_view.update(cx, |settings, cx| {
                                                settings.request_vpn_delete(id, cx)
                                            });
                                        },
                                    ),
                                )
                        })
                        .child(control);
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
                        .child(controls)
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        cards.push(section_header("Import Configuration"));
        if self.vpn_import_loading {
            cards.push(note_card("Checking installed VPN importers…"));
        } else if !self.vpn_import_capabilities.available {
            cards.push(note_card(
                self.vpn_import_capabilities
                    .limitation
                    .clone()
                    .unwrap_or_else(|| "VPN import is unavailable on this system".to_string()),
            ));
        } else if self.vpn_import_capabilities.plugins.is_empty() {
            cards.push(note_card(
                "No reviewed import-capable NetworkManager VPN plugins are installed.",
            ));
        } else {
            let import_rows = self
                .vpn_import_capabilities
                .plugins
                .iter()
                .enumerate()
                .map(|(index, capability)| {
                    let capability_id = capability.id.clone();
                    let import_view = view.clone();
                    row_base()
                        .child(tile("icons/key.svg", accent(), 20.0))
                        .child(text_block(
                            capability.name.clone().into(),
                            Some(capability.format_hint.clone().into()),
                        ))
                        .child(
                            Button::new(("vpn-import", index), "Import…")
                                .disabled(
                                    self.vpn_import_busy
                                        || self.vpn_busy.is_some()
                                        || self.vpn_import_preview.is_some()
                                        || self.vpn_editor_loading.is_some()
                                        || self.vpn_editor_busy
                                        || self.vpn_editor.is_some()
                                        || self.vpn_delete_preparing.is_some()
                                        || self.vpn_delete_busy
                                        || self.vpn_delete_preview.is_some(),
                                )
                                .on_click(move |_, _, cx| {
                                    let capability = capability_id.clone();
                                    import_view.update(cx, |settings, cx| {
                                        settings.choose_vpn_import(capability, cx)
                                    });
                                }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(import_rows));
            if let Some(limitation) = &self.vpn_import_capabilities.limitation {
                cards.push(note_card(limitation.clone()));
            }
        }
        cards.push(note_card(
            "Connections are controlled by the system network service. Authentication prompts are handled by the installed VPN plugin.",
        ));
        self.pane(cards)
    }

    fn render_vpn_editor(&self, cx: &Context<Self>) -> Vec<Div> {
        let Some(editor) = &self.vpn_editor else {
            return Vec::new();
        };
        let busy = self.vpn_editor_busy;
        let locked = busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some();
        let view = cx.entity();
        let persistent_view = view.clone();
        let authentication_view = view.clone();
        let authentication_configuration = editor.configuration.clone();
        let cancel_view = view.clone();
        let save_view = view.clone();
        let mut rows = vec![network_field_row(
            "Name",
            "Shown in VPN lists and connection menus",
            &editor.name,
            !locked,
        )];
        if editor.configuration.supports_vpn_options {
            rows.extend([
                network_field_row(
                    "Account Name",
                    "Optional non-secret username; passwords are not read here",
                    &editor.username,
                    !locked,
                ),
                row_base()
                    .child(text_block(
                        "Keep Connection".into(),
                        Some("Ask the VPN plugin to maintain the tunnel when supported".into()),
                    ))
                    .child(
                        Toggle::new("vpn-edit-persistent")
                            .checked(editor.persistent)
                            .disabled(locked)
                            .on_click(move |persistent, _, cx| {
                                persistent_view.update(cx, |settings, cx| {
                                    settings.set_vpn_editor_persistent(*persistent, cx)
                                });
                            }),
                    )
                    .into_any_element(),
                network_field_row(
                    "Connection Timeout",
                    "Seconds; 0 uses the VPN plugin default",
                    &editor.timeout,
                    !locked,
                ),
            ]);
        }
        if editor.configuration.supports_vpn_options {
            rows.push(
                row_base()
                    .child(tile("icons/key.svg", secondary(), 22.0))
                    .child(text_block(
                        "Saved Authentication".into(),
                        Some(
                            "Passwords, passphrases, and plugin tokens managed by NetworkManager"
                                .into(),
                        ),
                    ))
                    .child(
                        Button::new(
                            "vpn-forget-authentication",
                            if self.vpn_secret_preparing {
                                "Preparing…"
                            } else {
                                "Forget…"
                            },
                        )
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            let configuration = authentication_configuration.clone();
                            authentication_view.update(cx, |settings, cx| {
                                settings.request_vpn_secret_clear(configuration, cx)
                            });
                        }),
                    )
                    .into_any_element(),
            );
        } else {
            rows.push(value_row(
                "icons/key.svg",
                secondary(),
                "Private Key".into(),
                "Never cleared here".into(),
            ));
        }
        let mut sections = vec![
            section_header(format!("{} · Details", editor.configuration.name)),
            note_card(
                "This editor changes only typed, non-secret fields. NetworkManager and the installed VPN plugin keep the complete plugin configuration, passwords, certificates, and private keys unchanged.",
            ),
            card(rows),
        ];
        if !editor.configuration.supports_vpn_options {
            sections.push(note_card(format!(
                "{} profiles can be renamed here. Their protocol-specific configuration remains under NetworkManager's authority.",
                editor.configuration.service
            )));
        }
        if let Some(error) = &editor.validation_error {
            sections.push(note_card(error.clone()));
        }
        if busy {
            sections.push(
                div()
                    .mb_2()
                    .child(Progress::indeterminate().label("Saving VPN details…")),
            );
        }
        sections.push(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .mb_3()
                .child(
                    Button::new("vpn-edit-cancel", "Cancel")
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_vpn_edit(cx));
                        }),
                )
                .child(
                    Button::new("vpn-edit-save", if busy { "Saving…" } else { "Save" })
                        .primary()
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_vpn_edit(cx));
                        }),
                ),
        );
        sections
    }

    fn render_vpn_secret_clear_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_secret_preview.as_ref()?;
        let busy = self.vpn_secret_busy;
        let consequence = if preview.currently_connected {
            "The current connection will stay active. NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens, then the installed VPN plugin may ask for them after the next disconnect. This cannot be undone by rmac."
        } else {
            "NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens. The installed VPN plugin may ask for them on the next connection. This cannot be undone by rmac."
        };
        let content = div()
            .w(px(440.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Forget authentication for “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .child(note_card(
                "The VPN profile, server configuration, certificates, and current tunnel are not deleted. Native WireGuard private keys are never handled by this action.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Forgetting saved authentication…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_vpn_secret_clear(cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-confirm",
                            "Forget",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_vpn_secret_clear(cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-secret-clear-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_secret_clear(cx);
                        }
                        "enter" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_secret_clear(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_vpn_import_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_import_preview.as_ref()?;
        let busy = self.vpn_import_busy;
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Import “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(format!(
                                "{} recognized “{}”.",
                                preview.service, preview.source_name
                            )),
                    ),
            )
            .child(note_card(
                "The profile is temporary and cannot connect automatically. Import saves it to NetworkManager; passwords and keys remain under NetworkManager and the VPN plugin’s authority.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Finishing VPN import…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(false, cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-confirm",
                            "Import",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(true, cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-import-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(false, cx);
                        }
                        "enter" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(true, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_vpn_delete_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_delete_preview.as_ref()?;
        let busy = self.vpn_delete_busy;
        let consequence = if preview.will_disconnect {
            "The active connection will disconnect first. The saved profile and its NetworkManager-managed secrets will then be removed. This cannot be undone."
        } else {
            "The saved profile and its NetworkManager-managed secrets will be removed. This cannot be undone."
        };
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Delete “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Deleting VPN profile…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_vpn_delete(cx))),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-confirm",
                            "Delete",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_vpn_delete(cx))),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-delete-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_delete(cx);
                        }
                        "enter" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_delete(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_update_install_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let plan = self.updates_plan.as_ref()?;
        let requested = plan.requested.len();
        let changes = plan.changes.len();
        let installs = plan.change_count(rmac_updates::ChangeKind::Install);
        let removals = plan.change_count(rmac_updates::ChangeKind::Remove)
            + plan.change_count(rmac_updates::ChangeKind::Obsolete);
        let downgrades = plan.change_count(rmac_updates::ChangeKind::Downgrade);
        let destructive_preview = plan
            .changes
            .iter()
            .filter(|change| change.kind.is_destructive())
            .take(8)
            .map(|change| {
                format!(
                    "{} {} ({})",
                    change.name,
                    change.version,
                    change.kind.label()
                )
            })
            .collect::<Vec<_>>();
        let hidden_destructive = removals + downgrades - destructive_preview.len();
        let summary = format!(
            "PackageKit will apply {requested} requested updates through {changes} verified package changes. Dependencies are included in this preview."
        );
        let mut rows = vec![
            value_row(
                "icons/refresh-cw.svg",
                accent(),
                "Requested updates".into(),
                requested.to_string().into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Additional installs".into(),
                installs.to_string().into(),
            ),
        ];
        if removals > 0 {
            rows.push(value_row(
                "icons/shield.svg",
                hsl(0xff3b30),
                "Removals or replacements".into(),
                removals.to_string().into(),
            ));
        }
        if downgrades > 0 {
            rows.push(value_row(
                "icons/info.svg",
                hsl(0xff9500),
                "Downgrades".into(),
                downgrades.to_string().into(),
            ));
        }
        let view = cx.entity();
        let cancel_view = view.clone();
        let install_view = view.clone();
        let content = div()
            .w(px(460.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child("Install system updates?"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(summary),
                    ),
            )
            .child(card(rows))
            .when(plan.has_destructive_changes(), |dialog| {
                let mut warning = format!(
                    "This verified plan changes packages destructively: {}.",
                    destructive_preview.join(", ")
                );
                if hidden_destructive > 0 {
                    warning.push_str(&format!(" Plus {hidden_destructive} more shown in the counts above."));
                }
                dialog.child(note_card(warning))
            })
            .when_some(plan.restart.label(), |dialog, restart| {
                dialog.child(note_card(format!("Expected after installation: {restart}.")))
            })
            .child(note_card(
                "rmac installs only the exact revalidated plan and keeps PackageKit's trusted-only flag enabled. Authorization may be requested.",
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_update_plan(cx));
                        }),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-confirm",
                            "Install Updates",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(move |_, _, cx| {
                            install_view
                                .update(cx, |settings, cx| settings.confirm_update_plan(cx));
                        }),
                    ),
            );
        Some(
            rmac_ui::dialog("update-install-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_update_plan(cx);
                        }
                        "enter" => {
                            cx.stop_propagation();
                            this.confirm_update_plan(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    /// Direct per-volume capacity state from the mount service.
    fn storage_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let mut body = div().v_flex().child(card(vec![row_base()
            .child(tile("icons/hard-drive.svg", accent(), 22.0))
            .child(text_block(
                "Mounted volumes".into(),
                Some("System, removable, and network volumes".into()),
            ))
            .child(
                Button::new("refresh-storage", "Refresh")
                    .busy(self.storage_busy || self.storage_stream_refreshing)
                    .disabled(self.system_data_loading || self.storage_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_storage(cx));
                    }),
            )
            .into_any_element()]));

        if self.storage.is_empty() {
            return body.child(
                EmptyState::new("No storage volumes available")
                    .message("Refresh after the mount service becomes available")
                    .error(self.storage_error.is_some() || self.storage_stream_error.is_some()),
            );
        }

        for (index, volume) in self.storage.iter().enumerate() {
            let identity = volume.mount.identity.clone();
            let action_busy = self.storage_action_busy.as_deref() == Some(identity.as_str());
            let action_disabled = self.storage_action_busy.is_some() || self.storage_busy;
            let action_view = view.clone();
            body = body.child(section_header(volume.mount.name.clone()));
            let Some(usage) = volume.usage else {
                body = body.child(card(vec![row_base()
                    .child(tile("icons/hard-drive.svg", hsl(0xff9500), 22.0))
                    .child(text_block(
                        volume.mount.name.clone().into(),
                        volume.usage_error.clone().map(Into::into),
                    ))
                    .child(
                        Button::new(("open-storage-volume", index), "Review in Files")
                            .busy(action_busy)
                            .disabled(action_disabled)
                            .on_click(move |_, _, cx| {
                                action_view.update(cx, |settings, cx| {
                                    settings.open_storage_volume(identity.clone(), cx);
                                });
                            }),
                    )
                    .into_any_element()]));
                continue;
            };
            let identity = volume.mount.identity.clone();
            let action_view = view.clone();
            let available_color = if usage.is_low_space() {
                hsl(0xff3b30)
            } else {
                hsl(0x34c759)
            };
            body = body
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .mb_3()
                        .p_4()
                        .rounded(px(10.0))
                        .bg(card_bg())
                        .border_1()
                        .border_color(if usage.is_low_space() {
                            rmac_ui::mac::warning_border()
                        } else {
                            sep()
                        })
                        .child(
                            div()
                                .h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(15.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(volume.mount.name.clone()),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .text_size(rmac_ui::text_px(13.0))
                                                .text_color(secondary())
                                                .child(format!(
                                                    "{} available of {}",
                                                    fmt_gb(usage.available),
                                                    fmt_gb(usage.total)
                                                )),
                                        )
                                        .child(
                                            Button::new(
                                                ("open-storage-volume", index),
                                                "Review in Files",
                                            )
                                            .busy(action_busy)
                                            .disabled(action_disabled)
                                            .on_click(
                                                move |_, _, cx| {
                                                    action_view.update(cx, |settings, cx| {
                                                        settings.open_storage_volume(
                                                            identity.clone(),
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        ),
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
                                        .w(gpui::relative(usage.used_fraction()))
                                        .rounded(px(5.0))
                                        .bg(if usage.is_low_space() {
                                            hsl(0xff3b30)
                                        } else {
                                            accent()
                                        }),
                                ),
                        ),
                )
                .child(card(vec![
                    value_row(
                        "icons/database.svg",
                        secondary(),
                        "Capacity".into(),
                        fmt_gb(usage.total).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        hsl(0xff9500),
                        "Used".into(),
                        fmt_gb(usage.used).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        available_color,
                        "Available".into(),
                        fmt_gb(usage.available).into(),
                    ),
                ]));
            if usage.is_low_space() {
                body = body.child(note_card(
                    "Space is low on this volume. Review large personal files and application caches before removing anything; rmac does not guess which files are safe to delete.",
                ));
            }
        }
        body.child(note_card(
            "Review in Files opens only a currently revalidated mounted volume. Storage categories and destructive cleanup actions stay hidden until they can be measured and reversed safely.",
        ))
    }

    // ---- subpages -----------------------------------------------------

    fn render_subpage(&self, sub: &SubPage, cx: &Context<Self>) -> Div {
        let (title, body): (SharedString, Div) = match sub {
            SubPage::About => ("About".into(), self.about_body(cx)),
            SubPage::SoftwareUpdate => ("Software Update".into(), self.software_update_body(cx)),
            SubPage::Storage => ("Storage".into(), self.storage_body(cx)),
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
            SubPage::FocusSchedule { schedule_id } => {
                ("Schedule".into(), self.focus_schedule_body(schedule_id, cx))
            }
        };

        let header = div()
            .v_flex()
            .items_center()
            .gap_1()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .text_size(rmac_ui::text_px(20.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(title),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child("‹ Back, or press ⌘["),
            );

        div().v_flex().child(header).child(body)
    }

    fn software_update_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let prepare_view = view.clone();
        let refresh = Button::new("refresh-update-status", "Check Again")
            .busy(self.updates_busy)
            .disabled(self.updates_loading || self.updates_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_update_status(cx));
            });
        let mut body = div().v_flex().child(card(vec![
            value_row(
                "icons/info.svg",
                secondary(),
                "Current version".into(),
                self.sysinfo.operating_system.clone().into(),
            ),
            row_base()
                .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                .child(text_block(
                    "Package updates".into(),
                    Some("PackageKit · configured repositories".into()),
                ))
                .child(refresh)
                .into_any_element(),
        ]));

        if self.updates_preparing {
            let cancel_view = view.clone();
            let cancel_requested = self
                .updates_cancellation
                .as_ref()
                .is_some_and(rmac_updates::Cancellation::is_cancelled);
            return body
                .child(
                    Progress::indeterminate()
                        .label("Refreshing and simulating the trusted package plan…")
                        .mb_3(),
                )
                .child(card(vec![row_base()
                    .child(tile("icons/shield.svg", accent(), 22.0))
                    .child(text_block(
                        "Verifying dependencies".into(),
                        Some("No package changes have started".into()),
                    ))
                    .child(
                        Button::new(
                            "cancel-update-preparation",
                            if cancel_requested {
                                "Cancelling…"
                            } else {
                                "Cancel"
                            },
                        )
                        .busy(cancel_requested)
                        .disabled(cancel_requested)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_update_operation(cx));
                        }),
                    )
                    .into_any_element()]));
        }

        if self.updates_installing {
            let progress = self.updates_progress.clone().unwrap_or_default();
            let mut progress_label = progress.phase.label().to_string();
            if let Some(package) = &progress.current_package {
                progress_label.push_str(&format!(" · {package}"));
            }
            if let Some(remaining) = progress.remaining_seconds {
                progress_label.push_str(&format!(
                    " · about {}",
                    format_power_duration(u64::from(remaining))
                ));
            }
            let progress_element = match progress.percentage {
                Some(percentage) => Progress::new(f32::from(percentage) / 100.0),
                None => Progress::indeterminate(),
            }
            .label(progress_label)
            .mb_3();
            let cancel_requested = self
                .updates_cancellation
                .as_ref()
                .is_some_and(rmac_updates::Cancellation::is_cancelled);
            let cancel_view = view.clone();
            return body.child(progress_element).child(card(vec![row_base()
                .child(tile("icons/refresh-cw.svg", accent(), 22.0))
                .child(text_block(
                    "Installing trusted updates".into(),
                    Some("PackageKit owns the transaction; do not turn off this computer".into()),
                ))
                .child(
                    Button::new(
                        "cancel-update-installation",
                        if cancel_requested {
                            "Cancelling…"
                        } else {
                            "Cancel"
                        },
                    )
                    .busy(cancel_requested)
                    .disabled(!progress.allow_cancel || cancel_requested)
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| settings.cancel_update_operation(cx));
                    }),
                )
                .into_any_element()]));
        }

        if let Some(result) = &self.updates_result {
            let subtitle = result
                .restart
                .label()
                .map(str::to_owned)
                .unwrap_or_else(|| "No restart was requested by PackageKit".into());
            body = body.child(card(vec![value_row(
                "icons/shield.svg",
                hsl(0x34c759),
                format!("{} packages updated", result.changed_packages).into(),
                subtitle.into(),
            )]));
        }

        if self.updates_loading && self.updates.is_none() {
            return body.child(
                Progress::indeterminate()
                    .label("Reading available updates…")
                    .mb_3(),
            );
        }

        let Some(snapshot) = &self.updates else {
            return body
                .child(
                    EmptyState::new("Update service unavailable")
                        .message("Install and enable PackageKit, then check again")
                        .error(true),
                )
                .child(note_card(
                    "No package state is guessed from local files or command output.",
                ));
        };

        let security = snapshot.security_count();
        let blocked = snapshot.blocked_count();
        let status = if snapshot.updates.is_empty() {
            "Your system is up to date".to_string()
        } else if security > 0 {
            format!(
                "{} updates available · {security} security",
                snapshot.updates.len()
            )
        } else {
            format!("{} updates available", snapshot.updates.len())
        };
        body = body.child(card(vec![value_row(
            "icons/shield.svg",
            if security > 0 {
                hsl(0xff3b30)
            } else {
                hsl(0x34c759)
            },
            "Status".into(),
            status.into(),
        )]));

        if snapshot.can_prepare_install() {
            body = body.child(card(vec![row_base()
                .child(tile("icons/shield.svg", accent(), 22.0))
                .child(text_block(
                    "Install all trusted updates".into(),
                    Some("Refresh, simulate dependencies, then confirm the exact plan".into()),
                ))
                .child(
                    Button::new("prepare-update-installation", "Install All…")
                        .primary()
                        .disabled(self.updates_busy)
                        .on_click(move |_, _, cx| {
                            prepare_view.update(cx, |settings, cx| settings.prepare_updates(cx));
                        }),
                )
                .into_any_element()]));
        } else if !snapshot.updates.is_empty() {
            body = body.child(note_card(
                snapshot
                    .install_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| {
                        if snapshot.truncated {
                            "The complete update set is too large to confirm safely.".into()
                        } else if snapshot.installable_count() == 0 {
                            "Every reported update is currently blocked by PackageKit.".into()
                        } else {
                            "The PackageKit backend cannot install updates on this system.".into()
                        }
                    }),
            ));
        }

        if !snapshot.updates.is_empty() {
            body = body.child(section_header("Available Updates"));
            let rows = snapshot
                .updates
                .iter()
                .map(|update| {
                    let detail = if update.summary.is_empty() {
                        update.kind.label().to_string()
                    } else {
                        format!("{} · {}", update.kind.label(), update.summary)
                    };
                    row_base()
                        .child(tile(
                            "icons/refresh-cw.svg",
                            match update.kind {
                                rmac_updates::UpdateKind::Security => hsl(0xff3b30),
                                rmac_updates::UpdateKind::Blocked => hsl(0xff9500),
                                _ => accent(),
                            },
                            22.0,
                        ))
                        .child(text_block(update.name.clone().into(), Some(detail.into())))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(secondary())
                                .child(update.version.clone()),
                        )
                        .into_any_element()
                })
                .collect();
            body = body.child(card(rows));
        }
        if snapshot.truncated {
            body = body.child(note_card(
                "More updates are available than this bounded view can display.",
            ));
        }
        if blocked > 0 {
            body = body.child(note_card(
                "Blocked updates are shown for awareness but are never included in rmac's installation plan.",
            ));
        }
        body.child(note_card(
            "PackageKit refreshes, simulates, downloads, and installs without shell commands. rmac never retries with untrusted packages and always rereads remaining updates afterward.",
        ))
    }

    fn about_body(&self, cx: &Context<Self>) -> Div {
        let si = &self.sysinfo;
        let view = cx.entity();
        let hostname_row = if let Some(editor) = &self.hostname_editor {
            let save_view = view.clone();
            let cancel_view = view.clone();
            row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    "Hostname".into(),
                    Some("Letters, numbers, and hyphens · 63 bytes maximum".into()),
                ))
                .child(div().w(px(190.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("hostname-cancel", "Cancel")
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_hostname_edit(cx));
                        }),
                )
                .child(
                    Button::new("hostname-save", "Save")
                        .primary()
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_hostname(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    "Hostname".into(),
                    si.hostname_unavailable_reason.clone().map(Into::into),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(si.display_hostname().to_owned()),
                )
                .when(si.hostname_mutable, |row| {
                    row.child(Button::new("hostname-edit", "Edit").on_click(
                        move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_hostname_edit(window, cx)
                            });
                        },
                    ))
                })
                .into_any_element()
        };

        let mut facts = Vec::new();
        if let Some(vendor) = &si.hardware_vendor {
            facts.push(value_row(
                "icons/monitor.svg",
                secondary(),
                "Manufacturer".into(),
                vendor.clone().into(),
            ));
        }
        facts.extend([
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Model".into(),
                si.hardware_model
                    .clone()
                    .unwrap_or_else(|| "—".into())
                    .into(),
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "Processor".into(),
                si.processor.clone().unwrap_or_else(|| "—".into()).into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Memory".into(),
                si.memory.clone().unwrap_or_else(|| "—".into()).into(),
            ),
            value_row(
                "icons/refresh-cw.svg",
                secondary(),
                "Operating System".into(),
                si.operating_system.clone().into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Kernel".into(),
                si.kernel.clone().into(),
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "Architecture".into(),
                si.architecture.clone().into(),
            ),
        ]);
        if let Some(graphics) = &si.graphics {
            facts.push(value_row(
                "icons/monitor.svg",
                secondary(),
                "Graphics".into(),
                graphics.clone().into(),
            ));
        }
        if let Some(session) = &si.session {
            facts.push(value_row(
                "icons/panel-top.svg",
                secondary(),
                "Session".into(),
                session.clone().into(),
            ));
        }
        if let Some(desktop) = &si.desktop {
            facts.push(value_row(
                "icons/panel-top.svg",
                secondary(),
                "Desktop".into(),
                desktop.clone().into(),
            ));
        }

        let refresh_view = view.clone();
        let diagnostics_view = view.clone();
        let diagnostics = card(vec![
            row_base()
                .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                .child(text_block(
                    "System information".into(),
                    Some("Refresh facts changed outside rmac".into()),
                ))
                .child(
                    Button::new("refresh-system-information", "Refresh")
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            refresh_view
                                .update(cx, |settings, cx| settings.refresh_system_info(cx));
                        }),
                )
                .into_any_element(),
            row_base()
                .child(tile("icons/info.svg", accent(), 22.0))
                .child(text_block(
                    "System report".into(),
                    Some(
                        "Excludes hostname, username, serial numbers, addresses, and paths".into(),
                    ),
                ))
                .child(
                    Button::new(
                        "copy-system-report",
                        if self.diagnostics_copied {
                            "Copied"
                        } else {
                            "Copy"
                        },
                    )
                    .on_click(move |_, _, cx| {
                        diagnostics_view.update(cx, |settings, cx| settings.copy_diagnostics(cx));
                    }),
                )
                .into_any_element(),
        ]);

        div()
            .v_flex()
            .child(card(vec![hostname_row]))
            .child(card(facts))
            .child(diagnostics)
    }
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
