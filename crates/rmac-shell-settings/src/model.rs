//! Stable shared shell settings model.

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AppId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProviderId(pub String);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockPlacement {
    Left,
    #[default]
    Bottom,
    Right,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "scope", content = "output", rename_all = "kebab-case")]
pub enum OutputScope {
    #[default]
    All,
    Primary,
    Named(String),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepeatedClickBehavior {
    CycleWindows,
    HideApplication,
    /// macOS: clicking the frontmost application's Dock icon does nothing.
    #[default]
    DoNothing,
}

fn default_magnification_scale() -> f32 {
    1.5
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DockSettings {
    pub placement: DockPlacement,
    pub outputs: OutputScope,
    pub autohide: bool,
    pub magnification: bool,
    pub magnification_scale: f32,
    pub reserve_space: bool,
    pub repeated_click: RepeatedClickBehavior,
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            placement: DockPlacement::Bottom,
            outputs: OutputScope::All,
            autohide: false,
            magnification: false,
            magnification_scale: default_magnification_scale(),
            reserve_space: true,
            repeated_click: RepeatedClickBehavior::DoNothing,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockFormat {
    #[default]
    Locale,
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ClockSettings {
    pub format: ClockFormat,
    pub show_date: bool,
    pub show_seconds: bool,
    pub show_workspace: bool,
}

impl Default for ClockSettings {
    fn default() -> Self {
        Self {
            format: ClockFormat::Locale,
            show_date: true,
            show_seconds: false,
            show_workspace: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct IndicatorSettings {
    pub network: bool,
    pub vpn: bool,
    pub bluetooth: bool,
    pub sound: bool,
    pub power: bool,
    pub battery_percentage: bool,
    pub notifications: bool,
    pub focus: bool,
}

impl Default for IndicatorSettings {
    fn default() -> Self {
        Self {
            network: true,
            vpn: true,
            bluetooth: true,
            sound: true,
            power: true,
            battery_percentage: false,
            notifications: true,
            focus: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WallpaperFit {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WallpaperSelection {
    pub source: Option<String>,
    pub fit: WallpaperFit,
}

impl Default for WallpaperSelection {
    fn default() -> Self {
        Self {
            source: None,
            fit: WallpaperFit::Fill,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WallpaperSettings {
    pub default: WallpaperSelection,
    pub per_output: BTreeMap<String, WallpaperSelection>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct FocusSettings {
    pub enabled: bool,
    pub selected_mode: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ProviderPolicy {
    pub enabled: bool,
    pub allow_private_content: bool,
    pub allow_network: bool,
}

impl Default for ProviderPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_content: false,
            allow_network: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SpotlightSettings {
    /// Absolute directory roots omitted from filename and recent-document
    /// results. Shell text and URI forms are deliberately unsupported.
    pub excluded_paths: Vec<String>,
    /// Search across filesystem device boundaries. Off by default so mounted
    /// removable media is never traversed merely by opening the launcher.
    pub include_removable_mounts: bool,
}

/// What a screen corner does when the pointer reaches it (Desktop & Dock ›
/// Hot Corners). Only actions rmac can perform are offered; macOS's Quick
/// Note, screen saver and display sleep have no rmac backend yet.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HotCornerAction {
    #[default]
    None,
    MissionControl,
    ApplicationWindows,
    Desktop,
    NotificationCenter,
    Apps,
    LockScreen,
}

impl HotCornerAction {
    /// Every choice in the order macOS 26 lists them, "-" (none) last.
    pub const ALL: [Self; 7] = [
        Self::MissionControl,
        Self::ApplicationWindows,
        Self::Desktop,
        Self::NotificationCenter,
        Self::Apps,
        Self::LockScreen,
        Self::None,
    ];

    /// The pop-up title System Settings shows for this action.
    pub fn title(self) -> &'static str {
        match self {
            Self::None => "-",
            Self::MissionControl => "Mission Control",
            Self::ApplicationWindows => "Application Windows",
            Self::Desktop => "Desktop",
            Self::NotificationCenter => "Notification Centre",
            Self::Apps => "Apps",
            Self::LockScreen => "Lock Screen",
        }
    }
}

/// The four hot corners. All are off by default: the Mac's default Quick
/// Note corner has no rmac backend.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct HotCornerSettings {
    pub top_left: HotCornerAction,
    pub top_right: HotCornerAction,
    pub bottom_left: HotCornerAction,
    pub bottom_right: HotCornerAction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ShellSettings {
    pub pinned_apps: Vec<AppId>,
    pub dock: DockSettings,
    pub clock: ClockSettings,
    pub indicators: IndicatorSettings,
    pub wallpaper: WallpaperSettings,
    pub focus: FocusSettings,
    pub providers: BTreeMap<ProviderId, ProviderPolicy>,
    pub spotlight: SpotlightSettings,
    pub hot_corners: HotCornerSettings,
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            // The owner's reference Dock (FEEL_SPEC.md §C): Files leads
            // implicitly, then Apps, Notes, Text Editor, Terminal, System
            // Settings. No browser and no Downloads are pinned.
            pinned_apps: [
                rmac_apps::identity::FILES,
                rmac_apps::identity::APP_DRAWER,
                rmac_apps::identity::NOTES,
                rmac_apps::identity::TEXT_EDITOR,
                rmac_apps::identity::TERMINAL,
                rmac_apps::identity::SYSTEM_SETTINGS,
            ]
            .into_iter()
            .map(|identity| AppId(identity.into()))
            .collect(),
            dock: DockSettings::default(),
            clock: ClockSettings::default(),
            indicators: IndicatorSettings::default(),
            wallpaper: WallpaperSettings::default(),
            focus: FocusSettings::default(),
            providers: BTreeMap::new(),
            spotlight: SpotlightSettings::default(),
            hot_corners: HotCornerSettings::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub settings: ShellSettings,
    pub path: PathBuf,
    pub recovered_from_last_good: bool,
    pub migrated_from: Option<u32>,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolvePath,
    CreateDirectory,
    ReadSettings,
    WatchSettings,
    ParseSettings,
    MigrateSettings,
    ValidateSettings,
    SerializeSettings,
    SaveLastGood,
    SaveSettings,
    RestoreLastGood,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolvePath => "resolve the shell settings path",
            Self::CreateDirectory => "create the shell settings directory",
            Self::ReadSettings => "read shell settings",
            Self::WatchSettings => "watch shell settings",
            Self::ParseSettings => "parse shell settings",
            Self::MigrateSettings => "migrate shell settings",
            Self::ValidateSettings => "validate shell settings",
            Self::SerializeSettings => "serialize shell settings",
            Self::SaveLastGood => "save last-known-good shell settings",
            Self::SaveSettings => "save shell settings",
            Self::RestoreLastGood => "restore last-known-good shell settings",
        })
    }
}

pub type Error = Failure<Operation>;
