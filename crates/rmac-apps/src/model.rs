//! Stable application catalog and launch model.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    pub source: PathBuf,
    pub icon: Option<PathBuf>,
    pub categories: Vec<String>,
    /// Exact MIME types advertised by the desktop entry. An empty list means
    /// the application did not claim file-handler support.
    pub mime_types: Vec<String>,
    pub launch: LaunchSpec,
    /// Additional launcher actions declared by the desktop entry, in the
    /// author's `Actions=` order. macOS catalog entries currently omit these.
    pub actions: Vec<DesktopAction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub launch: LaunchSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAssociation {
    pub mime_type: String,
    pub default_application_id: Option<String>,
    pub handlers: Vec<Application>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFileWithError {
    pub default_changed: bool,
    pub(super) detail: String,
}

impl std::fmt::Display for OpenFileWithError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for OpenFileWithError {}

impl From<io::Error> for OpenFileWithError {
    fn from(error: io::Error) -> Self {
        Self {
            default_changed: false,
            detail: error.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ApplicationSource {
    Flatpak,
    Snap,
    AppImage,
    UserDesktopEntry,
    SystemDesktopEntry,
    OtherDesktopEntry,
    MacApplication,
}

impl ApplicationSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::Snap => "Snap",
            Self::AppImage => "AppImage",
            Self::UserDesktopEntry => "User desktop entries",
            Self::SystemDesktopEntry => "System desktop entries",
            Self::OtherDesktopEntry => "Other desktop entries",
            Self::MacApplication => "macOS applications",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceInventory {
    pub flatpak: usize,
    pub snap: usize,
    pub appimage: usize,
    pub user_desktop_entries: usize,
    pub system_desktop_entries: usize,
    pub other_desktop_entries: usize,
    pub mac_applications: usize,
}

impl SourceInventory {
    pub fn total(&self) -> usize {
        self.flatpak
            + self.snap
            + self.appimage
            + self.user_desktop_entries
            + self.system_desktop_entries
            + self.other_desktop_entries
            + self.mac_applications
    }
}

impl Application {
    /// Lower-cased, display-safe metadata used by application pickers. Source
    /// paths and command lines are deliberately excluded.
    pub fn searchable_text(&self) -> String {
        std::iter::once(self.name.to_lowercase())
            .chain(self.generic_name.iter().map(|value| value.to_lowercase()))
            .chain(self.keywords.iter().map(|value| value.to_lowercase()))
            .chain(self.categories.iter().map(|value| value.to_lowercase()))
            .chain(self.actions.iter().map(|action| action.name.to_lowercase()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn source_kind(&self) -> ApplicationSource {
        let source = self.source.to_string_lossy();
        if source.contains("/flatpak/exports/share/applications/") {
            ApplicationSource::Flatpak
        } else if source.contains("/snapd/desktop/applications/") {
            ApplicationSource::Snap
        } else if self.id.starts_with("appimagekit_")
            || matches!(&self.launch, LaunchSpec::Command { program, .. } if program.to_ascii_lowercase().ends_with(".appimage"))
        {
            ApplicationSource::AppImage
        } else if self.source.extension().and_then(|value| value.to_str()) == Some("app") {
            ApplicationSource::MacApplication
        } else if source.contains("/.local/share/applications/") {
            ApplicationSource::UserDesktopEntry
        } else if source.starts_with("/usr/share/applications/")
            || source.starts_with("/usr/local/share/applications/")
        {
            ApplicationSource::SystemDesktopEntry
        } else {
            ApplicationSource::OtherDesktopEntry
        }
    }
}

pub fn source_inventory(applications: &[Application]) -> SourceInventory {
    let mut inventory = SourceInventory::default();
    for application in applications {
        match application.source_kind() {
            ApplicationSource::Flatpak => inventory.flatpak += 1,
            ApplicationSource::Snap => inventory.snap += 1,
            ApplicationSource::AppImage => inventory.appimage += 1,
            ApplicationSource::UserDesktopEntry => inventory.user_desktop_entries += 1,
            ApplicationSource::SystemDesktopEntry => inventory.system_desktop_entries += 1,
            ApplicationSource::OtherDesktopEntry => inventory.other_desktop_entries += 1,
            ApplicationSource::MacApplication => inventory.mac_applications += 1,
        }
    }
    inventory
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchSpec {
    OpenPath(PathBuf),
    Command {
        program: String,
        args: Vec<String>,
        working_dir: Option<PathBuf>,
        terminal: bool,
    },
}
