use std::path::PathBuf;
use std::time::Duration;

use gpui::SharedString;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DockChange {
    Placement(rmac_shell_settings::DockPlacement),
    Outputs(rmac_shell_settings::OutputScope),
    Autohide(bool),
    Magnification(bool),
    MagnificationScale(f32),
    ReserveSpace(bool),
    RepeatedClick(rmac_shell_settings::RepeatedClickBehavior),
}

impl DockChange {
    pub(super) fn apply(self, dock: &mut rmac_shell_settings::DockSettings) {
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

/// One of the four hot corners in Desktop & Dock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HotCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HotCorner {
    pub(super) const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::TopLeft => "Top Left",
            Self::TopRight => "Top Right",
            Self::BottomLeft => "Bottom Left",
            Self::BottomRight => "Bottom Right",
        }
    }

    pub(super) fn id(self) -> &'static str {
        match self {
            Self::TopLeft => "hot-corner-top-left",
            Self::TopRight => "hot-corner-top-right",
            Self::BottomLeft => "hot-corner-bottom-left",
            Self::BottomRight => "hot-corner-bottom-right",
        }
    }

    pub(super) fn get(
        self,
        corners: &rmac_shell_settings::HotCornerSettings,
    ) -> rmac_shell_settings::HotCornerAction {
        match self {
            Self::TopLeft => corners.top_left,
            Self::TopRight => corners.top_right,
            Self::BottomLeft => corners.bottom_left,
            Self::BottomRight => corners.bottom_right,
        }
    }

    fn set(
        self,
        corners: &mut rmac_shell_settings::HotCornerSettings,
        action: rmac_shell_settings::HotCornerAction,
    ) {
        match self {
            Self::TopLeft => corners.top_left = action,
            Self::TopRight => corners.top_right = action,
            Self::BottomLeft => corners.bottom_left = action,
            Self::BottomRight => corners.bottom_right = action,
        }
    }
}

/// A Hot Corners change: what one corner does. `rmac-mission-control`
/// watches this field and maps a corner surface for every corner in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HotCornerChange {
    pub(super) corner: HotCorner,
    pub(super) action: rmac_shell_settings::HotCornerAction,
}

impl HotCornerChange {
    pub(super) fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        self.corner.set(&mut settings.hot_corners, self.action);
    }
}

/// A Menu Bar pane change: which status items the rmac menu bar shows, and
/// the clock's seconds. `rmac-shell-status` reads exactly these fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum MenuBarChange {
    Network(bool),
    Vpn(bool),
    Bluetooth(bool),
    Sound(bool),
    Power(bool),
    BatteryPercentage(bool),
    Notifications(bool),
    Focus(bool),
    ShowSeconds(bool),
}

impl MenuBarChange {
    pub(super) fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        let indicators = &mut settings.indicators;
        match self {
            Self::Network(value) => indicators.network = value,
            Self::Vpn(value) => indicators.vpn = value,
            Self::Bluetooth(value) => indicators.bluetooth = value,
            Self::Sound(value) => indicators.sound = value,
            Self::Power(value) => indicators.power = value,
            Self::BatteryPercentage(value) => indicators.battery_percentage = value,
            Self::Notifications(value) => indicators.notifications = value,
            Self::Focus(value) => indicators.focus = value,
            Self::ShowSeconds(value) => settings.clock.show_seconds = value,
        }
    }
}

pub(super) enum ShellSettingsStreamUpdate {
    Snapshot(Box<rmac_shell_settings::Snapshot>),
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WallpaperTarget {
    Default,
    Output(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum WallpaperChange {
    Source(Option<String>),
    Fit(rmac_shell_settings::WallpaperFit),
    UseDefault,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SpotlightAuthority {
    providers: std::collections::BTreeMap<
        rmac_shell_settings::ProviderId,
        rmac_shell_settings::ProviderPolicy,
    >,
    spotlight: rmac_shell_settings::SpotlightSettings,
}

impl SpotlightAuthority {
    pub(super) fn from_settings(settings: &rmac_shell_settings::ShellSettings) -> Self {
        Self {
            providers: settings.providers.clone(),
            spotlight: settings.spotlight.clone(),
        }
    }

    pub(super) fn apply_to(self, settings: &mut rmac_shell_settings::ShellSettings) {
        settings.providers = self.providers;
        settings.spotlight = self.spotlight;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SpotlightChange {
    ProviderEnabled { id: String, enabled: bool },
    ProviderPrivateContent { id: String, allowed: bool },
    ProviderNetwork { id: String, allowed: bool },
    IncludeRemovableMounts(bool),
    AddExclusion(String),
    RemoveExclusion(String),
}

impl SpotlightChange {
    pub(super) fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        match self {
            Self::ProviderEnabled { id, enabled } => {
                update_provider_policy(settings, id, |policy| policy.enabled = enabled);
            }
            Self::ProviderPrivateContent { id, allowed } => {
                update_provider_policy(settings, id, |policy| {
                    policy.allow_private_content = allowed;
                });
            }
            Self::ProviderNetwork { id, allowed } => {
                update_provider_policy(settings, id, |policy| {
                    policy.allow_network = allowed;
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
    pub(super) fn apply(
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

pub(super) enum ShellSettingsMutation {
    Change(DockChange),
    Restore(rmac_shell_settings::DockSettings),
    Wallpaper {
        target: WallpaperTarget,
        change: WallpaperChange,
    },
    RestoreWallpaper(rmac_shell_settings::WallpaperSettings),
    Spotlight(SpotlightChange),
    RestoreSpotlight(SpotlightAuthority),
    MenuBar(MenuBarChange),
    HotCorner(HotCornerChange),
    /// Desktop & Dock › Click wallpaper to reveal desktop.
    /// `rmac-mission-control` reads it on every wallpaper click.
    ClickWallpaperToReveal(rmac_shell_settings::ClickWallpaperToReveal),
}

impl ShellSettingsMutation {
    pub(super) fn apply(self, settings: &mut rmac_shell_settings::ShellSettings) {
        match self {
            Self::Change(change) => change.apply(&mut settings.dock),
            Self::Restore(dock) => settings.dock = dock,
            Self::Wallpaper { target, change } => {
                change.apply(&target, &mut settings.wallpaper);
            }
            Self::RestoreWallpaper(wallpaper) => settings.wallpaper = wallpaper,
            Self::Spotlight(change) => change.apply(settings),
            Self::RestoreSpotlight(spotlight) => spotlight.apply_to(settings),
            Self::MenuBar(change) => change.apply(settings),
            Self::HotCorner(change) => change.apply(settings),
            Self::ClickWallpaperToReveal(value) => settings.click_wallpaper_to_reveal = value,
        }
    }
}

pub(super) async fn watch_shell_settings(sender: async_channel::Sender<ShellSettingsStreamUpdate>) {
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

pub(super) fn persist_shell_settings_mutation(
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

pub(super) fn wallpaper_selection(
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

pub(super) fn wallpaper_source_name(
    selection: &rmac_shell_settings::WallpaperSelection,
) -> SharedString {
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

pub(super) fn composite_wallpaper_pixel(source: [u8; 4], background: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(source[3]);
    let blend = |channel: usize| {
        ((u32::from(source[channel]) * alpha + u32::from(background[channel]) * (255 - alpha))
            / 255) as u8
    };
    [blend(0), blend(1), blend(2), 255]
}

pub(super) fn render_wallpaper_preview(
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

pub(super) fn validate_wallpaper_choice(
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

pub(super) fn spotlight_provider_policy(
    settings: &rmac_shell_settings::ShellSettings,
    id: &str,
) -> rmac_shell_settings::ProviderPolicy {
    settings
        .providers
        .get(&rmac_shell_settings::ProviderId(id.to_owned()))
        .cloned()
        .unwrap_or_default()
}

pub(super) fn validate_search_exclusion(path: PathBuf) -> std::result::Result<String, String> {
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
