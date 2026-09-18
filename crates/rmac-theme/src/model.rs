use rmac_appearance::{
    AccentColor, ColorScheme, Contrast, MotionPreference, ResolvedAppearance, ResolvedColorScheme,
    Snapshot as HostSnapshot, TextScale,
};
use rmac_storage::{Failure, FileSystem};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::store::{default_accent, invalid_accent, validate_preferences};

pub(crate) const CURRENT_VERSION: u32 = 1;
pub(crate) const DEFAULT_ACCENT: (f64, f64, f64) = (0.0, 0.478_431_372_5, 1.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchemePreference {
    Automatic,
    Light,
    // The owner's reference Mac runs Dark (FEEL_SPEC.md §C), so a fresh rmac
    // session is dark until the user chooses otherwise.
    #[default]
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "rgb", rename_all = "kebab-case")]
pub enum AccentPreference {
    #[default]
    Automatic,
    Custom([f64; 3]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContrastPreference {
    #[default]
    Automatic,
    Normal,
    Higher,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionPreferenceSetting {
    #[default]
    Automatic,
    Full,
    Reduced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextScalePreference {
    #[default]
    Standard,
    Large,
    ExtraLarge,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub color_scheme: SchemePreference,
    pub accent_color: AccentPreference,
    pub contrast: ContrastPreference,
    pub motion: MotionPreferenceSetting,
    pub text_scale: TextScalePreference,
}

impl Preferences {
    pub fn resolve(&self, host: &HostSnapshot) -> Result<ResolvedAppearance, Error> {
        validate_preferences(self, Path::new("theme.json"))?;
        let color_scheme = match self.color_scheme {
            SchemePreference::Automatic => match host.color_scheme {
                ColorScheme::PreferDark => ResolvedColorScheme::Dark,
                ColorScheme::NoPreference | ColorScheme::PreferLight => ResolvedColorScheme::Light,
            },
            SchemePreference::Light => ResolvedColorScheme::Light,
            SchemePreference::Dark => ResolvedColorScheme::Dark,
        };
        let accent_color = match self.accent_color {
            AccentPreference::Automatic => host.accent_color.unwrap_or_else(default_accent),
            AccentPreference::Custom([red, green, blue]) => AccentColor::new(red, green, blue)
                .ok_or_else(|| invalid_accent(Path::new("theme.json")))?,
        };
        let contrast = match self.contrast {
            ContrastPreference::Automatic => host.contrast,
            ContrastPreference::Normal => Contrast::Normal,
            ContrastPreference::Higher => Contrast::Higher,
        };
        let motion = match self.motion {
            MotionPreferenceSetting::Automatic => host.motion,
            MotionPreferenceSetting::Full => MotionPreference::Full,
            MotionPreferenceSetting::Reduced => MotionPreference::Reduced,
        };
        let text_scale = match self.text_scale {
            TextScalePreference::Standard => TextScale::Standard,
            TextScalePreference::Large => TextScale::Large,
            TextScalePreference::ExtraLarge => TextScale::ExtraLarge,
        };
        Ok(ResolvedAppearance {
            color_scheme,
            accent_color,
            contrast,
            motion,
            text_scale,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub preferences: Preferences,
    pub effective: ResolvedAppearance,
    pub path: PathBuf,
    pub recovered_from_last_good: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    ResolvePath,
    CreateDirectory,
    ReadPreferences,
    WatchPreferences,
    ParsePreferences,
    ValidatePreferences,
    SerializePreferences,
    SaveLastGood,
    SavePreferences,
    RestoreLastGood,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolvePath => "resolve the theme preference path",
            Self::CreateDirectory => "create the theme preference directory",
            Self::ReadPreferences => "read theme preferences",
            Self::WatchPreferences => "watch theme preferences",
            Self::ParsePreferences => "parse theme preferences",
            Self::ValidatePreferences => "validate theme preferences",
            Self::SerializePreferences => "serialize theme preferences",
            Self::SaveLastGood => "save the last-known-good theme preferences",
            Self::SavePreferences => "save theme preferences",
            Self::RestoreLastGood => "restore the previous last-known-good theme preferences",
        })
    }
}

pub type Error = Failure<Operation>;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoredPreferences {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) preferences: Preferences,
}

pub struct ThemeStore<B = FileSystem> {
    pub(crate) path: PathBuf,
    pub(crate) backend: B,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StoreEvent {
    Changed,
    WatchError(Error),
}

/// Keeps the platform watcher alive and exposes coalescible store events.
pub struct ThemeWatcher {
    pub(crate) events: async_channel::Receiver<StoreEvent>,
    pub(crate) _watcher: notify::RecommendedWatcher,
}

impl ThemeWatcher {
    pub async fn recv(&self) -> Result<StoreEvent, async_channel::RecvError> {
        self.events.recv().await
    }

    pub fn try_recv(&self) -> Result<StoreEvent, async_channel::TryRecvError> {
        self.events.try_recv()
    }
}
