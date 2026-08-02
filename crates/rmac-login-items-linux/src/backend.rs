//! XDG autostart and systemd discovery and persistence backend.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct Environment {
    pub(super) config_home: PathBuf,
    pub(super) config_dirs: Vec<PathBuf>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) data_home: PathBuf,
    pub(super) desktops: Vec<String>,
}

impl Environment {
    pub(super) fn current() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from("/.rmac-unavailable"));
        let config_dirs = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|value| !value.is_empty())
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("/.rmac-unavailable"));
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            config_home,
            config_dirs,
            data_home,
            desktops,
        }
    }

    pub(super) fn autostart_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        std::iter::once(self.config_home.join("autostart"))
            .chain(self.config_dirs.iter().map(|path| path.join("autostart")))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn user_unit_dirs(&self) -> [PathBuf; 2] {
        [
            self.config_home.join("systemd/user"),
            self.data_home.join("systemd/user"),
        ]
    }
}

pub(super) fn prepare_add(environment: &Environment, source: &Path) -> Result<AddPreview, Error> {
    if !source.is_absolute() {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "choose a local desktop-entry file",
        ));
    }
    let id = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "entry filename is not UTF-8"))?;
    rmac_login_items::validate_id(id)?;
    let source_contents = read_entry_bytes(source)?;
    let contents = std::str::from_utf8(&source_contents)
        .map_err(|_| Error::new(ErrorKind::InvalidEntry, "desktop entry is not UTF-8"))?;
    let parsed = rmac_login_items::parse_entry(contents)?;
    let target = environment.config_home.join("autostart").join(id);
    if source == target {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "this entry is already installed in the user autostart directory",
        ));
    }
    let target_contents = read_optional_entry_bytes(&target)?;
    Ok(AddPreview::new(
        source.to_path_buf(),
        id.into(),
        parsed.name,
        parsed.command,
        source_contents,
        target_contents,
    ))
}

pub(super) fn preserve_disabled_after_removal(
    service: &SystemService,
    preview: &RemovePreview,
    refreshed: Snapshot,
) -> Result<Snapshot, Error> {
    let Some(revealed) = refreshed
        .items
        .iter()
        .find(|candidate| candidate.id == preview.id)
        .cloned()
    else {
        return Ok(refreshed);
    };
    if !revealed.enabled {
        return Ok(refreshed);
    }
    if revealed.user_owned {
        return Err(Error::new(
            ErrorKind::Conflict,
            "a new user login item appeared while the prior item was being removed",
        ));
    }

    let source_contents = read_entry_bytes(&revealed.source)?;
    let confirmed = service.snapshot()?;
    let confirmed_item = confirmed
        .items
        .iter()
        .find(|candidate| candidate.id == preview.id)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Conflict,
                "the revealed system login item changed before it could be disabled",
            )
        })?;
    if confirmed_item != &revealed || read_entry_bytes(&revealed.source)? != source_contents {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the revealed system login item changed before it could be disabled",
        ));
    }

    let target = service
        .environment
        .config_home
        .join("autostart")
        .join(&preview.id);
    if read_optional_entry_bytes(&target)?.is_some() {
        return Err(Error::new(
            ErrorKind::Conflict,
            "a new user login item appeared while the prior item was being removed",
        ));
    }
    let source = String::from_utf8(source_contents)
        .map_err(|_| Error::new(ErrorKind::InvalidEntry, "desktop entry is not UTF-8"))?;
    let hidden = rmac_login_items::with_hidden(&source, true, true)?;
    std::fs::create_dir_all(target.parent().expect("autostart target has a parent"))
        .map_err(mutation_error)?;
    rmac_storage::atomic_write(&target, hidden.as_bytes()).map_err(mutation_error)?;

    let final_snapshot = service.snapshot()?;
    let exact = final_snapshot.items.iter().any(|candidate| {
        candidate.id == preview.id
            && !candidate.enabled
            && candidate.user_owned
            && candidate.managed_override
            && candidate.source == target
    }) && read_entry_bytes(&target)? == hidden.as_bytes();
    if exact {
        Ok(final_snapshot)
    } else {
        Err(Error::new(
            ErrorKind::Mismatch,
            "the autostart inventory did not confirm the protective disabled override",
        ))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_background_services(
    environment: &Environment,
) -> Result<(Vec<BackgroundService>, bool), Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the systemd user manager is unavailable on the session bus",
        )
    })?;
    let proxy = systemd_proxy(&connection)?;
    let files = proxy
        .call::<_, _, Vec<(String, String)>>("ListUnitFiles", &())
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not list systemd user services",
            )
        })?;
    let unit_paths = proxy
        .get_property::<Vec<String>>("UnitPath")
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect::<Vec<_>>();
    let user_dirs = environment.user_unit_dirs();
    let user_names = user_unit_names(&user_dirs);
    let mut services = HashMap::new();
    for (raw_name, state) in files {
        let path = Path::new(&raw_name);
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !id.ends_with(".service") {
            continue;
        }
        let user_owned = user_names.contains(id)
            || path.is_absolute()
                && user_dirs
                    .iter()
                    .any(|directory| path.starts_with(directory));
        let source = if path.is_absolute() && std::fs::symlink_metadata(path).is_ok() {
            Some(path.to_path_buf())
        } else {
            unit_paths
                .iter()
                .map(|directory| directory.join(id))
                .find(|candidate| std::fs::symlink_metadata(candidate).is_ok())
        };
        if let Some(service) = rmac_login_items::background_service(id, &state, user_owned, source)?
        {
            services.insert(id.to_owned(), service);
        }
    }
    let mut services = services.into_values().collect::<Vec<_>>();
    services.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    let truncated = services.len() > rmac_login_items::MAX_BACKGROUND_SERVICES;
    services.truncate(rmac_login_items::MAX_BACKGROUND_SERVICES);
    Ok((services, truncated))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn systemd_background_services(
    _environment: &Environment,
) -> Result<(Vec<BackgroundService>, bool), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "systemd user services are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn user_unit_names(directories: &[PathBuf]) -> HashSet<String> {
    directories
        .iter()
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name.ends_with(".service"))
        .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_set_enabled(id: &str, enabled: bool) -> Result<(), Error> {
    let connection = zbus::blocking::Connection::session().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the systemd user manager is unavailable",
        )
    })?;
    let proxy = systemd_proxy(&connection)?;
    let files = vec![id];
    if enabled {
        let (carries_install_info, _changes): (bool, Vec<(String, String, String)>) = proxy
            .call("EnableUnitFiles", &(files, false, false))
            .map_err(systemd_mutation_error)?;
        if !carries_install_info {
            return Err(Error::new(
                ErrorKind::Mutation,
                "the unit has no [Install] information and cannot be enabled",
            ));
        }
    } else {
        let _: Vec<(String, String, String)> = proxy
            .call("DisableUnitFiles", &(files, false))
            .map_err(systemd_mutation_error)?;
    }
    proxy
        .call::<_, _, ()>("Reload", &())
        .map_err(systemd_mutation_error)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn systemd_set_enabled(_id: &str, _enabled: bool) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "systemd user service changes are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the systemd user manager is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_mutation_error(_error: zbus::Error) -> Error {
    Error::new(
        ErrorKind::Mutation,
        "the systemd user manager could not apply the requested change",
    )
}

pub(super) fn discover(environment: &Environment) -> Result<Snapshot, Error> {
    let user_dir = environment.config_home.join("autostart");
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let mut issues = Vec::new();
    let mut truncated = false;
    for directory in environment.autostart_dirs() {
        let mut entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries.flatten().collect::<Vec<_>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                push_issue(
                    &mut issues,
                    &mut truncated,
                    "Autostart directory".into(),
                    directory_read_issue(error.kind()).into(),
                );
                continue;
            }
        };
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.extension().and_then(|extension| extension.to_str()) != Some("desktop")
                || !seen.insert(id.to_owned())
            {
                continue;
            }
            if rmac_login_items::validate_id(id).is_err() {
                push_issue(
                    &mut issues,
                    &mut truncated,
                    "Invalid desktop entry".into(),
                    "desktop entry has an invalid filename".into(),
                );
                continue;
            }
            if items.len() >= rmac_login_items::MAX_ITEMS {
                truncated = true;
                continue;
            }
            let bytes = match read_entry_bytes(&path) {
                Ok(contents) => contents,
                Err(error) => {
                    push_issue(
                        &mut issues,
                        &mut truncated,
                        id.to_owned(),
                        error.to_string(),
                    );
                    continue;
                }
            };
            let contents = match std::str::from_utf8(&bytes) {
                Ok(contents) => contents,
                Err(_) => {
                    push_issue(
                        &mut issues,
                        &mut truncated,
                        id.to_owned(),
                        "desktop entry is not UTF-8".into(),
                    );
                    continue;
                }
            };
            match rmac_login_items::parse_entry(contents) {
                Ok(parsed) => {
                    let try_exec_available =
                        parsed.try_exec.as_deref().is_none_or(executable_exists);
                    let applies =
                        rmac_login_items::applies_to_session(&parsed, &environment.desktops)
                            && try_exec_available;
                    let user_owned = path.starts_with(&user_dir);
                    items.push(Item {
                        id: id.to_owned(),
                        name: parsed.name,
                        source: path,
                        enabled: !parsed.hidden,
                        applies_to_session: applies,
                        session_detail: (!applies).then(|| match parsed.try_exec.as_deref() {
                            Some(_) if !try_exec_available => {
                                "Required TryExec program is unavailable".into()
                            }
                            _ => "Excluded by OnlyShowIn/NotShowIn for this desktop".into(),
                        }),
                        user_owned,
                        managed_override: parsed.managed_override,
                        can_toggle: !parsed.hidden || user_owned,
                    });
                }
                Err(error) => push_issue(
                    &mut issues,
                    &mut truncated,
                    id.to_owned(),
                    error.to_string(),
                ),
            }
        }
    }
    items.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(Snapshot {
        items,
        background_services: Vec::new(),
        background_services_truncated: false,
        background_services_error: None,
        issues,
        truncated,
    })
}

pub(super) fn push_issue(
    issues: &mut Vec<Issue>,
    truncated: &mut bool,
    file: String,
    detail: String,
) {
    if issues.len() < rmac_login_items::MAX_ISSUES {
        issues.push(Issue { file, detail });
    } else {
        *truncated = true;
    }
}

pub(super) fn mutation_error(error: std::io::Error) -> Error {
    let detail = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            "permission was denied while changing the login item"
        }
        std::io::ErrorKind::NotFound => "the login item changed before the operation completed",
        std::io::ErrorKind::AlreadyExists => "the login item destination changed unexpectedly",
        _ => "the login item change could not be saved",
    };
    Error::new(ErrorKind::Mutation, detail)
}

pub(super) fn directory_read_issue(kind: std::io::ErrorKind) -> &'static str {
    if kind == std::io::ErrorKind::PermissionDenied {
        "permission was denied while reading an autostart directory"
    } else {
        "could not read an autostart directory"
    }
}

pub(super) fn read_optional_entry_bytes(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => read_entry_bytes(path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(entry_read_error(error.kind())),
    }
}

pub(super) fn read_entry_bytes(path: &Path) -> Result<Vec<u8>, Error> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| entry_read_error(error.kind()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(Error::new(
            ErrorKind::InvalidEntry,
            "login items must be regular files, not links or directories",
        ));
    }
    if metadata.len() > rmac_login_items::MAX_ENTRY_BYTES as u64 {
        return Err(Error::new(ErrorKind::InvalidEntry, "entry is too large"));
    }

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| entry_read_error(error.kind()))?;
    let opened = file
        .metadata()
        .map_err(|error| entry_read_error(error.kind()))?;
    if !opened.file_type().is_file() || opened.len() > rmac_login_items::MAX_ENTRY_BYTES as u64 {
        return Err(Error::new(ErrorKind::InvalidEntry, "entry is too large"));
    }
    let mut contents = Vec::with_capacity(opened.len() as usize);
    file.take(rmac_login_items::MAX_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut contents)
        .map_err(|error| entry_read_error(error.kind()))?;
    if contents.len() > rmac_login_items::MAX_ENTRY_BYTES {
        return Err(Error::new(ErrorKind::InvalidEntry, "entry is too large"));
    }
    Ok(contents)
}

pub(super) fn entry_read_error(kind: std::io::ErrorKind) -> Error {
    let (error_kind, detail) = match kind {
        std::io::ErrorKind::NotFound => (
            ErrorKind::Conflict,
            "the login item changed before it could be read",
        ),
        std::io::ErrorKind::PermissionDenied => (
            ErrorKind::Unavailable,
            "permission was denied while reading the login item",
        ),
        _ => (ErrorKind::Unavailable, "the login item could not be read"),
    };
    Error::new(error_kind, detail)
}

pub(super) fn executable_exists(program: &str) -> bool {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return candidate.is_file();
    }
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|directory| directory.join(candidate).is_file())
        })
        .unwrap_or(false)
}

pub(super) fn sync_parent(path: &Path) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::Mutation, "entry has no parent directory"))?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(mutation_error)
}
