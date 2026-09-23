//! The desktop's saved state (`~/.config/rmac/desktop.json`): icon
//! positions, Sort By, Use Stacks, the view options and the widgets. The
//! wallpaper process writes it; Notification Centre reads the widgets it
//! shows. A widget-gallery request file under the runtime directory lets
//! Notification Centre's "Edit Widgets" open the gallery.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::grid::{Placement, ViewOptions};
use crate::widgets::{Widget, WidgetKind, WidgetLocation, WidgetSize, MAX_WIDGETS};
use crate::{SortOrder, MAX_DESKTOP_ITEMS};

const SETTINGS_FILE: &str = "rmac/desktop.json";
const GALLERY_REQUEST_FILE: &str = "desktop-widget-gallery";
const MAX_SETTINGS_BYTES: usize = 256 * 1024;

/// Finder's desktop "Sort By": `None` keeps icons where they were put.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arrangement {
    #[default]
    None,
    SnapToGrid,
    Name,
    Kind,
    DateModified,
    Size,
}

impl Arrangement {
    /// The scan order the icons are laid out in.
    pub fn sort_order(self) -> SortOrder {
        match self {
            Self::None | Self::SnapToGrid | Self::Name => SortOrder::Name,
            Self::Kind => SortOrder::Kind,
            Self::DateModified => SortOrder::DateModified,
            Self::Size => SortOrder::Size,
        }
    }

    /// Whether icons are laid out in sorted order rather than kept where
    /// they were put.
    pub fn is_sorted(self) -> bool {
        !matches!(self, Self::None | Self::SnapToGrid)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DesktopSettings {
    pub arrangement: Arrangement,
    pub use_stacks: bool,
    pub view: ViewOptions,
    /// Icon positions by file name, for the unsorted desktop.
    pub positions: BTreeMap<String, Placement>,
    pub widgets: Vec<Widget>,
    pub next_widget_id: u64,
}

impl DesktopSettings {
    pub fn normalized(mut self) -> Self {
        self.view = self.view.normalized();
        self.positions
            .retain(|name, placement| !name.is_empty() && placement.is_valid());
        while self.positions.len() > MAX_DESKTOP_ITEMS {
            let Some(name) = self.positions.keys().next().cloned() else {
                break;
            };
            self.positions.remove(&name);
        }
        self.widgets.retain(Widget::is_valid);
        self.widgets.truncate(MAX_WIDGETS);
        let highest = self.widgets.iter().map(|widget| widget.id).max();
        if let Some(highest) = highest {
            self.next_widget_id = self.next_widget_id.max(highest + 1);
        }
        self
    }

    /// Forgets positions of items no longer on the Desktop.
    pub fn retain_positions<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let names = names.into_iter().collect::<std::collections::BTreeSet<_>>();
        self.positions
            .retain(|name, _| names.contains(name.as_str()));
    }

    pub fn add_widget(
        &mut self,
        kind: WidgetKind,
        size: WidgetSize,
        location: WidgetLocation,
    ) -> Option<u64> {
        if self.widgets.len() >= MAX_WIDGETS || !kind.sizes().contains(&size) {
            return None;
        }
        let id = self.next_widget_id.max(1);
        self.next_widget_id = id + 1;
        self.widgets.push(Widget {
            id,
            kind,
            size,
            location,
        });
        Some(id)
    }

    pub fn remove_widget(&mut self, id: u64) -> bool {
        let before = self.widgets.len();
        self.widgets.retain(|widget| widget.id != id);
        self.widgets.len() != before
    }

    pub fn widget_mut(&mut self, id: u64) -> Option<&mut Widget> {
        self.widgets.iter_mut().find(|widget| widget.id == id)
    }

    pub fn notification_center_widgets(&self) -> impl Iterator<Item = &Widget> {
        self.widgets
            .iter()
            .filter(|widget| widget.location == WidgetLocation::NotificationCenter)
    }
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
}

pub fn settings_path() -> Option<PathBuf> {
    config_root().map(|root| root.join(SETTINGS_FILE))
}

pub fn load_from(path: &Path) -> io::Result<DesktopSettings> {
    match rmac_storage::read_bounded_no_follow(path, MAX_SETTINGS_BYTES) {
        Ok(bytes) => serde_json::from_slice::<DesktopSettings>(&bytes)
            .map(DesktopSettings::normalized)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DesktopSettings::default()),
        Err(error) => Err(error),
    }
}

pub fn save_to(path: &Path, settings: &DesktopSettings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&settings.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

pub fn load() -> io::Result<DesktopSettings> {
    load_from(&settings_path().ok_or_else(no_home)?)
}

pub fn save(settings: &DesktopSettings) -> io::Result<()> {
    save_to(&settings_path().ok_or_else(no_home)?, settings)
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home directory")
}

/// Where a widget chosen in the gallery is added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GalleryTarget {
    Desktop,
    NotificationCenter,
}

impl GalleryTarget {
    fn token(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::NotificationCenter => "notification-center",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "desktop" => Some(Self::Desktop),
            "notification-center" => Some(Self::NotificationCenter),
            _ => None,
        }
    }
}

/// `$XDG_RUNTIME_DIR/rmac/desktop`, the private directory the request
/// file lives in.
pub fn runtime_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("rmac").join("desktop"))
}

/// Creates the request directory so it can be watched.
pub fn prepare_runtime_directory() -> io::Result<PathBuf> {
    let directory = runtime_directory().ok_or_else(no_home)?;
    if let Some(parent) = directory.parent() {
        std::fs::create_dir_all(parent)?;
    }
    rmac_storage::create_dir_all_private(&directory)?;
    Ok(directory)
}

pub fn gallery_request_path() -> Option<PathBuf> {
    runtime_directory().map(|directory| directory.join(GALLERY_REQUEST_FILE))
}

/// Asks the wallpaper process to open the widget gallery.
pub fn request_gallery(target: GalleryTarget) -> io::Result<()> {
    let path = prepare_runtime_directory()?.join(GALLERY_REQUEST_FILE);
    rmac_storage::atomic_write_private(&path, target.token().as_bytes())
}

/// Reads and removes a pending gallery request.
pub fn take_gallery_request() -> Option<GalleryTarget> {
    let path = gallery_request_path()?;
    let bytes = rmac_storage::read_bounded_no_follow(&path, 64).ok()?;
    let _ = std::fs::remove_file(&path);
    GalleryTarget::parse(std::str::from_utf8(&bytes).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn settings_round_trip_and_drop_invalid_entries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rmac-desktop-settings-{nonce}"));
        let path = root.join("desktop.json");
        assert_eq!(load_from(&path).unwrap(), DesktopSettings::default());
        let mut settings = DesktopSettings {
            arrangement: Arrangement::Kind,
            use_stacks: true,
            ..DesktopSettings::default()
        };
        settings.positions.insert(
            "a.txt".to_owned(),
            Placement {
                from_right: 34.0,
                top: 41.0,
            },
        );
        let clock = settings
            .add_widget(
                WidgetKind::Clock,
                WidgetSize::Small,
                WidgetLocation::Desktop {
                    left: 20.0,
                    top: 53.0,
                },
            )
            .unwrap();
        assert!(settings
            .add_widget(
                WidgetKind::Clock,
                WidgetSize::Medium,
                WidgetLocation::NotificationCenter
            )
            .is_none());
        let weather = settings
            .add_widget(
                WidgetKind::Weather,
                WidgetSize::Medium,
                WidgetLocation::NotificationCenter,
            )
            .unwrap();
        assert_ne!(clock, weather);
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, settings.clone().normalized());
        assert_eq!(loaded.notification_center_widgets().count(), 1);
        std::fs::remove_dir_all(root).unwrap();

        let mut stale = settings;
        stale.retain_positions(["b.txt"]);
        assert!(stale.positions.is_empty());
        assert!(stale.remove_widget(clock));
        assert!(!stale.remove_widget(clock));
    }

    #[test]
    fn next_widget_id_never_reuses_a_saved_id() {
        let settings = DesktopSettings {
            widgets: vec![Widget {
                id: 7,
                kind: WidgetKind::Battery,
                size: WidgetSize::Small,
                location: WidgetLocation::NotificationCenter,
            }],
            next_widget_id: 0,
            ..DesktopSettings::default()
        }
        .normalized();
        assert_eq!(settings.next_widget_id, 8);
    }

    #[test]
    fn arrangement_decides_scan_order_and_whether_icons_move_freely() {
        assert!(!Arrangement::None.is_sorted());
        assert!(!Arrangement::SnapToGrid.is_sorted());
        assert!(Arrangement::DateModified.is_sorted());
        assert_eq!(Arrangement::Size.sort_order(), SortOrder::Size);
        assert_eq!(
            GalleryTarget::parse(GalleryTarget::NotificationCenter.token()),
            Some(GalleryTarget::NotificationCenter)
        );
    }
}
