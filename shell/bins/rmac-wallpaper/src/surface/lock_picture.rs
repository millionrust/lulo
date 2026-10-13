//! LOCK-01: writes the small, fixed-size raw pixel caches the lock screen
//! (`crates/rmac-lock-provider-linux/src/picture.rs`) reads instead of
//! decoding an image itself. This process already decodes the desktop
//! wallpaper and already links the `image` crate, and unlike the lock
//! provider it never holds the user's typed password, so it is the safe
//! place to do that work (see `paint.rs`'s module doc for why the split
//! exists at all).

use std::path::PathBuf;

fn cache_path(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    Some(base.join("rmac").join(name))
}

/// Re-derived from the current wallpaper every time it is (re)decoded for
/// the desktop, so the lock screen picks up a wallpaper change on the next
/// lock without polling anything.
pub(crate) fn write_background(image: &rmac_wallpaper_image::Decoded) {
    let Some(thumbnail) = rmac_wallpaper_image::lock_thumbnail(image) else {
        return;
    };
    let Some(path) = cache_path("lock-wallpaper.rgb") else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if rmac_storage::create_dir_all_private(parent).is_err() {
        return;
    }
    if let Err(error) = rmac_storage::atomic_write_private(&path, &thumbnail) {
        eprintln!("the lock screen's wallpaper thumbnail could not be saved: {error}");
    }
}

/// Matches `crates/rmac-lock-provider-linux/src/picture.rs` `AVATAR_DIAMETER`
/// and `paint::layout::AVATAR_DIAMETER`.
const AVATAR_DIAMETER: u32 = 56;

/// The signed-in user's account picture: AccountsService's `IconFile`, or
/// `~/.face` (the same file `gnome-control-center-faces`/greeters use) when
/// AccountsService has none or is unreachable.
fn account_icon_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(path) = accounts_service_icon() {
        return Some(path);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".face"))
        .filter(|path| path.is_file())
}

#[cfg(target_os = "linux")]
fn accounts_service_icon() -> Option<PathBuf> {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    const SERVICE: &str = "org.freedesktop.Accounts";
    let connection = Connection::system().ok()?;
    let accounts = Proxy::new(
        &connection,
        SERVICE,
        "/org/freedesktop/Accounts",
        "org.freedesktop.Accounts",
    )
    .ok()?;
    // SAFETY: getuid has no preconditions.
    let uid = i64::from(unsafe { libc::getuid() });
    let path: OwnedObjectPath = accounts.call("FindUserById", &(uid,)).ok()?;
    let user = Proxy::new(
        &connection,
        SERVICE,
        path.into_inner(),
        "org.freedesktop.Accounts.User",
    )
    .ok()?;
    let icon_file: String = user.get_property("IconFile").ok()?;
    Some(PathBuf::from(icon_file)).filter(|path| path.is_file())
}

/// Reads and writes the account-picture cache once. Called at startup; an
/// account picture changed later is picked up on the next sign-in to this
/// desktop session, not live — an honest limitation rather than adding a
/// poll loop for something that almost never changes.
pub(crate) fn write_avatar_once() {
    let Some(icon_path) = account_icon_path() else {
        return;
    };
    let Ok(decoded) = image::open(&icon_path) else {
        return;
    };
    let square = decoded
        .resize_to_fill(
            AVATAR_DIAMETER,
            AVATAR_DIAMETER,
            image::imageops::FilterType::Triangle,
        )
        .into_rgb8();
    let Some(path) = cache_path("lock-avatar.rgb") else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if rmac_storage::create_dir_all_private(parent).is_err() {
        return;
    }
    if let Err(error) = rmac_storage::atomic_write_private(&path, square.as_raw()) {
        eprintln!("the lock screen's account picture could not be saved: {error}");
    }
}
