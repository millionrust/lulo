//! Display-only labels of history records: when each arrived, and what its
//! sender said and the kernel reported about who sent it.
//!
//! A label only names, times and stacks a card, the way macOS shows the
//! sending app's name, icon and a relative time. It never feeds policy,
//! replacement or authorization, so every field is optional and an invalid
//! one is dropped rather than failing the whole history file.

use super::*;

const MAX_DESKTOP_ID_BYTES: usize = 255;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Label {
    /// Wall-clock arrival, in milliseconds since the Unix epoch.
    pub posted_unix_ms: Option<u64>,
    /// The desktop-entry ID of the sender's systemd app scope
    /// (kernel-reported).
    pub desktop_id: Option<String>,
    /// The sender's executable (kernel-reported).
    pub executable: Option<String>,
    /// The sender's `desktop-entry` hint.
    pub hinted_desktop_id: Option<String>,
    /// The sender's `app_name`.
    pub app_name: Option<String>,
    /// The sender's `image-path` hint or `app_icon`: an icon-theme name, an
    /// absolute path or a `file://` URI.
    pub icon: Option<String>,
}

impl Label {
    /// The same label with every field that fails validation removed.
    pub fn sanitized(self) -> Self {
        Self {
            posted_unix_ms: self.posted_unix_ms.filter(|posted| *posted != 0),
            desktop_id: self.desktop_id.filter(|id| valid_desktop_id(id)),
            executable: self.executable.filter(|path| valid_executable(path)),
            hinted_desktop_id: self
                .hinted_desktop_id
                .map(|id| id.strip_suffix(".desktop").unwrap_or(&id).to_owned())
                .filter(|id| valid_desktop_id(id)),
            app_name: self
                .app_name
                .map(|name| name.trim().to_owned())
                .filter(|name| valid_text(name, MAX_NAME_BYTES)),
            icon: self.icon.filter(|icon| valid_icon(icon)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

pub fn valid_desktop_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DESKTOP_ID_BYTES
        && !id.starts_with('.')
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

pub fn valid_executable(path: &str) -> bool {
    path.starts_with('/') && valid_text(path, MAX_PATH_BYTES)
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

/// A theme name (`org.gnome.Nautilus`, `dialog-information`), an absolute
/// path or a `file://` URI. Relative paths and other schemes are refused.
fn valid_icon(icon: &str) -> bool {
    if !valid_text(icon, MAX_PATH_BYTES) {
        return false;
    }
    if icon.starts_with('/') {
        return !icon.split('/').any(|segment| segment == "..");
    }
    if let Some(path) = icon.strip_prefix("file://") {
        return path.starts_with('/') && !path.split('/').any(|segment| segment == "..");
    }
    icon.len() <= MAX_NAME_BYTES
        && icon
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-+".contains(character))
}

#[derive(Deserialize, Serialize)]
pub(super) struct StoredLabel {
    id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    posted_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hinted_desktop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
}

impl StoredLabel {
    pub(super) fn from_label(id: NotificationId, label: &Label) -> Self {
        Self {
            id: id.get(),
            posted_unix_ms: label.posted_unix_ms,
            desktop_id: label.desktop_id.clone(),
            executable: label.executable.clone(),
            hinted_desktop_id: label.hinted_desktop_id.clone(),
            app_name: label.app_name.clone(),
            icon: label.icon.clone(),
        }
    }

    /// `None` for an unusable ID or a label with nothing valid left.
    pub(super) fn into_label(self) -> Option<(NotificationId, Label)> {
        let id = NotificationId::from_protocol(self.id)?;
        let label = Label {
            posted_unix_ms: self.posted_unix_ms,
            desktop_id: self.desktop_id,
            executable: self.executable,
            hinted_desktop_id: self.hinted_desktop_id,
            app_name: self.app_name,
            icon: self.icon,
        }
        .sanitized();
        (!label.is_empty()).then_some((id, label))
    }
}
