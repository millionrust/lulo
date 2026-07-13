//! XDG autostart adapter for the supported Linux session.

use rmac_login_items::{Error, ErrorKind, Issue, Item, Service, Snapshot};
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
        discover(&self.environment)
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
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService::default().snapshot()
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<Snapshot, Error> {
    SystemService::default().set_enabled(id, enabled)
}

#[derive(Clone, Debug)]
struct Environment {
    config_home: PathBuf,
    config_dirs: Vec<PathBuf>,
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
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            config_home,
            config_dirs,
            desktops,
        }
    }

    fn autostart_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        std::iter::once(self.config_home.join("autostart"))
            .chain(self.config_dirs.iter().map(|path| path.join("autostart")))
    }
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
            root,
            Environment {
                config_home: user,
                config_dirs: vec![system],
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
