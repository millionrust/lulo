//! The real backends each page writes to. Every function is blocking or a
//! plain future and is run off the UI thread by the view.

use std::path::{Path, PathBuf};

/// The directory Ubuntu's `gnome-control-center-faces` package fills with
/// account pictures. Without it only the monogram is offered.
pub const FACES_DIRECTORY: &str = "/usr/share/pixmaps/faces";
const MAX_FACES: usize = 8;

/// The signed-in user as AccountsService reports them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub user_name: String,
    pub real_name: String,
    /// The current picture, if it is a readable file.
    pub icon_file: Option<PathBuf>,
}

/// Up to eight pictures from [`FACES_DIRECTORY`], sorted by name.
pub fn faces() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(FACES_DIRECTORY) else {
        return Vec::new();
    };
    let mut faces = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg"
                        )
                    })
        })
        .collect::<Vec<_>>();
    faces.sort();
    faces.truncate(MAX_FACES);
    faces
}

#[cfg(target_os = "linux")]
mod accounts {
    use super::*;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    const SERVICE: &str = "org.freedesktop.Accounts";

    fn user_proxy(connection: &Connection) -> Result<Proxy<'static>, String> {
        let accounts = Proxy::new(
            connection,
            SERVICE,
            "/org/freedesktop/Accounts",
            "org.freedesktop.Accounts",
        )
        .map_err(|error| error.to_string())?;
        // SAFETY: getuid has no preconditions.
        let uid = i64::from(unsafe { libc::getuid() });
        let path: OwnedObjectPath = accounts
            .call("FindUserById", &(uid,))
            .map_err(|error| error.to_string())?;
        Proxy::new(
            connection,
            SERVICE,
            path.into_inner(),
            "org.freedesktop.Accounts.User",
        )
        .map_err(|error| error.to_string())
    }

    pub fn account() -> Result<Account, String> {
        let connection = Connection::system().map_err(|error| error.to_string())?;
        let user = user_proxy(&connection)?;
        let user_name: String = user
            .get_property("UserName")
            .map_err(|error| error.to_string())?;
        let real_name: String = user
            .get_property("RealName")
            .map_err(|error| error.to_string())?;
        let icon_file: String = user.get_property("IconFile").unwrap_or_default();
        let icon_file = Some(PathBuf::from(icon_file)).filter(|path| path.is_file());
        Ok(Account {
            user_name,
            real_name,
            icon_file,
        })
    }

    /// AccountsService lets the active user change their own name and
    /// picture without an administrator password
    /// (org.freedesktop.accounts.change-own-user-data).
    pub fn set_real_name(name: &str) -> Result<(), String> {
        let connection = Connection::system().map_err(|error| error.to_string())?;
        user_proxy(&connection)?
            .call::<_, _, ()>("SetRealName", &(name,))
            .map_err(|error| error.to_string())
    }

    pub fn set_icon_file(path: &Path) -> Result<(), String> {
        let path = path
            .to_str()
            .ok_or_else(|| "the picture path is not UTF-8".to_owned())?;
        let connection = Connection::system().map_err(|error| error.to_string())?;
        user_proxy(&connection)?
            .call::<_, _, ()>("SetIconFile", &(path,))
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
pub use accounts::{account, set_icon_file, set_real_name};

#[cfg(not(target_os = "linux"))]
pub fn account() -> Result<Account, String> {
    Err("accounts are available in the Lulo OS Linux session".into())
}

#[cfg(not(target_os = "linux"))]
pub fn set_real_name(_name: &str) -> Result<(), String> {
    account().map(|_| ())
}

#[cfg(not(target_os = "linux"))]
pub fn set_icon_file(_path: &Path) -> Result<(), String> {
    account().map(|_| ())
}

/// Apply the chosen language (`LANG`) and region (the format categories)
/// through systemd-localed, which asks for an administrator password.
pub fn apply_locale(
    snapshot: &rmac_locale::Snapshot,
    language: &str,
    region: &str,
) -> Result<rmac_locale::Snapshot, String> {
    let with_language = snapshot
        .preview_language(language)
        .map_err(|error| error.to_string())?;
    let mut staged = snapshot.clone();
    staged.locale = with_language;
    let assignments = staged
        .preview_region(region)
        .map_err(|error| error.to_string())?;
    if rmac_locale::locale_assignments_match(&assignments, &snapshot.locale) {
        return Ok(snapshot.clone());
    }
    let encoded = assignments
        .iter()
        .map(rmac_locale::Assignment::encoded)
        .collect::<Vec<_>>();
    rmac_locale_linux::set_locale(&encoded).map_err(|error| error.to_string())
}

async fn theme_host() -> rmac_appearance::Snapshot {
    match rmac_appearance_portal::snapshot().await {
        Ok(host) => host,
        Err(_) => rmac_appearance::Snapshot::unavailable(
            "The desktop Settings portal is temporarily unavailable.",
        ),
    }
}

/// The saved Light / Dark / Auto preference.
pub async fn color_scheme() -> Result<rmac_theme::SchemePreference, String> {
    let host = theme_host().await;
    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the appearance preferences are unavailable".to_owned())?;
    store
        .load(&host)
        .map(|theme| theme.preferences.color_scheme)
        .map_err(|_| "the appearance preferences could not be read".to_owned())
}

/// Save Light / Dark / Auto the way System Settings › Appearance does.
pub async fn set_color_scheme(scheme: rmac_theme::SchemePreference) -> Result<(), String> {
    let host = theme_host().await;
    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the appearance preferences are unavailable".to_owned())?;
    let mut preferences = store
        .load(&host)
        .map_err(|_| "the appearance preferences could not be read".to_owned())?
        .preferences;
    if preferences.color_scheme == scheme {
        return Ok(());
    }
    preferences.color_scheme = scheme;
    store
        .save(&preferences, &host)
        .map(|_| ())
        .map_err(|_| "the appearance preference could not be saved".to_owned())
}
