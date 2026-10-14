//! Preview's persisted app-wide preferences (Settings window ⌘,), saved at
//! `~/.config/rmac/preview.json`. Mirrors `crates/weather/src/store.rs`'s
//! settings pattern.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "rmac/preview.json";
const MAX_SETTINGS_BYTES: usize = 16 * 1024;
/// A sane cap on how many toolbar items a settings file can list as
/// hidden, and how long each item's name can be.
const MAX_HIDDEN_TOOLBAR_ITEMS: usize = 32;
const MAX_TOOLBAR_ITEM_LEN: usize = 40;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    /// Settings ▸ General ▸ Window background: an 0xRRGGBB colour behind
    /// a transparent image or outside a PDF page, applied to new windows
    /// (View ▸ Show Image Background can still override it per-window).
    pub window_background: u32,
    /// Settings ▸ Images: default state of View ▸ Show Image Background
    /// for a newly opened image window.
    pub show_image_background_default: bool,
    /// Settings ▸ PDF: default state of View ▸ Use Dark Appearance for
    /// PDF for a newly opened PDF window.
    pub dark_appearance_for_pdf_default: bool,
    /// View ▸ Customise Toolbar…: names of built-in toolbar controls the
    /// user has hidden (an allow-list of opt-OUT names the toolbar render
    /// code checks membership against), e.g. "sidebar", "zoom", "markup",
    /// "search". Stored as strings (not an enum) so an old settings file
    /// from a future Lulo version with more toolbar items still loads.
    pub hidden_toolbar_items: Vec<String>,
    /// Settings ▸ Images ▸ When opening files: several files chosen together
    /// open in one window (the Mac's default) or each in its own.
    pub open_each_in_own_window: bool,
    /// Settings ▸ PDF ▸ On opening documents: "Start on the last viewed
    /// page".
    pub start_on_last_viewed_page: bool,
    /// Settings ▸ PDF ▸ Opening for the first time: Show as.
    pub pdf_layout: PdfLayout,
    /// Settings ▸ PDF ▸ Annotations: "Add name to annotations" and the
    /// name, written as each new annotation's author (`/T`).
    pub add_name_to_annotations: bool,
    pub annotation_name: String,
}

/// Settings ▸ PDF ▸ Opening for the first time: Show as.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum PdfLayout {
    /// Preview's own default: single pages, scrolling continuously.
    #[default]
    Automatically,
    ContinuousScroll,
    SinglePage,
    TwoPages,
}

impl PdfLayout {
    pub const ALL: [Self; 4] = [
        Self::Automatically,
        Self::ContinuousScroll,
        Self::SinglePage,
        Self::TwoPages,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatically => "Automatically",
            Self::ContinuousScroll => "Continuous Scroll",
            Self::SinglePage => "Single Page",
            Self::TwoPages => "Two Pages",
        }
    }

    /// (two pages side by side, continuous scrolling) for a new window.
    pub fn view_mode(self) -> (bool, bool) {
        match self {
            Self::Automatically | Self::ContinuousScroll => (false, true),
            Self::SinglePage => (false, false),
            Self::TwoPages => (true, false),
        }
    }
}

const MAX_ANNOTATION_NAME_LEN: usize = 128;

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Measured macOS 26 Preview document-area colour (light
            // appearance); see crates/preview/src/metrics.rs's
            // `light::DOCUMENT`.
            window_background: 0xE9E9ED,
            show_image_background_default: false,
            dark_appearance_for_pdf_default: false,
            hidden_toolbar_items: Vec::new(),
            open_each_in_own_window: false,
            start_on_last_viewed_page: false,
            pdf_layout: PdfLayout::default(),
            add_name_to_annotations: false,
            annotation_name: String::new(),
        }
    }
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        for item in &mut self.hidden_toolbar_items {
            *item = item.chars().take(MAX_TOOLBAR_ITEM_LEN).collect();
        }
        let mut seen = BTreeSet::new();
        self.hidden_toolbar_items
            .retain(|item| seen.insert(item.clone()));
        self.hidden_toolbar_items.truncate(MAX_HIDDEN_TOOLBAR_ITEMS);
        self.annotation_name = self
            .annotation_name
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_ANNOTATION_NAME_LEN)
            .collect();
        self
    }

    /// The author new annotations carry, if "Add name to annotations" is on
    /// and a name is set.
    pub fn annotation_author(&self) -> Option<String> {
        let name = self.annotation_name.trim();
        (self.add_name_to_annotations && !name.is_empty()).then(|| name.to_owned())
    }
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

pub fn settings_path() -> Option<PathBuf> {
    config_root().map(|root| root.join(SETTINGS_FILE))
}

pub fn load_settings_from(path: &Path) -> io::Result<Settings> {
    match rmac_storage::read_bounded_no_follow(path, MAX_SETTINGS_BYTES) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map(Settings::normalized)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error),
    }
}

pub fn save_settings_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&settings.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

pub fn load_settings() -> io::Result<Settings> {
    load_settings_from(&settings_path().ok_or_else(no_home)?)
}

pub fn save_settings(settings: &Settings) -> io::Result<()> {
    save_settings_to(&settings_path().ok_or_else(no_home)?, settings)
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rmac-preview-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn annotation_author_needs_the_switch_and_a_name() {
        let mut settings = Settings {
            annotation_name: " Jacob\u{7} ".to_owned(),
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.annotation_author(), None);
        settings.add_name_to_annotations = true;
        assert_eq!(settings.annotation_author().as_deref(), Some("Jacob"));
        settings.annotation_name.clear();
        assert_eq!(settings.annotation_author(), None);
        assert_eq!(PdfLayout::TwoPages.view_mode(), (true, false));
        assert_eq!(PdfLayout::Automatically.view_mode(), (false, true));
    }

    #[test]
    fn missing_file_loads_defaults() {
        let directory = scratch("missing");
        let path = directory.join("preview.json");
        assert_eq!(load_settings_from(&path).unwrap(), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let directory = scratch("roundtrip");
        let path = directory.join("nested/preview.json");
        assert!(!path.parent().unwrap().exists());
        let settings = Settings {
            window_background: 0x112233,
            show_image_background_default: true,
            dark_appearance_for_pdf_default: true,
            hidden_toolbar_items: vec!["sidebar".to_owned(), "zoom".to_owned()],
            open_each_in_own_window: true,
            start_on_last_viewed_page: true,
            pdf_layout: PdfLayout::TwoPages,
            add_name_to_annotations: true,
            annotation_name: "Jacob".to_owned(),
        };
        save_settings_to(&path, &settings).unwrap();
        assert!(path.exists());
        assert_eq!(load_settings_from(&path).unwrap(), settings);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn corrupt_file_errors_instead_of_silently_defaulting() {
        let directory = scratch("corrupt");
        let path = directory.join("preview.json");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_settings_from(&path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn normalized_dedupes_and_truncates_hidden_toolbar_items() {
        let mut settings = Settings {
            hidden_toolbar_items: vec!["sidebar".to_owned(), "sidebar".to_owned()],
            ..Settings::default()
        };
        for index in 0..40 {
            settings.hidden_toolbar_items.push(format!("item-{index}"));
        }
        settings.hidden_toolbar_items.push("x".repeat(100));
        let normalized = settings.normalized();
        assert_eq!(
            normalized
                .hidden_toolbar_items
                .iter()
                .filter(|item| *item == "sidebar")
                .count(),
            1
        );
        assert!(normalized.hidden_toolbar_items.len() <= MAX_HIDDEN_TOOLBAR_ITEMS);
        assert!(normalized
            .hidden_toolbar_items
            .iter()
            .all(|item| item.chars().count() <= MAX_TOOLBAR_ITEM_LEN));
    }
}
