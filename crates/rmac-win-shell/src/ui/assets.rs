//! The shell's own artwork, embedded: the Lulo apps' icons and the menu
//! bar's glyphs, the same files Lulo OS ships (`packaging/rmac-apps/icons`
//! and `shell/assets`). Everything else falls through to rmac-ui's shared
//! icons.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! assets {
    ($($path:literal => $file:literal),* $(,)?) => {
        const ASSETS: &[(&str, &[u8])] = &[$(($path, include_bytes!($file))),*];
    };
}

assets! {
    "apps/org.rmac.Files.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Files.svg",
    "desktop/folder.svg" => "../../../../assets/icons/folder.svg",
    "dock/trash-empty.svg" => "../../../rmac-dock/assets/icons/trash-empty.svg",
    "dock/trash-full.svg" => "../../../rmac-dock/assets/icons/trash-full.svg",
    "apps/org.rmac.Notes.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Notes.svg",
    "apps/org.rmac.Calculator.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Calculator.svg",
    "apps/org.rmac.Clock.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Clock.svg",
    "apps/org.rmac.Weather.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Weather.svg",
    "apps/org.rmac.Preview.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Preview.svg",
    "apps/org.rmac.TextEditor.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.TextEditor.svg",
    "apps/org.rmac.Terminal.svg" => "../../../../packaging/rmac-apps/icons/org.rmac.Terminal.svg",
    "status/rmac.svg" => "../../../../shell/assets/status/rmac.svg",
    "status/spotlight.svg" => "../../../../shell/assets/status/spotlight.svg",
    "status/battery.svg" => "../../../../shell/assets/status/battery.svg",
    "symbols/battery-charging.svg" => "../../../../shell/assets/symbols/battery-charging.svg",
    "symbols/search.svg" => "../../../../shell/assets/symbols/search.svg",
    "symbols/folder.svg" => "../../../../shell/assets/symbols/folder.svg",
    "symbols/wifi-1.svg" => "../../../../shell/assets/symbols/wifi-1.svg",
    "symbols/wifi-2.svg" => "../../../../shell/assets/symbols/wifi-2.svg",
    "symbols/wifi-3.svg" => "../../../../shell/assets/symbols/wifi-3.svg",
    "symbols/speaker-0.svg" => "../../../../shell/assets/symbols/speaker-0.svg",
    "symbols/speaker-1.svg" => "../../../../shell/assets/symbols/speaker-1.svg",
    "symbols/speaker-2.svg" => "../../../../shell/assets/symbols/speaker-2.svg",
    "symbols/speaker-3.svg" => "../../../../shell/assets/symbols/speaker-3.svg",
    "symbols/speaker-muted.svg" => "../../../../shell/assets/symbols/speaker-muted.svg",
}

pub struct ShellAssets;

impl AssetSource for ShellAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

/// The Wi-Fi glyph for one to three bars.
pub fn wifi_glyph(bars: u8) -> &'static str {
    match bars {
        0 | 1 => "symbols/wifi-1.svg",
        2 => "symbols/wifi-2.svg",
        _ => "symbols/wifi-3.svg",
    }
}

/// The speaker glyph for a volume level (0 to 1).
pub fn speaker_glyph(level: f32, muted: bool) -> &'static str {
    if muted {
        "symbols/speaker-muted.svg"
    } else if level <= 0.001 {
        "symbols/speaker-0.svg"
    } else if level < 0.34 {
        "symbols/speaker-1.svg"
    } else if level < 0.67 {
        "symbols/speaker-2.svg"
    } else {
        "symbols/speaker-3.svg"
    }
}
