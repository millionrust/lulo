//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · local account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive and backed by typed Linux/macOS services. Unsupported
//! mutations are explicitly unavailable rather than represented by local state.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Read-only panes use real platform state rather than fabricated values.

use std::borrow::Cow;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

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

#[derive(Clone)]
struct Category {
    name: SharedString,
    icon: &'static str,
    color: Hsla,
    desc: SharedString,
}

const GENERAL_DESTINATIONS: [&str; 3] = ["About", "Software Update", "Storage"];

// ---- interactive state enums --------------------------------------------

/// A navigation subpage pushed onto the back stack from a row chevron.
#[derive(Clone)]
enum SubPage {
    About,
    SoftwareUpdate,
    Storage,
    NotificationApp { app_id: String },
    FocusMode { mode_id: String },
    FocusSchedule { schedule_id: String },
}

#[derive(Clone, Debug, PartialEq)]
enum DockChange {
    Placement(rmac_shell_settings::DockPlacement),
    Outputs(rmac_shell_settings::OutputScope),
    Autohide(bool),
    Magnification(bool),
    MagnificationScale(f32),
    ReserveSpace(bool),
    RepeatedClick(rmac_shell_settings::RepeatedClickBehavior),
}

impl DockChange {
    fn apply(self, dock: &mut rmac_shell_settings::DockSettings) {
        match self {
            Self::Placement(value) => dock.placement = value,
            Self::Outputs(value) => dock.outputs = value,
            Self::Autohide(value) => dock.autohide = value,
            Self::Magnification(value) => dock.magnification = value,
            Self::MagnificationScale(value) => dock.magnification_scale = value,
            Self::ReserveSpace(value) => dock.reserve_space = value,
            Self::RepeatedClick(value) => dock.repeated_click = value,
        }
    }
}

enum ShellSettingsStreamUpdate {
    Snapshot(Box<rmac_shell_settings::Snapshot>),
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WallpaperTarget {
    Default,
    Output(String),
}

#[derive(Clone, Debug, PartialEq)]
enum WallpaperChange {
    Source(Option<String>),
    Fit(rmac_shell_settings::WallpaperFit),
    UseDefault,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SpotlightAuthority {
    providers: std::collections::BTreeMap<
        rmac_shell_settings::ProviderId,
        rmac_shell_settings::ProviderPolicy,
    >,
    spotlight: rmac_shell_settings::SpotlightSettings,
}

impl SpotlightAuthority {
    fn from_settings(settings: &rmac_shell_settings::ShellSettings) -> Self {
        Self {
            providers: settings.providers.clone(),
            spotlight: settings.spotlight.clone(),
        }
    }

    fn apply_to(self, settings: &mut rmac_shell_settings::ShellSettings) {
        settings.providers = self.providers;
        settings.spotlight = self.spotlight;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SpotlightChange {
    ProviderEnabled { id: String, enabled: bool },
    ProviderPrivateContent { id: String, allowed: bool },
    IncludeRemovableMounts(bool),
    AddExclusion(String),
    RemoveExclusion(String),
}

impl SpotlightChange {
    fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        match self {
            Self::ProviderEnabled { id, enabled } => {
                update_provider_policy(settings, id, |policy| policy.enabled = enabled);
            }
            Self::ProviderPrivateContent { id, allowed } => {
                update_provider_policy(settings, id, |policy| {
                    policy.allow_private_content = allowed;
                });
            }
            Self::IncludeRemovableMounts(enabled) => {
                settings.spotlight.include_removable_mounts = enabled;
            }
            Self::AddExclusion(path) => {
                if !settings.spotlight.excluded_paths.contains(&path) {
                    settings.spotlight.excluded_paths.push(path);
                }
            }
            Self::RemoveExclusion(path) => {
                settings
                    .spotlight
                    .excluded_paths
                    .retain(|excluded| excluded != &path);
            }
        }
    }
}

fn update_provider_policy(
    settings: &mut rmac_shell_settings::ShellSettings,
    id: String,
    update: impl FnOnce(&mut rmac_shell_settings::ProviderPolicy),
) {
    let id = rmac_shell_settings::ProviderId(id);
    let policy = settings.providers.entry(id.clone()).or_default();
    update(policy);
    if policy == &rmac_shell_settings::ProviderPolicy::default() {
        settings.providers.remove(&id);
    }
}

impl WallpaperChange {
    fn apply(
        self,
        target: &WallpaperTarget,
        wallpaper: &mut rmac_shell_settings::WallpaperSettings,
    ) {
        if matches!(self, Self::UseDefault) {
            if let WallpaperTarget::Output(output) = target {
                wallpaper.per_output.remove(output);
            }
            return;
        }

        let default = wallpaper.default.clone();
        let selection = match target {
            WallpaperTarget::Default => &mut wallpaper.default,
            WallpaperTarget::Output(output) => wallpaper
                .per_output
                .entry(output.clone())
                .or_insert(default),
        };
        match self {
            Self::Source(source) => selection.source = source,
            Self::Fit(fit) => selection.fit = fit,
            Self::UseDefault => unreachable!("handled before selecting a wallpaper target"),
        }
    }
}

enum ShellSettingsMutation {
    Change(DockChange),
    Restore(rmac_shell_settings::DockSettings),
    Wallpaper {
        target: WallpaperTarget,
        change: WallpaperChange,
    },
    RestoreWallpaper(rmac_shell_settings::WallpaperSettings),
    Spotlight(SpotlightChange),
    RestoreSpotlight(SpotlightAuthority),
}

impl ShellSettingsMutation {
    fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        match self {
            Self::Change(change) => change.apply(&mut settings.dock),
            Self::Restore(dock) => settings.dock = dock,
            Self::Wallpaper { target, change } => {
                change.apply(&target, &mut settings.wallpaper);
            }
            Self::RestoreWallpaper(wallpaper) => settings.wallpaper = wallpaper,
            Self::Spotlight(change) => change.apply(settings),
            Self::RestoreSpotlight(spotlight) => spotlight.apply_to(settings),
        }
    }
}

async fn watch_shell_settings(sender: async_channel::Sender<ShellSettingsStreamUpdate>) {
    loop {
        let setup = blocking::unblock(|| {
            let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
            let watcher = store.watch()?;
            let snapshot = store.load()?;
            Ok::<_, rmac_shell_settings::Error>((store, watcher, snapshot))
        })
        .await;
        let (mut store, watcher, snapshot) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if sender
                    .send(ShellSettingsStreamUpdate::Unavailable(error.to_string()))
                    .await
                    .is_err()
                {
                    return;
                }
                async_io::Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };
        if sender
            .send(ShellSettingsStreamUpdate::Snapshot(Box::new(snapshot)))
            .await
            .is_err()
        {
            return;
        }
        loop {
            match watcher.recv().await {
                Ok(rmac_shell_settings::StoreEvent::Changed) => {
                    let (returned_store, result) = blocking::unblock(move || {
                        let result = store.load();
                        (store, result)
                    })
                    .await;
                    store = returned_store;
                    let update = match result {
                        Ok(snapshot) => ShellSettingsStreamUpdate::Snapshot(Box::new(snapshot)),
                        Err(error) => ShellSettingsStreamUpdate::Unavailable(error.to_string()),
                    };
                    if sender.send(update).await.is_err() {
                        return;
                    }
                }
                Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                    if sender
                        .send(ShellSettingsStreamUpdate::Unavailable(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
                Err(_) => break,
            }
        }
        async_io::Timer::after(Duration::from_secs(1)).await;
    }
}

fn persist_shell_settings_mutation(
    mutation: ShellSettingsMutation,
) -> std::result::Result<rmac_shell_settings::Snapshot, rmac_shell_settings::Error> {
    let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
    let mut settings = store.load()?.settings;
    mutation.apply(&mut settings);
    store.save(&settings)?;
    // Read the complete document back so UI state is accepted only from the
    // same authority every shell process consumes.
    store.load()
}

const WALLPAPER_PREVIEW_WIDTH: u32 = 480;
const WALLPAPER_PREVIEW_HEIGHT: u32 = 270;

fn wallpaper_selection(
    wallpaper: &rmac_shell_settings::WallpaperSettings,
    target: &WallpaperTarget,
) -> (rmac_shell_settings::WallpaperSelection, bool) {
    match target {
        WallpaperTarget::Default => (wallpaper.default.clone(), true),
        WallpaperTarget::Output(output) => wallpaper.per_output.get(output).cloned().map_or_else(
            || (wallpaper.default.clone(), false),
            |selection| (selection, true),
        ),
    }
}

fn wallpaper_source_name(selection: &rmac_shell_settings::WallpaperSelection) -> SharedString {
    match rmac_wallpaper::parse_source(selection.source.as_deref()) {
        Ok(rmac_wallpaper::Source::BuiltIn(id)) => id.metadata().title.into(),
        Ok(rmac_wallpaper::Source::File(path)) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Local image".into())
            .into(),
        Err(_) => "Invalid saved source".into(),
    }
}

fn composite_wallpaper_pixel(source: [u8; 4], background: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(source[3]);
    let blend = |channel: usize| {
        ((u32::from(source[channel]) * alpha + u32::from(background[channel]) * (255 - alpha))
            / 255) as u8
    };
    [blend(0), blend(1), blend(2), 255]
}

fn render_wallpaper_preview(
    selection: &rmac_shell_settings::WallpaperSelection,
) -> std::result::Result<std::sync::Arc<gpui::RenderImage>, String> {
    let source = rmac_wallpaper::parse_source(selection.source.as_deref())
        .map_err(|_| "the saved wallpaper source is invalid".to_owned())?;
    let resolved = rmac_wallpaper_system::resolve(&source).map_err(|error| error.to_string())?;
    let decoded = rmac_wallpaper_image::Cache::new(0)
        .get_or_decode(
            resolved,
            rmac_compositor::PhysicalSize {
                width: WALLPAPER_PREVIEW_WIDTH,
                height: WALLPAPER_PREVIEW_HEIGHT,
            },
        )
        .map_err(|error| error.to_string())?;
    let layout = rmac_wallpaper::layout(
        selection.fit,
        decoded.physical_size(),
        rmac_compositor::LogicalSize {
            width: f64::from(WALLPAPER_PREVIEW_WIDTH),
            height: f64::from(WALLPAPER_PREVIEW_HEIGHT),
        },
        1.0,
    )
    .map_err(|_| "the wallpaper fit could not be previewed".to_owned())?;

    let mut bgra = Vec::with_capacity(
        usize::try_from(WALLPAPER_PREVIEW_WIDTH * WALLPAPER_PREVIEW_HEIGHT * 4)
            .expect("fixed wallpaper preview size fits usize"),
    );
    let destination = layout.destination;
    for y in 0..WALLPAPER_PREVIEW_HEIGHT {
        for x in 0..WALLPAPER_PREVIEW_WIDTH {
            let sample = if layout.tiled {
                Some((x % decoded.width, y % decoded.height))
            } else {
                let px = f64::from(x) + 0.5;
                let py = f64::from(y) + 0.5;
                let inside = px >= destination.x
                    && py >= destination.y
                    && px < destination.x + destination.width
                    && py < destination.y + destination.height;
                inside.then(|| {
                    let source_x = ((px - destination.x) / destination.width
                        * f64::from(decoded.width))
                    .floor()
                    .clamp(0.0, f64::from(decoded.width - 1))
                        as u32;
                    let source_y = ((py - destination.y) / destination.height
                        * f64::from(decoded.height))
                    .floor()
                    .clamp(0.0, f64::from(decoded.height - 1))
                        as u32;
                    (source_x, source_y)
                })
            };
            let background = [30_u8, 30, 32, 255];
            let rgba = sample.map_or(background, |(source_x, source_y)| {
                let index = usize::try_from(
                    (u64::from(source_y) * u64::from(decoded.width) + u64::from(source_x)) * 4,
                )
                .expect("bounded decoded image index fits usize");
                composite_wallpaper_pixel(
                    decoded.rgba[index..index + 4]
                        .try_into()
                        .expect("decoded wallpaper pixel has four channels"),
                    background,
                )
            });
            // GPUI's RenderImage upload path consumes BGRA pixels.
            bgra.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
    }
    let buffer =
        image::RgbaImage::from_raw(WALLPAPER_PREVIEW_WIDTH, WALLPAPER_PREVIEW_HEIGHT, bgra)
            .ok_or_else(|| "the wallpaper preview buffer was invalid".to_owned())?;
    Ok(std::sync::Arc::new(gpui::RenderImage::new(vec![
        image::Frame::new(buffer),
    ])))
}

fn validate_wallpaper_choice(
    path: PathBuf,
    fit: rmac_shell_settings::WallpaperFit,
) -> std::result::Result<String, String> {
    let source = path
        .to_str()
        .ok_or_else(|| "The selected wallpaper path cannot be represented as text".to_owned())?
        .to_owned();
    let selection = rmac_shell_settings::WallpaperSelection {
        source: Some(source.clone()),
        fit,
    };
    render_wallpaper_preview(&selection).map_err(|error| {
        format!("The selected file is not a usable PNG, JPEG, or WebP image: {error}")
    })?;
    Ok(source)
}

fn spotlight_provider_policy(
    settings: &rmac_shell_settings::ShellSettings,
    id: &str,
) -> rmac_shell_settings::ProviderPolicy {
    settings
        .providers
        .get(&rmac_shell_settings::ProviderId(id.to_owned()))
        .cloned()
        .unwrap_or_default()
}

fn validate_search_exclusion(path: PathBuf) -> std::result::Result<String, String> {
    let canonical = path
        .canonicalize()
        .map_err(|_| "The selected search exclusion is no longer available".to_owned())?;
    if !canonical.is_dir() {
        return Err("Search exclusions must be folders".into());
    }
    canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "The selected folder path cannot be represented as text".to_owned())
}

fn wifi_stream_snapshot_is_current(
    captured_generation: u64,
    current_generation: u64,
    busy: bool,
    loading: bool,
) -> bool {
    !busy && !loading && captured_generation == current_generation
}

fn bluetooth_stream_snapshot_is_current(
    captured_generation: u64,
    current_generation: u64,
    busy: bool,
    loading: bool,
) -> bool {
    !busy && !loading && captured_generation == current_generation
}

struct WifiPasswordPrompt {
    network: rmac_network::WifiNetworkId,
    ssid: SharedString,
    editor: Entity<InputState>,
    validation_error: Option<SharedString>,
}

struct WifiForgetPrompt {
    network: rmac_network::WifiNetworkId,
    ssid: SharedString,
}

enum BluetoothPairingDisplay {
    PinCode(String),
    Passkey { passkey: u32, entered: u16 },
}

struct BluetoothPairingState {
    device_id: String,
    name: SharedString,
    session: rmac_bluetooth::PairingSession,
    prompt: Option<rmac_bluetooth::PairingPrompt>,
    display: Option<BluetoothPairingDisplay>,
    editor: Entity<InputState>,
    validation_error: Option<SharedString>,
    stopping: bool,
}

struct BluetoothForgetPrompt {
    device_id: String,
    name: SharedString,
}

struct Settings {
    system_data_loading: bool,
    system_data_busy: bool,
    system_data_error: Option<SharedString>,
    hostname_editor: Option<Entity<InputState>>,
    diagnostics_copied: bool,
    account: SharedString,
    sysinfo: rmac_system_info::Snapshot,
    screen_reader: ScreenReaderCapability,
    updates_loading: bool,
    updates_busy: bool,
    updates_error: Option<SharedString>,
    updates: Option<rmac_updates::Snapshot>,
    time_loading: bool,
    time_busy: bool,
    time_error: Option<SharedString>,
    time_stream_error: Option<SharedString>,
    time: Option<rmac_time::Snapshot>,
    timezone_editor: Option<Entity<InputState>>,
    locale_loading: bool,
    locale_busy: bool,
    locale_error: Option<SharedString>,
    locale_stream_error: Option<SharedString>,
    locale: Option<rmac_locale::Snapshot>,
    locale_editor: Option<Entity<InputState>>,
    locale_revert: Option<Vec<String>>,
    x11_layout_editor: Option<Entity<InputState>>,
    x11_variant_editor: Option<Entity<InputState>>,
    x11_options_editor: Option<Entity<InputState>>,
    x11_keyboard_revert: Option<rmac_locale::X11Keyboard>,
    login_items_loading: bool,
    login_item_busy: Option<String>,
    login_items_error: Option<SharedString>,
    login_items_stream_error: Option<SharedString>,
    login_items: Option<rmac_login_items::Snapshot>,
    login_item_add: Option<rmac_login_items::AddPreview>,
    login_item_remove: Option<(String, String)>,
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
    vpn_error: Option<SharedString>,
    audio_error: Option<SharedString>,
    power_error: Option<SharedString>,
    display_error: Option<SharedString>,
    input_error: Option<SharedString>,
    theme_error: Option<SharedString>,
    shell_settings_error: Option<SharedString>,
    shell_settings_stream_error: Option<SharedString>,
    gtk_text_error: Option<SharedString>,
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
    shortcut_status_loading: bool,
    shortcut_status: Option<rmac_shortcuts::BackendStatus>,
    shortcut_status_error: Option<SharedString>,

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
    wifi_generation: u64,
    wifi_connecting: Option<rmac_network::WifiNetworkId>,
    wifi_forgetting: Option<rmac_network::WifiNetworkId>,
    wifi_forget_confirmation: Option<WifiForgetPrompt>,
    wifi_password_prompt: Option<WifiPasswordPrompt>,
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

    // External GTK application text
    gtk_text_loading: bool,
    gtk_text_busy: bool,

    // Privacy & Security
    privacy_loading: bool,
    privacy_busy: Option<(rmac_privacy::PortalResource, String)>,
    privacy_reset_confirmation: Option<rmac_privacy::PortalDecision>,
    security_coverage_loading: bool,

    // Sound
    audio_loading: bool,
    audio_busy: bool,
    output_volume_generation: u64,
    input_volume_generation: u64,
    output_volume: Entity<SliderState>,
    input_volume: Entity<SliderState>,

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
    KeyboardRepeatPreset {
        delay_ms: u32,
        rate: u32,
    },
    KeyboardNumlock(bool),
    MouseNaturalScroll(bool),
    MouseLeftHanded(bool),
    MouseMiddleEmulation(bool),
    MouseAccelSpeed(f64),
    MouseAccelProfile(rmac_input::AccelProfile),
    MousePrecisionPreset {
        speed: f64,
        profile: rmac_input::AccelProfile,
    },
    TouchpadNaturalScroll(bool),
    TouchpadLeftHanded(bool),
    TouchpadMiddleEmulation(bool),
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
    TextScale(rmac_theme::TextScalePreference),
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
    sysinfo: std::result::Result<rmac_system_info::Snapshot, rmac_system_info::Error>,
    storage: std::result::Result<Vec<rmac_mounts::Volume>, rmac_mounts::Error>,
    screen_reader: ScreenReaderCapability,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ScreenReaderCapability {
    niri_session: bool,
    xwayland: bool,
    orca_path: Option<PathBuf>,
}

impl ScreenReaderCapability {
    fn ready(&self) -> bool {
        self.niri_session && self.xwayland && self.orca_path.is_some()
    }

    fn limitation(&self) -> Option<&'static str> {
        if !self.niri_session {
            Some("Start the desktop through a full niri-session")
        } else if !self.xwayland {
            Some("Xwayland is required by Orca in the current niri integration")
        } else if self.orca_path.is_none() {
            Some("Install Orca to enable screen-reader support")
        } else {
            None
        }
    }
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

const FOCUS_DAYS: [(rmac_focus::Weekday, &str); 7] = [
    (rmac_focus::Weekday::Monday, "M"),
    (rmac_focus::Weekday::Tuesday, "T"),
    (rmac_focus::Weekday::Wednesday, "W"),
    (rmac_focus::Weekday::Thursday, "T"),
    (rmac_focus::Weekday::Friday, "F"),
    (rmac_focus::Weekday::Saturday, "S"),
    (rmac_focus::Weekday::Sunday, "S"),
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
                match event {
                    rmac_privacy::WatchEvent::Changed => {
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_privacy_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if this.privacy_busy.is_none() && !this.privacy_loading {
                                    this.finish_privacy_update(result);
                                    this.privacy_stream_error = None;
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_privacy::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.privacy_stream_error = Some(
                                    "Live portal permission updates are temporarily unavailable"
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
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_login_items_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if this.login_item_busy.is_none() {
                                    this.finish_login_items_update(result);
                                    this.login_items_stream_error = None;
                                    cx.notify();
                                }
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
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_time_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if !this.time_busy {
                                    this.finish_time_update(result);
                                    this.time_stream_error = None;
                                    cx.notify();
                                }
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

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
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
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_locale_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if !this.locale_busy {
                                    this.finish_locale_update(result);
                                    this.locale_stream_error = None;
                                    cx.notify();
                                }
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
                        let generation = match this.update(cx, |this: &mut Settings, cx| {
                            this.wifi_stream_error = None;
                            cx.notify();
                            (!this.wifi_busy && !this.wifi_loading)
                                .then_some(this.wifi_generation)
                        }) {
                            Ok(generation) => generation,
                            Err(_) => break,
                        };
                        let Some(generation) = generation else {
                            continue;
                        };
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_network::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if wifi_stream_snapshot_is_current(
                                    generation,
                                    this.wifi_generation,
                                    this.wifi_busy,
                                    this.wifi_loading,
                                ) {
                                    this.finish_wifi_stream_update(result);
                                    cx.notify();
                                }
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
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
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
                        this.dock_compositor.apply(event);
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
            hostname_editor: None,
            diagnostics_copied: false,
            account: std::env::var("USER")
                .unwrap_or_else(|_| "User".into())
                .into(),
            sysinfo: rmac_system_info::Snapshot::default(),
            screen_reader: ScreenReaderCapability::default(),
            updates_loading: true,
            updates_busy: false,
            updates_error: None,
            updates: None,
            time_loading: true,
            time_busy: false,
            time_error: None,
            time_stream_error: None,
            time: None,
            timezone_editor: None,
            locale_loading: true,
            locale_busy: false,
            locale_error: None,
            locale_stream_error: None,
            locale: None,
            locale_editor: None,
            locale_revert: None,
            x11_layout_editor: None,
            x11_variant_editor: None,
            x11_options_editor: None,
            x11_keyboard_revert: None,
            login_items_loading: true,
            login_item_busy: None,
            login_items_error: None,
            login_items_stream_error: None,
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
            vpn_error: None,
            audio_error: None,
            power_error: None,
            display_error: None,
            input_error: None,
            theme_error: None,
            shell_settings_error: None,
            shell_settings_stream_error: None,
            gtk_text_error: None,
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
            shortcut_status_loading: true,
            shortcut_status: None,
            shortcut_status_error: None,

            network_loading: true,
            network_busy: false,

            vpn: rmac_network::VpnSnapshot::default(),
            vpn_loading: true,
            vpn_busy: None,

            wifi_available: false,
            wifi_loading: true,
            wifi_busy: false,
            wifi_generation: 0,
            wifi_connecting: None,
            wifi_forgetting: None,
            wifi_forget_confirmation: None,
            wifi_password_prompt: None,
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
            gtk_text_loading: true,
            gtk_text_busy: false,

            privacy_loading: true,
            privacy_busy: None,
            privacy_reset_confirmation: None,
            security_coverage_loading: true,

            audio_loading: true,
            audio_busy: false,
            output_volume_generation: 0,
            input_volume_generation: 0,
            output_volume,
            input_volume,

            power_loading: true,
            power_busy: false,

            display_loading: true,
            display_busy: false,
            display_revert: None,

            input_loading: true,
            input_busy: false,
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

    fn apply_system_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.account = snapshot.account.into();
        self.screen_reader = snapshot.screen_reader;
        match snapshot.sysinfo {
            Ok(sysinfo) => {
                self.sysinfo = sysinfo;
                self.system_data_error = None;
            }
            Err(error) => {
                self.system_data_error =
                    Some(format!("Could not read system information: {error}").into());
            }
        }
        match snapshot.storage {
            Ok(storage) => {
                self.storage = storage;
                self.storage_error = None;
            }
            Err(error) => {
                self.storage_error =
                    Some(format!("Could not read storage volumes: {error}").into());
            }
        }
        self.system_data_loading = false;
    }

    fn start_hostname_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system_data_busy || !self.sysinfo.hostname_mutable {
            return;
        }
        let hostname = self
            .sysinfo
            .static_hostname
            .clone()
            .unwrap_or_else(|| self.sysinfo.hostname.clone());
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(hostname)
                .placeholder("studio-pc")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.hostname_editor = Some(editor);
        self.system_data_error = None;
        cx.notify();
    }

    fn cancel_hostname_edit(&mut self, cx: &mut Context<Self>) {
        if !self.system_data_busy {
            self.hostname_editor = None;
            self.system_data_error = None;
            cx.notify();
        }
    }

    fn submit_hostname(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        let Some(editor) = self.hostname_editor.as_ref() else {
            return;
        };
        let hostname = editor.read(cx).value().trim().to_string();
        if let Err(error) = rmac_system_info::validate_static_hostname(&hostname) {
            self.system_data_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.system_data_busy = true;
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_system_info::set_static_hostname(&hostname) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.hostname_editor = None;
                        this.system_data_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error = Some(error.to_string().into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_system_info(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        self.system_data_busy = true;
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, screen_reader) = cx
                .background_executor()
                .spawn(async {
                    (
                        rmac_system_info::snapshot(),
                        gather_screen_reader_capability(),
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                this.screen_reader = screen_reader;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.system_data_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error =
                            Some(format!("Could not refresh system information: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        let report = self
            .sysinfo
            .diagnostic_report(self.display.graphics.as_deref());
        cx.write_to_clipboard(ClipboardItem::new_string(report));
        self.diagnostics_copied = true;
        cx.notify();
    }

    fn finish_update_status(
        &mut self,
        result: std::result::Result<rmac_updates::Snapshot, rmac_updates::Error>,
    ) {
        self.updates_loading = false;
        self.updates_busy = false;
        match result {
            Ok(snapshot) => {
                self.updates = Some(snapshot);
                self.updates_error = None;
            }
            Err(error) => {
                self.updates_error = Some(format!("Could not check for updates: {error}").into());
            }
        }
    }

    fn refresh_update_status(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy {
            return;
        }
        self.updates_busy = true;
        self.updates_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::refresh()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        if self.storage_busy {
            return;
        }
        self.storage_busy = true;
        self.storage_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_mounts::volumes() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.storage_busy = false;
                match result {
                    Ok(volumes) => {
                        this.storage = volumes;
                        this.storage_error = None;
                    }
                    Err(error) => {
                        this.storage_error =
                            Some(format!("Could not refresh storage volumes: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_time_update(
        &mut self,
        result: std::result::Result<rmac_time::Snapshot, rmac_time::Error>,
    ) {
        self.time_loading = false;
        self.time_busy = false;
        match result {
            Ok(snapshot) => {
                self.time = Some(snapshot);
                self.time_error = None;
            }
            Err(error) => {
                self.time_error = Some(format!("Could not update date and time: {error}").into());
            }
        }
    }

    fn refresh_time(&mut self, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_automatic_time(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_ntp(enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn start_timezone_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_busy || self.timezone_editor.is_some() {
            return;
        }
        let Some(snapshot) = &self.time else {
            return;
        };
        let timezone = snapshot.timezone.clone();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(timezone)
                .placeholder("Asia/Kolkata")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.timezone_editor = Some(editor);
        self.time_error = None;
        cx.notify();
    }

    fn cancel_timezone_edit(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.timezone_editor = None;
            self.time_error = None;
            cx.notify();
        }
    }

    fn submit_timezone(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.timezone_editor, &self.time) else {
            return;
        };
        let timezone = editor.read(cx).value().trim().to_string();
        if let Err(error) = snapshot.validate_timezone(&timezone) {
            self.time_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.time_busy = true;
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_timezone(&timezone) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if result.is_ok() {
                    this.timezone_editor = None;
                }
                this.finish_time_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_locale_update(
        &mut self,
        result: std::result::Result<rmac_locale::Snapshot, rmac_locale::Error>,
    ) {
        self.locale_loading = false;
        self.locale_busy = false;
        match result {
            Ok(snapshot) => {
                self.locale = Some(snapshot);
                self.locale_error = None;
            }
            Err(error) => {
                self.locale_error =
                    Some(format!("Could not update language and region: {error}").into());
            }
        }
    }

    fn refresh_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_loading || self.locale_busy {
            return;
        }
        self.locale_busy = true;
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_locale_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn start_locale_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy || self.locale_editor.is_some() {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let language = snapshot.language().to_owned();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(language)
                .placeholder("en_US.UTF-8")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.locale_editor = Some(editor);
        self.locale_error = None;
        cx.notify();
    }

    fn cancel_locale_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.locale_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    fn submit_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.locale_editor, &self.locale) else {
            return;
        };
        let language = editor.read(cx).value().trim().to_owned();
        let next = match snapshot.preview_language(&language) {
            Ok(assignments) => assignments
                .iter()
                .map(rmac_locale::Assignment::encoded)
                .collect::<Vec<_>>(),
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let previous = snapshot.encoded_locale();
        self.locale_busy = true;
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_locale(&next) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.locale_editor = None;
                    this.locale_revert = Some(previous);
                }
                this.finish_locale_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(previous) = self.locale_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_locale(&previous) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.locale_revert = None;
                    this.locale_editor = None;
                }
                this.finish_locale_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn start_x11_keyboard_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.x11_layout_editor.is_some()
            || self.input.keyboard_layout_authority
                != rmac_input::KeyboardLayoutAuthority::SystemLocaled
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let layout = snapshot.x11_layout.clone();
        let variant = snapshot.x11_variant.clone();
        let options = snapshot.x11_options.clone();
        let layout_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(layout)
                .placeholder("us,de")
        });
        let variant_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(variant)
                .placeholder(",nodeadkeys")
        });
        let options_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(options)
                .placeholder("grp:ctrl_space_toggle")
        });
        let focus = layout_editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.x11_layout_editor = Some(layout_editor);
        self.x11_variant_editor = Some(variant_editor);
        self.x11_options_editor = Some(options_editor);
        self.locale_error = None;
        cx.notify();
    }

    fn cancel_x11_keyboard_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.x11_layout_editor = None;
            self.x11_variant_editor = None;
            self.x11_options_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    fn submit_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(layout_editor), Some(variant_editor), Some(options_editor), Some(snapshot)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
            &self.locale,
        ) else {
            return;
        };
        let layout = layout_editor.read(cx).value().trim().to_owned();
        let variant = variant_editor.read(cx).value().trim().to_owned();
        let options = options_editor.read(cx).value().trim().to_owned();
        let keyboard = match snapshot.preview_x11_keyboard(&layout, &variant, &options) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let previous = snapshot.x11_keyboard();
        self.locale_busy = true;
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_x11_keyboard(&keyboard) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.x11_layout_editor = None;
                    this.x11_variant_editor = None;
                    this.x11_options_editor = None;
                    this.x11_keyboard_revert = Some(previous);
                }
                this.finish_locale_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn revert_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(keyboard) = self.x11_keyboard_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_x11_keyboard(&keyboard) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.x11_keyboard_revert = None;
                }
                this.finish_locale_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_login_items_update(
        &mut self,
        result: std::result::Result<rmac_login_items::Snapshot, rmac_login_items::Error>,
    ) {
        self.login_items_loading = false;
        self.login_item_busy = None;
        match result {
            Ok(snapshot) => {
                self.login_items = Some(snapshot);
                self.login_items_error = None;
            }
            Err(error) => {
                self.login_items_error =
                    Some(format!("Could not update login items: {error}").into());
            }
        }
    }

    fn refresh_login_items(&mut self, cx: &mut Context<Self>) {
        if self.login_items_loading || self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some("refresh".into());
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_login_item_enabled(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(id.clone());
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_background_service_enabled(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("systemd:{id}"));
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_background_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn reveal_login_item(&mut self, id: String, background: bool, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("reveal:{id}"));
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    if background {
                        rmac_login_items_linux::background_service_source(&id)
                    } else {
                        rmac_login_items_linux::autostart_source(&id)
                    }
                })
                .await;
            let result = match path {
                Ok(path) => rmac_portal::show_item(&path)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                this.login_items_error = result
                    .err()
                    .map(|error| format!("Could not reveal login item: {error}").into());
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some("choose".into());
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_desktop_entry().await;
            let preview = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move { rmac_login_items_linux::prepare_add_source(&path) })
                        .await,
                ),
                Ok(None) => None,
                Err(error) => Some(Err(rmac_login_items::Error::new(
                    rmac_login_items::ErrorKind::Unavailable,
                    error.to_string(),
                ))),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                match preview {
                    Some(Ok(preview)) => {
                        this.login_item_add = Some(preview);
                        this.login_items_error = None;
                    }
                    Some(Err(error)) => {
                        this.login_items_error =
                            Some(format!("Could not add login item: {error}").into());
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_add_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some(preview) = self.login_item_add.clone() else {
            return;
        };
        self.login_item_busy = Some("add".into());
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_login_items_linux::add_source(&preview.source, preview.replacing)
                })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_add = None;
                }
                this.finish_login_items_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn request_remove_login_item(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        if self.login_item_busy.is_none() {
            self.login_item_remove = Some((id, name));
            cx.notify();
        }
    }

    fn confirm_remove_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some((id, _)) = self.login_item_remove.clone() else {
            return;
        };
        self.login_item_busy = Some("remove".into());
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::remove_autostart(&id) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_remove = None;
                }
                this.finish_login_items_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_sharing_update(
        &mut self,
        result: std::result::Result<rmac_sharing::Snapshot, rmac_sharing::Error>,
    ) {
        self.sharing_loading = false;
        self.sharing_busy = false;
        match result {
            Ok(snapshot) => {
                self.sharing = Some(snapshot);
                self.sharing_error = None;
            }
            Err(error) => {
                self.sharing_error = Some(format!("Could not update Sharing: {error}").into());
            }
        }
    }

    fn refresh_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_loading || self.sharing_busy {
            return;
        }
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
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
    }

    fn confirm_remote_login(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_remote_login(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_file_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.file_sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_file_sharing(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.file_sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_wifi_snapshot(&mut self, snapshot: rmac_network::WifiSnapshot) {
        self.wifi_available = snapshot.available;
        self.wifi_on = snapshot.enabled;
        self.wifi_interface = snapshot.interface;
        self.wifi_networks = snapshot.networks;
        self.wifi_saved_networks = snapshot.saved_networks;
    }

    fn begin_wifi_mutation(&mut self) {
        self.wifi_generation = self.wifi_generation.wrapping_add(1);
        self.wifi_busy = true;
    }

    fn finish_wifi_stream_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_stream_error = None;
            }
            Err(_) => {
                self.wifi_stream_error =
                    Some("Live Wi-Fi state could not be refreshed from NetworkManager".into());
            }
        }
    }

    fn finish_wifi_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_connecting = None;
        self.wifi_cancellation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_error = None;
            }
            Err(error) => {
                self.wifi_error = Some(format!("Could not update Wi-Fi: {error}").into());
            }
        }
    }

    fn finish_wifi_password_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_connecting = None;
        self.wifi_cancellation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_password_prompt = None;
                self.wifi_error = None;
            }
            Err(error) if error.is_cancelled() => {
                self.wifi_password_prompt = None;
                self.wifi_error = None;
            }
            Err(error) => {
                if let Some(prompt) = &mut self.wifi_password_prompt {
                    prompt.validation_error =
                        Some(format!("Could not join this network: {error}").into());
                } else {
                    self.wifi_error = Some(format!("Could not join Wi-Fi: {error}").into());
                }
            }
        }
    }

    fn finish_wifi_forget_update(
        &mut self,
        result: std::result::Result<rmac_network::WifiSnapshot, rmac_network::Error>,
        recovery_snapshot: Option<rmac_network::WifiSnapshot>,
    ) {
        self.wifi_loading = false;
        self.wifi_busy = false;
        self.wifi_forgetting = None;
        self.wifi_forget_confirmation = None;
        match result {
            Ok(snapshot) => {
                self.apply_wifi_snapshot(snapshot);
                self.wifi_error = None;
            }
            Err(error) => {
                if let Some(snapshot) = recovery_snapshot {
                    self.apply_wifi_snapshot(snapshot);
                }
                self.wifi_error = Some(format!("Could not forget Wi-Fi network: {error}").into());
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
            ThemeChange::TextScale(value) => preferences.text_scale = value,
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

    fn refresh_gtk_text(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy {
            return;
        }
        self.gtk_text_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_gtk_text_scale(&mut self, factor: f64, cx: &mut Context<Self>) {
        if self.gtk_text_loading
            || self.gtk_text_busy
            || !self
                .gtk_text
                .as_ref()
                .is_some_and(|snapshot| snapshot.available && snapshot.writable)
        {
            return;
        }
        self.gtk_text_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_gtk_settings::set_text_scale(factor) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_privacy(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() {
            return;
        }
        self.privacy_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
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
            && self
                .privacy
                .as_ref()
                .is_some_and(|snapshot| snapshot.can_reset)
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
        let Some(decision) = self.privacy_reset_confirmation.take() else {
            return;
        };
        if self.privacy_busy.is_some() {
            return;
        }
        let resource = decision.resource;
        let app_id = decision.app_id;
        self.privacy_busy = Some((resource, app_id.clone()));
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_privacy_linux::reset_decision(resource, &app_id) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
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
            InputChange::KeyboardRepeatPreset { delay_ms, rate } => {
                settings.keyboard.repeat_delay_ms = delay_ms;
                settings.keyboard.repeat_rate = rate;
            }
            InputChange::KeyboardNumlock(value) => settings.keyboard.numlock = value,
            InputChange::MouseNaturalScroll(value) => settings.mouse.natural_scroll = value,
            InputChange::MouseLeftHanded(value) => settings.mouse.left_handed = value,
            InputChange::MouseMiddleEmulation(value) => settings.mouse.middle_emulation = value,
            InputChange::MouseAccelSpeed(value) => settings.mouse.accel_speed = value,
            InputChange::MouseAccelProfile(value) => settings.mouse.accel_profile = value,
            InputChange::MousePrecisionPreset { speed, profile } => {
                settings.mouse.accel_speed = speed;
                settings.mouse.accel_profile = profile;
            }
            InputChange::TouchpadNaturalScroll(value) => {
                settings.touchpad.pointer.natural_scroll = value
            }
            InputChange::TouchpadLeftHanded(value) => settings.touchpad.pointer.left_handed = value,
            InputChange::TouchpadMiddleEmulation(value) => {
                settings.touchpad.pointer.middle_emulation = value
            }
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
        self.begin_wifi_mutation();
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
        self.begin_wifi_mutation();
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

    fn connect_wifi(&mut self, network: rmac_network::WifiNetworkId, cx: &mut Context<Self>) {
        if self.wifi_busy || self.wifi_loading || !self.wifi_available || !self.wifi_on {
            return;
        }
        let Some(candidate) = self
            .wifi_networks
            .iter()
            .find(|candidate| candidate.id == network)
        else {
            return;
        };
        if !candidate.can_connect() {
            return;
        }
        self.begin_wifi_mutation();
        self.wifi_connecting = Some(network.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::connect(&network) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn request_wifi_forget(
        &mut self,
        network: rmac_network::WifiNetworkId,
        ssid: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy
            || self.wifi_loading
            || !self
                .wifi_saved_networks
                .iter()
                .any(|candidate| candidate.id == network)
        {
            return;
        }
        self.wifi_forget_confirmation = Some(WifiForgetPrompt { network, ssid });
        self.wifi_error = None;
        cx.notify();
    }

    fn cancel_wifi_forget(&mut self, cx: &mut Context<Self>) {
        if !self.wifi_busy {
            self.wifi_forget_confirmation = None;
            cx.notify();
        }
    }

    fn confirm_wifi_forget(&mut self, cx: &mut Context<Self>) {
        if self.wifi_busy || self.wifi_loading {
            return;
        }
        let Some(network) = self
            .wifi_forget_confirmation
            .as_ref()
            .map(|prompt| prompt.network.clone())
        else {
            return;
        };
        if !self
            .wifi_saved_networks
            .iter()
            .any(|candidate| candidate.id == network)
        {
            self.wifi_forget_confirmation = None;
            self.wifi_error = Some("The saved Wi-Fi network is no longer available.".into());
            cx.notify();
            return;
        }

        self.begin_wifi_mutation();
        self.wifi_forgetting = Some(network.clone());
        self.wifi_forget_confirmation = None;
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::forget(&network);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_forget_update(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    fn select_wifi_network(
        &mut self,
        network: rmac_network::WifiNetwork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.wifi_busy
            || self.wifi_loading
            || !self.wifi_available
            || !self.wifi_on
            || !network.can_connect()
            || !self
                .wifi_networks
                .iter()
                .any(|candidate| candidate.id == network.id)
        {
            return;
        }
        if !network.needs_password() {
            self.connect_wifi(network.id, cx);
            return;
        }

        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = editor.read(cx).focus_handle(cx);
        self.wifi_password_prompt = Some(WifiPasswordPrompt {
            network: network.id,
            ssid: network.ssid.into(),
            editor,
            validation_error: None,
        });
        self.wifi_error = None;
        window.focus(&focus);
        cx.notify();
    }

    fn cancel_wifi_password(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.wifi_cancellation {
            cancellation.cancel();
        } else {
            self.wifi_password_prompt = None;
            self.wifi_connecting = None;
        }
        cx.notify();
    }

    fn submit_wifi_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wifi_busy || !self.wifi_available || !self.wifi_on {
            return;
        }
        let Some(prompt) = &self.wifi_password_prompt else {
            return;
        };
        let network = prompt.network.clone();
        let value = prompt.editor.read(cx).value().to_string();
        let password = match rmac_network::WifiPassword::new(value, &network) {
            Ok(password) => password,
            Err(error) => {
                if let Some(prompt) = &mut self.wifi_password_prompt {
                    prompt.validation_error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };

        // Drop the editor entity that contained the secret, including its undo
        // history, as soon as ownership moves into the zeroizing password type.
        let empty_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .clean_on_escape()
                .placeholder("Password")
        });
        let focus = empty_editor.read(cx).focus_handle(cx);
        if let Some(prompt) = &mut self.wifi_password_prompt {
            prompt.editor = empty_editor;
            prompt.validation_error = None;
        }
        window.focus(&focus);

        let cancellation = rmac_network::WifiCancellation::new();
        self.begin_wifi_mutation();
        self.wifi_connecting = Some(network.clone());
        self.wifi_cancellation = Some(cancellation.clone());
        self.wifi_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_network::connect_with_password(&network, password, &cancellation)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_password_update(result);
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
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_error = None;
            }
            Err(error) => {
                self.bluetooth_error = Some(format!("Could not update Bluetooth: {error}").into());
            }
        }
    }

    fn finish_bluetooth_stream_update(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_stream_error = None;
            }
            Err(error) => {
                self.bluetooth_stream_error =
                    Some(format!("Could not refresh live Bluetooth state: {error}").into());
            }
        }
    }

    fn apply_bluetooth_snapshot(&mut self, snapshot: rmac_bluetooth::Snapshot) {
        self.bluetooth_available = snapshot.available;
        self.bluetooth_on = snapshot.powered;
        self.bt_discoverable = snapshot.discoverable;
        self.bluetooth_discovering = snapshot.discovering;
        self.bluetooth_adapter_name = snapshot.adapter_name;
        self.bt_devices = snapshot.devices;
    }

    fn begin_bluetooth_mutation(&mut self) {
        self.bluetooth_generation = self.bluetooth_generation.wrapping_add(1);
        self.bluetooth_busy = true;
    }

    fn set_bluetooth_powered(&mut self, powered: bool, cx: &mut Context<Self>) {
        if self.bluetooth_busy || self.bluetooth_loading || !self.bluetooth_available {
            return;
        }
        self.begin_bluetooth_mutation();
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
        self.begin_bluetooth_mutation();
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
        self.begin_bluetooth_mutation();
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
        self.begin_bluetooth_mutation();
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

    fn begin_bluetooth_pairing(
        &mut self,
        device_id: String,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy
            || self.bluetooth_loading
            || !self.bluetooth_available
            || !self.bluetooth_on
            || !self
                .bt_devices
                .iter()
                .any(|device| device.id == device_id && !device.paired)
        {
            return;
        }

        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .clean_on_escape()
                .placeholder("PIN or passkey")
        });
        let editor_focus = editor.read(cx).focus_handle(cx);
        let (session, events) = rmac_bluetooth::PairingSession::new();
        self.begin_bluetooth_mutation();
        let pairing_generation = self.bluetooth_generation;
        self.bluetooth_pairing = Some(BluetoothPairingState {
            device_id: device_id.clone(),
            name,
            session: session.clone(),
            prompt: None,
            display: None,
            editor,
            validation_error: None,
            stopping: false,
        });
        self.bluetooth_error = None;
        window.focus(&editor_focus);
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if !this.bluetooth_busy || this.bluetooth_generation != pairing_generation {
                            return;
                        }
                        let Some(pairing) = &mut this.bluetooth_pairing else {
                            return;
                        };
                        match event {
                            rmac_bluetooth::PairingEvent::Prompt(prompt) => {
                                pairing.prompt = Some(prompt);
                                pairing.display = None;
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::DisplayPinCode { pin_code } => {
                                pairing.prompt = None;
                                pairing.display = Some(BluetoothPairingDisplay::PinCode(pin_code));
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::DisplayPasskey { passkey, entered } => {
                                pairing.prompt = None;
                                pairing.display =
                                    Some(BluetoothPairingDisplay::Passkey { passkey, entered });
                                pairing.validation_error = None;
                            }
                            rmac_bluetooth::PairingEvent::TimedOut => {
                                pairing.prompt = None;
                                pairing.display = None;
                                pairing.validation_error =
                                    Some("Pairing confirmation timed out.".into());
                                pairing.stopping = true;
                            }
                            rmac_bluetooth::PairingEvent::Canceled => {
                                pairing.prompt = None;
                                pairing.display = None;
                                pairing.validation_error = None;
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
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_bluetooth::pair(&device_id, &session);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_bluetooth::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_bluetooth_pairing(result, recovery_snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_bluetooth_pairing(
        &mut self,
        result: std::result::Result<rmac_bluetooth::Snapshot, rmac_bluetooth::Error>,
        recovery_snapshot: Option<rmac_bluetooth::Snapshot>,
    ) {
        self.bluetooth_loading = false;
        self.bluetooth_busy = false;
        self.bluetooth_pairing = None;
        match result {
            Ok(snapshot) => {
                self.apply_bluetooth_snapshot(snapshot);
                self.bluetooth_error = None;
            }
            Err(error) => {
                if let Some(snapshot) = recovery_snapshot {
                    self.apply_bluetooth_snapshot(snapshot);
                }
                if error.is_canceled() || error.is_rejected() {
                    self.bluetooth_error = None;
                } else {
                    self.bluetooth_error =
                        Some(format!("Could not pair Bluetooth device: {error}").into());
                }
            }
        }
    }

    fn submit_bluetooth_pairing_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pairing) = &self.bluetooth_pairing else {
            return;
        };
        let Some(prompt) = pairing.prompt.clone() else {
            return;
        };
        let submitted = match prompt.kind {
            rmac_bluetooth::PairingPromptKind::EnterPinCode => {
                let value = pairing.editor.read(cx).value().to_string();
                match rmac_bluetooth::PairingPinCode::new(value) {
                    Ok(value) => pairing.session.submit_pin_code(prompt.id, value),
                    Err(error) => {
                        if let Some(pairing) = &mut self.bluetooth_pairing {
                            pairing.validation_error = Some(error.to_string().into());
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            rmac_bluetooth::PairingPromptKind::EnterPasskey => {
                let value = pairing.editor.read(cx).value().to_string();
                match rmac_bluetooth::PairingPasskey::new(value) {
                    Ok(value) => pairing.session.submit_passkey(prompt.id, value),
                    Err(error) => {
                        if let Some(pairing) = &mut self.bluetooth_pairing {
                            pairing.validation_error = Some(error.to_string().into());
                        }
                        cx.notify();
                        return;
                    }
                }
            }
            _ => pairing.session.accept(prompt.id),
        };

        if let Some(pairing) = &mut self.bluetooth_pairing {
            if submitted {
                let empty_editor = cx.new(|cx| {
                    InputState::new(window, cx)
                        .clean_on_escape()
                        .placeholder("PIN or passkey")
                });
                let focus = empty_editor.read(cx).focus_handle(cx);
                pairing.editor = empty_editor;
                pairing.prompt = None;
                pairing.display = None;
                pairing.validation_error = None;
                window.focus(&focus);
            } else {
                pairing.validation_error = Some("This pairing request has expired.".into());
            }
        }
        cx.notify();
    }

    fn reject_bluetooth_pairing_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(pairing) = &mut self.bluetooth_pairing else {
            return;
        };
        let Some(prompt) = pairing.prompt.take() else {
            return;
        };
        if pairing.session.reject(prompt.id) {
            pairing.display = None;
            pairing.validation_error = None;
            pairing.stopping = true;
        } else {
            pairing.validation_error = Some("This pairing request has expired.".into());
        }
        cx.notify();
    }

    fn cancel_bluetooth_pairing(&mut self, cx: &mut Context<Self>) {
        let Some(pairing) = &self.bluetooth_pairing else {
            return;
        };
        if pairing.stopping {
            return;
        }
        pairing.session.cancel();
        let device_id = pairing.device_id.clone();
        if let Some(pairing) = &mut self.bluetooth_pairing {
            pairing.prompt = None;
            pairing.display = None;
            pairing.validation_error = None;
            pairing.stopping = true;
        }
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_bluetooth::cancel_pairing(&device_id);
            })
            .detach();
        cx.notify();
    }

    fn request_bluetooth_forget(
        &mut self,
        device_id: String,
        name: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.bluetooth_busy
            || self.bluetooth_loading
            || !self
                .bt_devices
                .iter()
                .any(|device| device.id == device_id && device.paired)
        {
            return;
        }
        self.bluetooth_forget_confirmation = Some(BluetoothForgetPrompt { device_id, name });
        self.bluetooth_error = None;
        cx.notify();
    }

    fn cancel_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
        if !self.bluetooth_busy {
            self.bluetooth_forget_confirmation = None;
            cx.notify();
        }
    }

    fn confirm_bluetooth_forget(&mut self, cx: &mut Context<Self>) {
        if self.bluetooth_busy || self.bluetooth_loading {
            return;
        }
        let Some(device_id) = self
            .bluetooth_forget_confirmation
            .as_ref()
            .map(|prompt| prompt.device_id.clone())
        else {
            return;
        };
        if !self
            .bt_devices
            .iter()
            .any(|device| device.id == device_id && device.paired)
        {
            self.bluetooth_forget_confirmation = None;
            self.bluetooth_error = Some("The Bluetooth device is no longer paired.".into());
            cx.notify();
            return;
        }

        self.begin_bluetooth_mutation();
        self.bluetooth_forgetting = Some(device_id.clone());
        self.bluetooth_forget_confirmation = None;
        self.bluetooth_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery_snapshot) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_bluetooth::remove_device(&device_id);
                    let recovery_snapshot = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_bluetooth::snapshot().ok());
                    (result, recovery_snapshot)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.bluetooth_loading = false;
                this.bluetooth_busy = false;
                this.bluetooth_forgetting = None;
                match result {
                    Ok(snapshot) => {
                        this.apply_bluetooth_snapshot(snapshot);
                        this.bluetooth_error = None;
                    }
                    Err(error) => {
                        if let Some(snapshot) = recovery_snapshot {
                            this.apply_bluetooth_snapshot(snapshot);
                        }
                        this.bluetooth_error =
                            Some(format!("Could not forget Bluetooth device: {error}").into());
                    }
                }
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

    fn select_category(&mut self, name: &str, cx: &mut Context<Self>) {
        let target = self
            .sections
            .iter()
            .enumerate()
            .find_map(|(section, items)| {
                items
                    .iter()
                    .position(|category| category.name.as_ref() == name)
                    .map(|item| (section, item))
            });
        if let Some(target) = target {
            self.selected = target;
            self.nav.clear();
            cx.notify();
        }
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
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(self.account.clone()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
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
                                    .text_size(rmac_ui::text_px(13.0))
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
        debug_assert!(category_has_dedicated_renderer(
            self.current().name.as_ref()
        ));
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
                "Date & Time" => self.render_date_time(cx),
                "Language & Region" => self.render_language_region(cx),
                "Login Items" => self.render_login_items(cx),
                "Sharing" => self.render_sharing(cx),
                "Accessibility" => self.render_accessibility(cx),
                "Privacy & Security" => self.render_privacy_security(cx),
                "Network" => self.render_network(cx),
                "VPN" => self.render_vpn(cx),
                "Desktop & Dock" => self.render_desktop_dock(cx),
                "Spotlight" => self.render_spotlight(cx),
                "Wallpaper" => self.render_wallpaper(cx),
                _ => self.render_unregistered_category(),
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
                    .text_size(rmac_ui::text_px(22.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(cat.name.clone()),
            )
            .child(
                div()
                    .max_w(px(440.0))
                    .text_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary())
                    .child(cat.desc.clone()),
            )
    }

    fn pane(&self, cards: Vec<Div>) -> Div {
        div().v_flex().child(self.render_hero()).children(cards)
    }

    // ---- explicit shell-owned panes ----------------------------------

    fn render_desktop_dock(&self, cx: &Context<Self>) -> Div {
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
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("rmac Dock"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("dock-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.shell_settings_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view
                                    .update(cx, |settings, cx| settings.revert_dock_change(cx));
                            }),
                    )
                    .child(
                        Button::new(
                            "dock-refresh",
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
                                settings.refresh_shell_settings(false, cx)
                            });
                        }),
                    ),
            )];

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative Dock settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. No Dock preference can be changed until it is readable again.",
            ));
            return self.pane(cards);
        };
        let dock = &snapshot.settings.dock;
        let enabled = !self.shell_settings_busy;
        let outputs_live =
            self.dock_compositor.connection == rmac_compositor::ConnectionState::Connected;

        cards.push(section_header("Position and visibility"));
        cards.push(card(vec![
            dock_segment_row(
                view.clone(),
                "dock-placement",
                "Position on screen",
                &DOCK_PLACEMENT_OPTIONS,
                match dock.placement {
                    rmac_shell_settings::DockPlacement::Left => Some(0),
                    rmac_shell_settings::DockPlacement::Bottom => Some(1),
                    rmac_shell_settings::DockPlacement::Right => Some(2),
                },
                enabled,
            ),
            dock_switch_row(
                view.clone(),
                "dock-autohide",
                "Automatically hide and show the Dock",
                Some("Reveal uses deliberate edge pressure so it does not steal focus".into()),
                dock.autohide,
                enabled,
                DockChange::Autohide,
            ),
            dock_switch_row(
                view.clone(),
                "dock-reserve-space",
                "Reserve screen space",
                Some("Keep tiled windows outside the visible Dock area".into()),
                dock.reserve_space,
                enabled,
                DockChange::ReserveSpace,
            ),
        ]));

        cards.push(section_header("Magnification"));
        cards.push(card(vec![
            dock_switch_row(
                view.clone(),
                "dock-magnification",
                "Magnify icons",
                Some("Reduced Motion overrides this effect at runtime".into()),
                dock.magnification,
                enabled,
                DockChange::Magnification,
            ),
            dock_segment_row(
                view.clone(),
                "dock-magnification-scale",
                "Maximum size",
                &DOCK_MAGNIFICATION_OPTIONS,
                [1.25_f32, 1.5, 2.0]
                    .iter()
                    .position(|value| (dock.magnification_scale - value).abs() < f32::EPSILON),
                enabled && dock.magnification,
            ),
        ]));
        if ![1.25_f32, 1.5, 2.0]
            .iter()
            .any(|value| (dock.magnification_scale - value).abs() < f32::EPSILON)
        {
            cards.push(note_card(format!(
                "The saved magnification is {:.2}×. Choose a preset to replace it, or leave it unchanged.",
                dock.magnification_scale
            )));
        }

        cards.push(section_header("Application clicks"));
        cards.push(card(vec![dock_segment_row(
            view.clone(),
            "dock-repeated-click",
            "Click a focused app again",
            &DOCK_REPEATED_CLICK_OPTIONS,
            match dock.repeated_click {
                rmac_shell_settings::RepeatedClickBehavior::CycleWindows => Some(0),
                rmac_shell_settings::RepeatedClickBehavior::DoNothing => Some(1),
                rmac_shell_settings::RepeatedClickBehavior::HideApplication => None,
            },
            enabled,
        )]));
        if dock.repeated_click == rmac_shell_settings::RepeatedClickBehavior::HideApplication {
            cards.push(note_card(
                "The saved behavior requests application hiding, but niri has no application-hide action. The Dock reports that action as unavailable; choose Cycle Windows or Do Nothing for supported behavior.",
            ));
        }

        cards.push(section_header("Displays"));
        let mut output_rows = vec![dock_output_row(
            &view,
            "all",
            "All displays".into(),
            Some("Follow every enabled niri output".into()),
            dock.outputs == rmac_shell_settings::OutputScope::All,
            enabled,
            rmac_shell_settings::OutputScope::All,
        )];
        for output in self
            .dock_compositor
            .outputs
            .values()
            .filter(|output| output.enabled())
        {
            let output_id = output.id.0.clone();
            let display_name = format!("{} {}", output.make, output.model)
                .trim()
                .to_owned();
            let title = if display_name.is_empty() {
                output_id.clone()
            } else {
                display_name
            };
            let selected =
                dock.outputs == rmac_shell_settings::OutputScope::Named(output_id.clone());
            output_rows.push(dock_output_row(
                &view,
                &format!("named-{output_id}"),
                title.into(),
                Some(format!("niri output {output_id}").into()),
                selected,
                enabled && outputs_live,
                rmac_shell_settings::OutputScope::Named(output_id),
            ));
        }
        cards.push(card(output_rows));

        match &dock.outputs {
            rmac_shell_settings::OutputScope::Primary => cards.push(note_card(
                "Primary output is saved, but the current Dock runtime has no authoritative primary-output source and would create no surface. Choose All displays or a connected niri output.",
            )),
            rmac_shell_settings::OutputScope::Named(name)
                if !self
                    .dock_compositor
                    .outputs
                    .values()
                    .any(|output| output.enabled() && output.id.0 == *name) =>
            {
                cards.push(note_card(format!(
                    "The saved output {name} is not currently enabled in niri. The preference is preserved, but the Dock creates no surface there until it returns."
                )));
            }
            _ => {}
        }

        let connection = match self.dock_compositor.connection {
            rmac_compositor::ConnectionState::Connected => "Connected",
            rmac_compositor::ConnectionState::Connecting => "Connecting",
            rmac_compositor::ConnectionState::Reconnecting => "Reconnecting",
            rmac_compositor::ConnectionState::Disconnected => "Unavailable",
        };
        let enabled_outputs = self
            .dock_compositor
            .outputs
            .values()
            .filter(|output| output.enabled())
            .count();
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
                "niri event stream".into(),
                connection.into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Enabled outputs".into(),
                enabled_outputs.to_string().into(),
            ),
        ]));
        if snapshot.recovered_from_last_good || snapshot.migrated_from.is_some() {
            cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                snapshot.migrated_from.map_or_else(
                    || "Recovered the last-known-good Dock preferences.".into(),
                    |version| format!("Migrated Dock preferences from version {version}."),
                )
            })));
        }
        if self.dock_compositor.connection != rmac_compositor::ConnectionState::Connected {
            cards.push(note_card(
                "niri is not connected in this process. Output-specific choices are limited to currently known outputs; saved Dock policy remains editable and is applied when the rmac niri session is available.",
            ));
        }
        cards.push(note_card(
            "These controls configure only the original rmac Dock. They do not modify GNOME or third-party docks, and niri continues to own workspace and window-layout rules.",
        ));
        self.pane(cards)
    }

    fn render_spotlight(&self, cx: &Context<Self>) -> Div {
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
                if *can_configure {
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
        ]));
        if let Some(error) = self.shortcut_status_error.clone() {
            cards.push(note_card(error));
        }
        cards.push(note_card(
            "The portal owns user consent and the actual trigger. The fallback is enabled only when the broker reports it is required, so one shortcut backend owns Logo/Mod+Space at a time.",
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

    fn render_unregistered_category(&self) -> Div {
        self.pane(vec![note_card(
            "This category is not registered with a System Settings renderer. It does not read or change system settings.",
        )])
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
                "Select a network to connect. New WPA Personal and SAE networks ask for their password; enterprise and legacy security remain unavailable until their dedicated setup flows exist.",
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

    fn render_wifi_forget_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.wifi_forget_confirmation.as_ref()?;
        let message = format!(
            "This computer will remove every accessible saved profile for “{}”. If the network is active, it will disconnect. You will need its password to join again.",
            prompt.ssid
        );
        Some(
            rmac_ui::alert(
                "Forget This Network?",
                message,
                vec![
                    rmac_ui::dialog_button(
                        "wifi-forget-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_wifi_forget(cx)))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "wifi-forget-confirm",
                        "Forget",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_wifi_forget(cx)))
                    .into_any_element(),
                ],
            )
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

    fn render_bluetooth_forget_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prompt = self.bluetooth_forget_confirmation.as_ref()?;
        Some(
            rmac_ui::alert(
                "Forget This Device?",
                format!(
                    "This computer will remove pairing information for “{}” and disconnect it. You will need to pair it again to reconnect.",
                    prompt.name
                ),
                vec![
                    rmac_ui::dialog_button(
                        "bluetooth-forget-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_bluetooth_forget(cx)
                    }))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "bluetooth-forget-confirm",
                        "Forget",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_bluetooth_forget(cx)
                    }))
                    .into_any_element(),
                ],
            )
            .into_any_element(),
        )
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
                    GENERAL_DESTINATIONS[0].into(),
                    self.sysinfo.hardware_model.clone().map(Into::into),
                    SubPage::About,
                ),
                nav_row(
                    view.clone(),
                    "icons/refresh-cw.svg",
                    hsl(0x8e8e93),
                    GENERAL_DESTINATIONS[1].into(),
                    Some(self.sysinfo.operating_system.clone().into()),
                    SubPage::SoftwareUpdate,
                ),
                nav_row(
                    view.clone(),
                    "icons/database.svg",
                    hsl(0x8e8e93),
                    GENERAL_DESTINATIONS[2].into(),
                    None,
                    SubPage::Storage,
                ),
            ]),
            note_card(
                "Device continuity and media-receiver controls are hidden until rmac has reviewed Linux service authorities for them.",
            ),
        ];
        self.pane(cards)
    }

    // ---- Date & Time -------------------------------------------------

    fn render_date_time(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-date-time", "Refresh")
            .busy(self.time_busy)
            .disabled(self.time_loading || self.time_busy)
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
                    Button::new("timezone-edit", "Edit").on_click(move |_, window, cx| {
                        edit_view
                            .update(cx, |settings, cx| settings.start_timezone_edit(window, cx));
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
                    snapshot.formatted_local_time().into(),
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
            "Manual clock setting is not connected yet. The hardware clock remains read-only because UTC is the recommended Linux configuration.",
        ));
        self.pane(cards)
    }

    // ---- Language & Region -------------------------------------------

    fn render_language_region(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-language-region", "Refresh")
            .busy(self.locale_busy)
            .disabled(self.locale_loading || self.locale_busy)
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
                        .disabled(self.locale_busy)
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_locale_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let mut cards = vec![card(vec![language_row])];
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
                "The niri config uses includes, so rmac cannot prove which file owns XKB settings. The system default remains read-only until include traversal is implemented."
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
                        Some("Available until the next successful change".into()),
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
            .busy(self.login_item_busy.as_deref() == Some("refresh"))
            .disabled(self.login_items_loading || self.login_item_busy.is_some())
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
                "Review “{}” ({}). {}",
                preview.name,
                preview.id,
                if preview.replacing {
                    "A user entry with this filename exists and will be replaced only after confirmation."
                } else {
                    "The validated entry will be copied into your user autostart directory."
                }
            )));
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
                    let remove_name = item.name.clone();
                    let busy = self.login_item_busy.as_deref() == Some(item.id.as_str());
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
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    remove_view.update(cx, |settings, cx| {
                                        settings.request_remove_login_item(
                                            remove_id.clone(),
                                            remove_name.clone(),
                                            cx,
                                        );
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
        if let Some((_, name)) = &self.login_item_remove {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Remove “{name}”? Its user-owned desktop entry will be moved to Trash. If a system entry with the same filename exists, it will remain visible but disabled."
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
                    .busy(self.theme_busy)
                    .disabled(self.theme_loading || self.theme_busy)
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
                    !self.theme_busy,
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
                    !self.theme_busy,
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
                    !self.theme_busy,
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
                    .busy(self.gtk_text_busy)
                    .disabled(self.gtk_text_loading || self.gtk_text_busy)
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
                        snapshot.writable && !self.gtk_text_busy,
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
                    self.input.can_configure && !self.input_busy,
                ),
                input_switch_row(
                    cx.entity(),
                    "accessibility-middle-emulation",
                    "icons/mouse.svg",
                    "Middle-button emulation",
                    Some("Press the left and right mouse buttons together"),
                    mouse.middle_emulation,
                    self.input.can_configure && !self.input_busy,
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
        cards.push(card(vec![
            row_base()
                .child(tile(
                    "icons/accessibility.svg",
                    if screen_reader.ready() {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    22.0,
                ))
                .child(text_block(
                    "Niri/Orca prerequisites".into(),
                    Some(if screen_reader.ready() {
                        "Detected".into()
                    } else {
                        "Incomplete".into()
                    }),
                ))
                .child(
                    Button::new("refresh-screen-reader", "Refresh")
                        .busy(self.system_data_busy)
                        .disabled(self.system_data_busy)
                        .on_click(move |_, _, cx| {
                            screen_reader_refresh_view
                                .update(cx, |settings, cx| settings.refresh_system_info(cx));
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
                "Xwayland".into(),
                if screen_reader.xwayland {
                    "Available".into()
                } else {
                    "Unavailable".into()
                },
            ),
            value_row(
                "icons/accessibility.svg",
                secondary(),
                "Orca".into(),
                if screen_reader.orca_path.is_some() {
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
        if let Some(limitation) = screen_reader.limitation() {
            cards.push(note_card(limitation));
        } else {
            cards.push(note_card(
                "The session prerequisites are present, but environment detection cannot prove that Xwayland and Orca will operate correctly. Use the niri default shortcut to test speech on the Linux PC.",
            ));
        }
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
                    .busy(self.privacy_loading)
                    .disabled(self.privacy_loading || self.privacy_busy.is_some())
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
                                    .disabled(!snapshot.can_reset || self.privacy_busy.is_some())
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
                            sources.main + sources.restricted,
                            sources.universe + sources.multiverse
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
            "Reset removes only the selected stored portal decision through PermissionStore version 2. Permission tokens are displayed verbatim because the store does not interpret them. Package and application source counts describe provenance signals, not repository trust or the security state of individual applications.",
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
                            .text_size(rmac_ui::text_px(12.0))
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

        cards.push(note_card(
            "Output, microphone, and default devices use the system audio service. Session alert sounds and interface effects stay hidden until the rmac sound policy service exists.",
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

    fn render_keyboard(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
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
                Some(speed_index(settings.accel_speed)),
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "mouse-acceleration",
                "Acceleration",
                &MOUSE_PROFILES,
                Some(usize::from(
                    settings.accel_profile == rmac_input::AccelProfile::Flat,
                )),
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
            input_switch_row(
                cx.entity(),
                "mouse-middle-emulation",
                "icons/mouse.svg",
                "Middle-button emulation",
                Some("Press the left and right buttons together for middle click"),
                settings.middle_emulation,
                self.input.can_configure && !self.input_busy,
                InputChange::MouseMiddleEmulation,
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
                Some(speed_index(settings.pointer.accel_speed)),
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "touchpad-acceleration",
                "Acceleration",
                &TOUCHPAD_PROFILES,
                Some(usize::from(
                    settings.pointer.accel_profile == rmac_input::AccelProfile::Flat,
                )),
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
            input_switch_row(
                cx.entity(),
                "touchpad-middle-emulation",
                "icons/touchpad.svg",
                "Middle-click emulation",
                Some("Press the left and right click areas together"),
                settings.pointer.middle_emulation,
                self.input.can_configure && !self.input_busy,
                InputChange::TouchpadMiddleEmulation,
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
                    .when(self.display_revert.is_some(), |actions| {
                        actions.child(
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

    /// Direct per-volume capacity state from the mount service.
    fn storage_body(&self, cx: &Context<Self>) -> Div {
        let refresh_view = cx.entity();
        let mut body = div().v_flex().child(card(vec![row_base()
            .child(tile("icons/hard-drive.svg", accent(), 22.0))
            .child(text_block(
                "Mounted volumes".into(),
                Some("System and user-visible removable volumes".into()),
            ))
            .child(
                Button::new("refresh-storage", "Refresh")
                    .busy(self.storage_busy)
                    .disabled(self.storage_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_storage(cx));
                    }),
            )
            .into_any_element()]));

        if self.storage.is_empty() {
            return body.child(
                EmptyState::new("No storage volumes available")
                    .message("Refresh after the mount service becomes available")
                    .error(self.storage_error.is_some()),
            );
        }

        for volume in &self.storage {
            body = body.child(section_header(volume.mount.name.clone()));
            let Some(usage) = volume.usage else {
                body = body.child(card(vec![row_base()
                    .child(tile("icons/hard-drive.svg", hsl(0xff9500), 22.0))
                    .child(text_block(
                        volume.mount.name.clone().into(),
                        volume.usage_error.clone().map(Into::into),
                    ))
                    .into_any_element()]));
                continue;
            };
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
                                .items_baseline()
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(15.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(volume.mount.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .text_color(secondary())
                                        .child(format!(
                                            "{} available of {}",
                                            fmt_gb(usage.available),
                                            fmt_gb(usage.total)
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
            "Storage categories and cleanup actions stay hidden until they can be measured and reversed safely.",
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
                "Some updates are blocked by package dependencies. Ubuntu Software Updater can show the dependency details.",
            ));
        }
        body.child(note_card(
            "Checking is live. Download and installation are not connected in this build; use Ubuntu Software Updater to review and apply changes.",
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

        let mut facts = vec![
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
        ];
        if let Some(graphics) = &self.display.graphics {
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
            .or_else(|| self.updates_error.clone())
            .or_else(|| self.storage_error.clone())
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
            .or_else(|| self.vpn_error.clone())
            .or_else(|| self.audio_error.clone())
            .or_else(|| self.power_error.clone())
            .or_else(|| self.display_error.clone())
            .or_else(|| self.input_error.clone())
            .or_else(|| self.theme_error.clone())
            .or_else(|| self.shell_settings_error.clone())
            .or_else(|| self.shell_settings_stream_error.clone())
            .or_else(|| self.wallpaper_error.clone())
            .or_else(|| self.spotlight_error.clone())
            .or_else(|| self.gtk_text_error.clone())
            .or_else(|| self.privacy_error.clone())
            .or_else(|| self.privacy_stream_error.clone());
        let wifi_password_dialog = self.render_wifi_password_dialog(cx);
        let wifi_forget_dialog = self.render_wifi_forget_dialog(cx);
        let bluetooth_pairing_dialog = self.render_bluetooth_pairing_dialog(cx);
        let bluetooth_forget_dialog = self.render_bluetooth_forget_dialog(cx);
        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.wifi_forget_confirmation.is_some() {
                    cx.stop_propagation();
                    this.cancel_wifi_forget(cx);
                } else if event.keystroke.key == "escape"
                    && this.bluetooth_forget_confirmation.is_some()
                {
                    cx.stop_propagation();
                    this.cancel_bluetooth_forget(cx);
                }
            }))
            .on_action(cx.listener(|t, _: &GoBack, _, cx| t.go_back(cx)))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, _| {
                if this.wifi_forgetting.is_some() || this.bluetooth_forgetting.is_some() {
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
                            this.updates_error = None;
                            this.storage_error = None;
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
                            this.vpn_error = None;
                            this.audio_error = None;
                            this.power_error = None;
                            this.display_error = None;
                            this.input_error = None;
                            this.theme_error = None;
                            this.shell_settings_error = None;
                            this.shell_settings_stream_error = None;
                            this.wallpaper_error = None;
                            this.spotlight_error = None;
                            this.gtk_text_error = None;
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
            .when_some(wifi_forget_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_pairing_dialog, |root, dialog| root.child(dialog))
            .when_some(bluetooth_forget_dialog, |root, dialog| root.child(dialog))
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
    snapshot
        .locale
        .iter()
        .find(|assignment| assignment.key == key)
        .map(|assignment| assignment.value.clone())
        .unwrap_or_else(|| snapshot.language().to_owned())
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

type InputOption = (&'static str, InputChange);
type ThemeOption = (&'static str, ThemeChange);
type GtkTextScaleOption = (&'static str, f64);

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
const THEME_TEXT_SCALE_OPTIONS: [ThemeOption; 3] = [
    (
        "Standard",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::Standard),
    ),
    (
        "Large",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::Large),
    ),
    (
        "Extra Large",
        ThemeChange::TextScale(rmac_theme::TextScalePreference::ExtraLarge),
    ),
];
const GTK_TEXT_SCALE_OPTIONS: [GtkTextScaleOption; 3] =
    [("Standard", 1.0), ("Large", 1.2), ("Extra Large", 1.3)];

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
const KEYBOARD_RESPONSE_PRESETS: [InputOption; 3] = [
    (
        "Standard",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 600,
            rate: 25,
        },
    ),
    (
        "Deliberate",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 1_000,
            rate: 15,
        },
    ),
    (
        "Minimal",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 1_500,
            rate: 10,
        },
    ),
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
const MOUSE_PRECISION_PRESETS: [InputOption; 3] = [
    (
        "Standard",
        InputChange::MousePrecisionPreset {
            speed: 0.0,
            profile: rmac_input::AccelProfile::Adaptive,
        },
    ),
    (
        "Steady",
        InputChange::MousePrecisionPreset {
            speed: -0.5,
            profile: rmac_input::AccelProfile::Adaptive,
        },
    ),
    (
        "Precise",
        InputChange::MousePrecisionPreset {
            speed: -0.5,
            profile: rmac_input::AccelProfile::Flat,
        },
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

// ---- platform reads kept off the UI thread -------------------------------

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
        sysinfo: rmac_system_info::snapshot(),
        storage: rmac_mounts::volumes(),
        screen_reader: gather_screen_reader_capability(),
    }
}

fn gather_screen_reader_capability() -> ScreenReaderCapability {
    let niri_session = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .any(|value| {
            value
                .split([':', ';'])
                .any(|desktop| desktop.eq_ignore_ascii_case("niri"))
        });
    let xwayland = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty());
    let orca_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .take(128)
            .map(|directory| directory.join("orca"))
            .find(|candidate| is_executable_file(candidate))
    });
    ScreenReaderCapability {
        niri_session,
        xwayland,
        orca_path,
    }
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
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

fn categories() -> Vec<Vec<Category>> {
    let blue = hsl(0x0a84ff);
    let gray = hsl(0x8e8e93);
    let green = hsl(0x34c759);
    let red = hsl(0xff3b30);
    let pink = hsl(0xff2d55);
    let indigo = hsl(0x5e5ce6);
    let teal = hsl(0x30b0c7);

    let cat = |name: &str, icon: &'static str, color: Hsla, desc: &str| Category {
        name: name.to_string().into(),
        icon,
        color,
        desc: desc.to_string().into(),
    };

    vec![
        vec![
            cat(
                "Wi-Fi",
                "icons/wifi.svg",
                blue,
                "Connect to Wi-Fi networks and manage known networks.",
            ),
            cat(
                "Bluetooth",
                "icons/bluetooth.svg",
                blue,
                "Pair and manage Bluetooth devices.",
            ),
            cat(
                "Network",
                "icons/globe.svg",
                blue,
                "Configure network services and connections.",
            ),
            cat(
                "VPN",
                "icons/key.svg",
                blue,
                "Set up and manage VPN configurations.",
            ),
            cat(
                "Battery",
                "icons/battery-charging.svg",
                green,
                "Monitor battery usage and energy settings.",
            ),
        ],
        vec![
            cat(
                "General",
                "icons/settings.svg",
                gray,
                "View system information, update status, and storage.",
            ),
            cat(
                "Date & Time",
                "icons/clock.svg",
                blue,
                "Adjust the time zone and network time synchronization.",
            ),
            cat(
                "Language & Region",
                "icons/languages.svg",
                blue,
                "Choose the system language and inspect regional formats.",
            ),
            cat(
                "Login Items",
                "icons/app-window.svg",
                blue,
                "Choose applications and services that start when you sign in.",
            ),
            cat(
                "Sharing",
                "icons/globe.svg",
                blue,
                "Control reviewed remote access and file-sharing services.",
            ),
            cat(
                "Accessibility",
                "icons/accessibility.svg",
                blue,
                "Customize the computer for the way you work.",
            ),
            cat(
                "Appearance",
                "icons/palette.svg",
                hsl(0x1d1d1f),
                "Change how windows, buttons, and menus look.",
            ),
            cat(
                "Desktop & Dock",
                "icons/app-window.svg",
                gray,
                "Choose authoritative rmac Dock behavior and display placement.",
            ),
            cat(
                "Displays",
                "icons/monitor.svg",
                blue,
                "Arrange displays and adjust resolution.",
            ),
            cat(
                "Spotlight",
                "icons/search.svg",
                gray,
                "Choose search results, file privacy, and excluded folders.",
            ),
            cat(
                "Wallpaper",
                "icons/image.svg",
                teal,
                "Choose original or local images for every niri display.",
            ),
        ],
        vec![
            cat(
                "Notifications",
                "icons/bell.svg",
                red,
                "Choose how you receive notifications.",
            ),
            cat(
                "Sound",
                "icons/volume-2.svg",
                pink,
                "Adjust sound effects and output.",
            ),
            cat(
                "Keyboard",
                "icons/keyboard.svg",
                gray,
                "Adjust key repeat behavior and keyboard startup options.",
            ),
            cat(
                "Mouse",
                "icons/mouse.svg",
                gray,
                "Adjust tracking, scrolling, acceleration, and buttons.",
            ),
            cat(
                "Trackpad",
                "icons/touchpad.svg",
                gray,
                "Adjust tracking, tapping, scrolling, and gestures.",
            ),
            cat(
                "Focus",
                "icons/moon.svg",
                indigo,
                "Stay focused by silencing notifications.",
            ),
        ],
        vec![
            cat(
                "Lock Screen",
                "icons/lock.svg",
                gray,
                "Adjust your lock screen and login.",
            ),
            cat(
                "Privacy & Security",
                "icons/shield.svg",
                blue,
                "Control what the system and applications can access.",
            ),
        ],
    ]
}

fn category_name_for_pane_id(pane_id: &str) -> Option<&'static str> {
    Some(match pane_id {
        "wifi" => "Wi-Fi",
        "bluetooth" => "Bluetooth",
        "network" => "Network",
        "vpn" => "VPN",
        "battery" => "Battery",
        "general" => "General",
        "date-time" => "Date & Time",
        "language-region" => "Language & Region",
        "login-items" => "Login Items",
        "sharing" => "Sharing",
        "accessibility" => "Accessibility",
        "appearance" => "Appearance",
        "desktop-dock" => "Desktop & Dock",
        "displays" => "Displays",
        "spotlight" => "Spotlight",
        "wallpaper" => "Wallpaper",
        "notifications" => "Notifications",
        "sound" => "Sound",
        "keyboard" => "Keyboard",
        "mouse" => "Mouse",
        "trackpad" => "Trackpad",
        "focus" => "Focus",
        "lock-screen" => "Lock Screen",
        "privacy-security" => "Privacy & Security",
        _ => return None,
    })
}

fn category_position(sections: &[Vec<Category>], name: &str) -> Option<(usize, usize)> {
    sections
        .iter()
        .enumerate()
        .find_map(|(section, categories)| {
            categories
                .iter()
                .position(|category| category.name == name)
                .map(|row| (section, row))
        })
}

fn category_has_dedicated_renderer(name: &str) -> bool {
    matches!(
        name,
        "Wi-Fi"
            | "Bluetooth"
            | "Network"
            | "VPN"
            | "Battery"
            | "General"
            | "Date & Time"
            | "Language & Region"
            | "Login Items"
            | "Sharing"
            | "Accessibility"
            | "Appearance"
            | "Desktop & Dock"
            | "Displays"
            | "Spotlight"
            | "Wallpaper"
            | "Notifications"
            | "Sound"
            | "Keyboard"
            | "Mouse"
            | "Trackpad"
            | "Focus"
            | "Lock Screen"
            | "Privacy & Security"
    )
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
        bluetooth_stream_snapshot_is_current, categories, category_has_dedicated_renderer,
        category_name_for_pane_id, category_position, composite_wallpaper_pixel,
        notification_policy_with, render_wallpaper_preview, wallpaper_selection,
        wifi_stream_snapshot_is_current, DockChange, NotificationPolicyChange,
        ScreenReaderCapability, ShellSettingsMutation, SpotlightAuthority, SpotlightChange,
        WallpaperChange, WallpaperTarget, GENERAL_DESTINATIONS,
    };

    #[test]
    fn general_navigation_contains_only_truthful_destinations() {
        assert_eq!(
            GENERAL_DESTINATIONS,
            ["About", "Software Update", "Storage"]
        );
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
    fn optional_unowned_panes_and_generic_clickable_rows_are_absent() {
        let categories = categories().into_iter().flatten().collect::<Vec<_>>();
        let names = categories
            .iter()
            .map(|category| category.name.to_string())
            .collect::<Vec<_>>();
        assert!(!names.iter().any(|name| name == "Assistant & Intelligence"));
        assert!(!names.iter().any(|name| name == "Screen Time"));

        assert!(!names.iter().any(|name| name == "Handoff"));
        assert!(names.iter().any(|name| name == "Language & Region"));
        assert!(names.iter().any(|name| name == "Login Items"));
        assert!(names.iter().any(|name| name == "Sharing"));
        assert!(names
            .iter()
            .all(|name| category_has_dedicated_renderer(name)));
    }

    #[test]
    fn every_launcher_setting_destination_routes_to_a_visible_category() {
        let sections = categories();
        for entry in rmac_launcher_providers::system_settings_entries() {
            let category = category_name_for_pane_id(&entry.pane_id)
                .unwrap_or_else(|| panic!("missing category route for {}", entry.pane_id));
            assert!(category_position(&sections, category).is_some());
        }
        assert!(category_name_for_pane_id("assistant").is_none());
        assert!(category_name_for_pane_id("screen-time").is_none());
    }

    #[test]
    fn screen_reader_readiness_requires_every_niri_orca_authority() {
        let mut capability = ScreenReaderCapability::default();
        assert_eq!(
            capability.limitation(),
            Some("Start the desktop through a full niri-session")
        );
        capability.niri_session = true;
        assert_eq!(
            capability.limitation(),
            Some("Xwayland is required by Orca in the current niri integration")
        );
        capability.xwayland = true;
        assert_eq!(
            capability.limitation(),
            Some("Install Orca to enable screen-reader support")
        );
        capability.orca_path = Some("/usr/bin/orca".into());
        assert!(capability.ready());
        assert_eq!(capability.limitation(), None);
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
