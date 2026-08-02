//! PackageKit normalization and bounded-text helpers.

use super::*;

pub(super) fn finish_exit(
    exit: u32,
    backend_error: Option<Error>,
    operation: &str,
) -> Result<(), Error> {
    match exit {
        1 => backend_error.map_or(Ok(()), Err),
        3 | 9 => Err(Error::new(
            ErrorKind::Cancelled,
            format!("the {operation} was cancelled"),
        )),
        4 => Err(Error::new(
            ErrorKind::Trust,
            "a repository signing key must be reviewed outside rmac",
        )),
        5 => Err(Error::new(
            ErrorKind::Interaction,
            "a package licence must be reviewed outside rmac",
        )),
        7 => Err(Error::new(
            ErrorKind::Interaction,
            "the package backend requires installation media",
        )),
        8 => Err(Error::new(
            ErrorKind::Trust,
            "the transaction requires an untrusted package",
        )),
        _ => Err(backend_error.unwrap_or_else(|| {
            Error::new(
                ErrorKind::Backend,
                format!("the update service finished with status {exit}"),
            )
        })),
    }
}

pub(super) fn packagekit_error(code: u32, _detail: &str) -> Error {
    match code {
        17 | 65 => Error::new(ErrorKind::Cancelled, "the update transaction was cancelled"),
        3 => Error::new(
            ErrorKind::Unavailable,
            "the PackageKit backend does not support this update operation",
        ),
        2 => Error::new(
            ErrorKind::Backend,
            "a network connection is required to download the updates",
        ),
        10 | 37 | 43 | 64 => Error::new(
            ErrorKind::Backend,
            "PackageKit could not download update data from the configured repositories",
        ),
        13 => Error::new(
            ErrorKind::Backend,
            "the update dependencies could not be resolved",
        ),
        26 | 67 => Error::new(
            ErrorKind::Backend,
            "another package transaction currently holds the package-manager lock",
        ),
        5 | 30 | 31 | 50 | 51 => Error::new(
            ErrorKind::Trust,
            "PackageKit refused an untrusted or invalidly signed package",
        ),
        34 | 47 => Error::new(
            ErrorKind::Interaction,
            "PackageKit requires a licence or installation media that rmac cannot accept silently",
        ),
        46 => Error::new(
            ErrorKind::Backend,
            "there is not enough disk space to install the updates",
        ),
        48 => Error::new(
            ErrorKind::Authorization,
            "update authorization was denied or cancelled",
        ),
        27 | 41 | 49 => Error::new(ErrorKind::Stale, "no installable updates remain"),
        61 => Error::new(
            ErrorKind::Stale,
            "the package database changed during the update transaction",
        ),
        35 | 36 | 39 | 60 => Error::new(
            ErrorKind::Backend,
            "a package conflict prevents the update plan from being installed",
        ),
        38 | 40 | 56..=59 | 66 => Error::new(
            ErrorKind::Backend,
            "the package backend could not complete the update transaction safely",
        ),
        _ => Error::new(
            ErrorKind::Backend,
            format!("the package backend reported error {code}"),
        ),
    }
}

pub(super) fn package_identity(package_id: &str) -> Option<(String, String)> {
    if package_id.is_empty()
        || package_id.len() > MAX_PACKAGE_ID_BYTES
        || package_id.chars().any(char::is_control)
    {
        return None;
    }
    let mut fields = package_id.split(';');
    let name = fields.next()?;
    let version = fields.next()?;
    let _architecture = fields.next()?;
    let _repository = fields.next()?;
    if fields.next().is_some() || name.is_empty() || version.is_empty() {
        return None;
    }
    let name = bounded_text(name);
    let version = bounded_text(version);
    (!name.is_empty() && !version.is_empty()).then_some((name, version))
}

pub(super) fn change_rank(kind: ChangeKind) -> u8 {
    match kind {
        ChangeKind::Update => 0,
        ChangeKind::Install => 1,
        ChangeKind::Reinstall => 2,
        ChangeKind::Remove => 3,
        ChangeKind::Obsolete => 4,
        ChangeKind::Downgrade => 5,
    }
}

pub(super) fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_TEXT_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}
