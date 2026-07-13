//! XDG autostart adapter for the supported Linux session.

use rmac_login_items::{BackgroundService, Error, ErrorKind, Issue, Item, Service, Snapshot};
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SystemService {
    environment: Environment,
}

impl Default for SystemService {
    fn default() -> Self {
        Self {
            environment: Environment::current(),
        }
    }
}

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let mut snapshot = discover(&self.environment)?;
        match systemd_background_services(&self.environment) {
            Ok((services, truncated)) => {
                snapshot.background_services = services;
                snapshot.background_services_truncated = truncated;
            }
            Err(error) => snapshot.background_services_error = Some(error.to_string()),
        }
        Ok(snapshot)
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error> {
        rmac_login_items::validate_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists")
            })?;
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this system entry cannot be enabled without a user-owned override",
            ));
        }
        let user_path = self.environment.config_home.join("autostart").join(id);
        if enabled && item.managed_override && item.source == user_path {
            std::fs::remove_file(&user_path).map_err(mutation_error)?;
            sync_parent(&user_path)?;
        } else {
            let source = std::fs::read_to_string(&item.source).map_err(mutation_error)?;
            let managed = !item.user_owned;
            let updated = rmac_login_items::with_hidden(&source, !enabled, managed)?;
            std::fs::create_dir_all(
                user_path
                    .parent()
                    .expect("an autostart entry always has a parent"),
            )
            .map_err(mutation_error)?;
            rmac_storage::atomic_write(&user_path, updated.as_bytes()).map_err(mutation_error)?;
        }
        self.snapshot()
    }

    fn set_background_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error> {
        rmac_login_items::validate_service_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .background_services
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "user service no longer exists"))?;
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this user service does not have a safe persistent transition",
            ));
        }
        systemd_set_enabled(id, enabled)?;
        let refreshed = self.snapshot()?;
        let changed = refreshed
            .background_services
            .iter()
            .find(|item| item.id == id)
            .is_some_and(|item| item.enabled == enabled);
        if changed {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mutation,
                "the user manager did not confirm the requested unit-file state",
            ))
        }
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService::default().snapshot()
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<Snapshot, Error> {
    SystemService::default().set_enabled(id, enabled)
}

pub fn set_background_enabled(id: &str, enabled: bool) -> Result<Snapshot, Error> {
    SystemService::default().set_background_enabled(id, enabled)
}

#[derive(Clone, Debug)]
struct Environment {
    config_home: PathBuf,
    config_dirs: Vec<PathBuf>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    data_home: PathBuf,
    desktops: Vec<String>,
}

impl Environment {
    fn current() -> Self {
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

    fn autostart_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        std::iter::once(self.config_home.join("autostart"))
            .chain(self.config_dirs.iter().map(|path| path.join("autostart")))
    }

    #[cfg(target_os = "linux")]
    fn user_unit_dirs(&self) -> [PathBuf; 2] {
        [
            self.config_home.join("systemd/user"),
            self.data_home.join("systemd/user"),
        ]
    }
}

#[cfg(target_os = "linux")]
fn systemd_background_services(
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
        .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
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
        if let Some(service) = rmac_login_items::background_service(id, &state, user_owned)? {
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
fn systemd_background_services(
    _environment: &Environment,
) -> Result<(Vec<BackgroundService>, bool), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "systemd user services are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn user_unit_names(directories: &[PathBuf]) -> HashSet<String> {
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
fn systemd_set_enabled(id: &str, enabled: bool) -> Result<(), Error> {
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
fn systemd_set_enabled(_id: &str, _enabled: bool) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "systemd user service changes are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn systemd_proxy(
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
fn systemd_mutation_error(error: zbus::Error) -> Error {
    Error::new(ErrorKind::Mutation, error.to_string())
}

fn discover(environment: &Environment) -> Result<Snapshot, Error> {
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
                    directory.display().to_string(),
                    format!("could not read directory: {error}"),
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
            if items.len() >= rmac_login_items::MAX_ITEMS {
                truncated = true;
                continue;
            }
            let contents = match std::fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(error) => {
                    push_issue(
                        &mut issues,
                        &mut truncated,
                        id.to_owned(),
                        format!("could not read entry: {error}"),
                    );
                    continue;
                }
            };
            match rmac_login_items::parse_entry(&contents) {
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
                            Some(program) if !try_exec_available => {
                                format!("TryExec is unavailable: {program}")
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

fn push_issue(issues: &mut Vec<Issue>, truncated: &mut bool, file: String, detail: String) {
    if issues.len() < rmac_login_items::MAX_ISSUES {
        issues.push(Issue { file, detail });
    } else {
        *truncated = true;
    }
}

fn mutation_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Mutation, error.to_string())
}

fn executable_exists(program: &str) -> bool {
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

fn sync_parent(path: &Path) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::Mutation, "entry has no parent directory"))?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(mutation_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn environment() -> (PathBuf, Environment) {
        let root = std::env::temp_dir().join(format!(
            "rmac-login-items-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let user = root.join("user");
        let system = root.join("system");
        std::fs::create_dir_all(system.join("autostart")).unwrap();
        (
            root.clone(),
            Environment {
                config_home: user,
                config_dirs: vec![system],
                data_home: root.join("data"),
                desktops: vec!["rmac".into()],
            },
        )
    }

    #[test]
    fn precedence_disable_and_managed_restore_follow_xdg_contract() {
        let (root, environment) = environment();
        let system = environment.config_dirs[0]
            .join("autostart")
            .join("demo.desktop");
        std::fs::write(
            &system,
            "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n",
        )
        .unwrap();
        let service = SystemService {
            environment: environment.clone(),
        };
        assert!(service.snapshot().unwrap().items[0].enabled);
        let disabled = service.set_enabled("demo.desktop", false).unwrap();
        assert!(!disabled.items[0].enabled);
        assert!(disabled.items[0].managed_override);
        let enabled = service.set_enabled("demo.desktop", true).unwrap();
        assert!(enabled.items[0].enabled);
        assert_eq!(enabled.items[0].source, system);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_higher_priority_entry_is_reported_and_hides_lower_entry() {
        let (root, environment) = environment();
        let user = environment.config_home.join("autostart");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("demo.desktop"), "not a desktop entry").unwrap();
        std::fs::write(
            environment.config_dirs[0].join("autostart/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=System Demo\nExec=demo\n",
        )
        .unwrap();
        let snapshot = discover(&environment).unwrap();
        assert!(snapshot.items.is_empty());
        assert_eq!(snapshot.issues.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
