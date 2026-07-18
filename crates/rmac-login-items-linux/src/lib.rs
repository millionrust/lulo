//! XDG autostart adapter for the supported Linux session.

use rmac_login_items::{
    AddPreview, BackgroundService, Error, ErrorKind, Issue, Item, RemovePreview, Service, Snapshot,
};
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

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
            .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists"))?
            .clone();
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this system entry cannot be enabled without a user-owned override",
            ));
        }
        let source_before = read_entry_bytes(&item.source)?;
        let confirmed = self.snapshot()?;
        let confirmed_item = confirmed
            .items
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::Conflict, "autostart entry changed before save")
            })?;
        if confirmed_item != &item || read_entry_bytes(&item.source)? != source_before {
            return Err(Error::new(
                ErrorKind::Conflict,
                "autostart entry changed before save; refresh and try again",
            ));
        }
        let user_path = self.environment.config_home.join("autostart").join(id);
        let expected_contents = if enabled && item.managed_override && item.source == user_path {
            std::fs::remove_file(&user_path).map_err(mutation_error)?;
            sync_parent(&user_path)?;
            None
        } else {
            let source = String::from_utf8(source_before)
                .map_err(|_| Error::new(ErrorKind::InvalidEntry, "autostart entry is not UTF-8"))?;
            let managed = !item.user_owned;
            let updated = rmac_login_items::with_hidden(&source, !enabled, managed)?;
            std::fs::create_dir_all(
                user_path
                    .parent()
                    .expect("an autostart entry always has a parent"),
            )
            .map_err(mutation_error)?;
            rmac_storage::atomic_write(&user_path, updated.as_bytes()).map_err(mutation_error)?;
            Some(updated.into_bytes())
        };
        let refreshed = self.snapshot()?;
        let confirmed_state = refreshed
            .items
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| {
                candidate.enabled == enabled
                    && if expected_contents.is_some() {
                        candidate.source == user_path
                            && candidate.user_owned
                            && candidate.managed_override != item.user_owned
                    } else {
                        candidate.source != user_path && !candidate.managed_override
                    }
            });
        let exact_file = match expected_contents {
            Some(expected) => read_entry_bytes(&user_path)? == expected,
            None => read_optional_entry_bytes(&user_path)?.is_none(),
        };
        if confirmed_state && exact_file {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the autostart inventory did not confirm the requested state",
            ))
        }
    }

    fn set_background_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error> {
        rmac_login_items::validate_service_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .background_services
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "user service no longer exists"))?
            .clone();
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this user service does not have a safe persistent transition",
            ));
        }
        let confirmed = self.snapshot()?;
        if confirmed
            .background_services
            .iter()
            .find(|candidate| candidate.id == id)
            != Some(&item)
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the user service changed before save; refresh and try again",
            ));
        }
        systemd_set_enabled(id, enabled)?;
        let refreshed = self.snapshot()?;
        let changed = refreshed
            .background_services
            .iter()
            .find(|item| item.id == id)
            .is_some_and(|candidate| {
                candidate.enabled == enabled
                    && candidate.user_owned
                    && candidate.source == item.source
            });
        if changed {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the user manager did not confirm the requested unit-file state",
            ))
        }
    }

    fn prepare_add(&self, source: &Path) -> Result<AddPreview, Error> {
        prepare_add(&self.environment, source)
    }

    fn add(&self, preview: &AddPreview) -> Result<Snapshot, Error> {
        let current_source = read_entry_bytes(&preview.source)?;
        if current_source != preview.source_contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the selected desktop entry changed after review",
            ));
        }
        let contents = String::from_utf8(current_source)
            .map_err(|_| Error::new(ErrorKind::InvalidEntry, "desktop entry is not UTF-8"))?;
        let contents = rmac_login_items::with_hidden(&contents, false, false)?;
        let target = self
            .environment
            .config_home
            .join("autostart")
            .join(&preview.id);
        let current_target = read_optional_entry_bytes(&target)?;
        if current_target.as_deref() != preview.target_contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the destination login item changed after review",
            ));
        }
        std::fs::create_dir_all(target.parent().expect("autostart target has a parent"))
            .map_err(mutation_error)?;
        rmac_storage::atomic_write(&target, contents.as_bytes()).map_err(mutation_error)?;
        let refreshed = self.snapshot()?;
        let exact_inventory = refreshed.items.iter().any(|item| {
            item.id == preview.id
                && item.enabled
                && item.source == target
                && item.name == preview.name
        });
        let exact_file = read_entry_bytes(&target)? == contents.as_bytes();
        if exact_inventory && exact_file {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the autostart inventory did not confirm the installed login item",
            ))
        }
    }

    fn prepare_remove(&self, id: &str) -> Result<RemovePreview, Error> {
        rmac_login_items::validate_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists")
            })?;
        if !item.user_owned || item.managed_override {
            return Err(Error::new(
                ErrorKind::Mutation,
                "only user-owned application entries can be moved to Trash",
            ));
        }
        let contents = read_entry_bytes(&item.source)?;
        Ok(RemovePreview::new(
            item.id.clone(),
            item.name.clone(),
            item.source.clone(),
            contents,
        ))
    }

    fn remove(&self, preview: &RemovePreview) -> Result<Snapshot, Error> {
        let current = self.prepare_remove(&preview.id)?;
        if current != *preview || read_entry_bytes(&preview.source)? != preview.contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the login item changed after removal was confirmed",
            ));
        }
        trash::delete(&preview.source).map_err(|_| {
            Error::new(
                ErrorKind::Mutation,
                "the login item could not be moved to Trash",
            )
        })?;
        let refreshed = self.snapshot()?;
        preserve_disabled_after_removal(self, preview, refreshed)
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

pub fn prepare_add_source(source: &Path) -> Result<AddPreview, Error> {
    SystemService::default().prepare_add(source)
}

pub fn add_source(preview: &AddPreview) -> Result<Snapshot, Error> {
    SystemService::default().add(preview)
}

pub fn prepare_remove_autostart(id: &str) -> Result<RemovePreview, Error> {
    SystemService::default().prepare_remove(id)
}

pub fn remove_autostart(preview: &RemovePreview) -> Result<Snapshot, Error> {
    SystemService::default().remove(preview)
}

pub fn autostart_source(id: &str) -> Result<PathBuf, Error> {
    rmac_login_items::validate_id(id)?;
    SystemService::default()
        .snapshot()?
        .items
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.source)
        .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists"))
}

pub fn background_service_source(id: &str) -> Result<PathBuf, Error> {
    rmac_login_items::validate_service_id(id)?;
    SystemService::default()
        .snapshot()?
        .background_services
        .into_iter()
        .find(|item| item.id == id)
        .and_then(|item| item.source)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "the user service file is unavailable",
            )
        })
}

#[cfg(target_os = "linux")]
pub async fn watch(
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    let environment = Environment::current();
    let _filesystem = match filesystem_watcher(&environment, sender.clone()) {
        Ok(watcher) => Some(watcher),
        Err(_) => {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            None
        }
    };
    loop {
        match watch_systemd_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    sender
        .send(rmac_login_items::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "login item watcher closed"))
}

#[cfg(target_os = "linux")]
fn filesystem_watcher(
    environment: &Environment,
    sender: async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<notify::RecommendedWatcher, Error> {
    use notify::{RecursiveMode, Watcher as _};

    let roots = filesystem_event_roots(environment);
    let filter_roots = roots.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Unavailable);
            return;
        };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| filter_roots.iter().any(|root| path.starts_with(root)))
        {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Changed);
        }
    })
    .map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the login item filesystem watcher is unavailable",
        )
    })?;
    let mut watched = 0;
    for root in roots {
        let target = if root.is_dir() {
            root
        } else if let Some(parent) = root.parent().filter(|parent| parent.is_dir()) {
            parent.to_path_buf()
        } else {
            continue;
        };
        if watcher.watch(&target, RecursiveMode::Recursive).is_ok() {
            watched += 1;
        }
    }
    if watched == 0 {
        Err(Error::new(
            ErrorKind::Unavailable,
            "no login item directories could be watched",
        ))
    } else {
        Ok(watcher)
    }
}

#[cfg(target_os = "linux")]
fn filesystem_event_roots(environment: &Environment) -> Vec<PathBuf> {
    let mut roots = vec![
        environment.config_home.join("autostart"),
        environment.config_home.join("systemd/user"),
        environment.data_home.join("systemd/user"),
    ];
    roots.extend(
        environment
            .config_dirs
            .iter()
            .map(|directory| directory.join("autostart")),
    );
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(target_os = "linux")]
async fn watch_systemd_once(
    sender: &async_channel::Sender<rmac_login_items::WatchEvent>,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::session().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "systemd user event stream is unavailable",
        )
    })?;
    let unit_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd signal sender"))?
        .path("/org/freedesktop/systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd manager path"))?
        .interface("org.freedesktop.systemd1.Manager")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd manager interface"))?
        .member("UnitFilesChanged")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd unit signal"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid owner signal"))?
        .add_arg("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid systemd owner filter"))?
        .build();
    let mut units = MessageStream::for_match_rule(unit_rule, &connection, Some(8))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch user unit files"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| {
            Error::new(
                ErrorKind::Unavailable,
                "could not watch user manager restarts",
            )
        })?
        .fuse();
    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = units.next() => {
                message
                    .ok_or_else(|| Error::new(ErrorKind::Unavailable, "user unit stream ended"))?
                    .map_err(|_| Error::new(ErrorKind::Unavailable, "user unit stream failed"))?;
                true
            },
            message = owners.next() => systemd_owner_reappeared(message)?,
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_login_items::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
fn systemd_owner_reappeared(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "user manager owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "user manager owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Unavailable, "invalid user manager owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.systemd1" && !new_owner.is_empty()
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

fn prepare_add(environment: &Environment, source: &Path) -> Result<AddPreview, Error> {
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

fn preserve_disabled_after_removal(
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
fn systemd_mutation_error(_error: zbus::Error) -> Error {
    Error::new(
        ErrorKind::Mutation,
        "the systemd user manager could not apply the requested change",
    )
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

fn push_issue(issues: &mut Vec<Issue>, truncated: &mut bool, file: String, detail: String) {
    if issues.len() < rmac_login_items::MAX_ISSUES {
        issues.push(Issue { file, detail });
    } else {
        *truncated = true;
    }
}

fn mutation_error(error: std::io::Error) -> Error {
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

fn directory_read_issue(kind: std::io::ErrorKind) -> &'static str {
    if kind == std::io::ErrorKind::PermissionDenied {
        "permission was denied while reading an autostart directory"
    } else {
        "could not read an autostart directory"
    }
}

fn read_optional_entry_bytes(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => read_entry_bytes(path).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(entry_read_error(error.kind())),
    }
}

fn read_entry_bytes(path: &Path) -> Result<Vec<u8>, Error> {
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

fn entry_read_error(kind: std::io::ErrorKind) -> Error {
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

    #[test]
    fn user_manager_idle_exit_is_ignored_but_reappearance_refreshes() {
        assert!(!owner_change_reappeared("org.freedesktop.systemd1", ""));
        assert!(owner_change_reappeared("org.freedesktop.systemd1", ":1.42"));
        assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
    }

    #[test]
    fn add_requires_explicit_replacement_and_installs_enabled_entry() {
        let (root, environment) = environment();
        let source = root.join("demo.desktop");
        let target = environment.config_home.join("autostart/demo.desktop");
        std::fs::write(
            &source,
            "[Desktop Entry]\nType=Application\nName=Demo\nHidden=true\nExec=demo\n",
        )
        .unwrap();
        let service = SystemService { environment };
        let preview = service.prepare_add(&source).unwrap();
        assert!(!preview.replacing);
        assert_eq!(preview.command, "demo");
        let snapshot = service.add(&preview).unwrap();
        assert!(snapshot.items[0].enabled);
        assert!(service.add(&preview).is_err());
        let replacement = service.prepare_add(&source).unwrap();
        assert!(replacement.replacing);
        std::fs::write(
            &target,
            "[Desktop Entry]\nType=Application\nName=External\nExec=external\n",
        )
        .unwrap();
        assert_eq!(
            service.add(&replacement).unwrap_err().kind(),
            ErrorKind::Conflict
        );
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .contains("Exec=external"));
        let replacement = service.prepare_add(&source).unwrap();
        assert!(service.add(&replacement).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_rejects_a_source_changed_after_confirmation() {
        let (root, environment) = environment();
        let source = root.join("demo.desktop");
        std::fs::write(
            &source,
            "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n",
        )
        .unwrap();
        let service = SystemService { environment };
        let preview = service.prepare_add(&source).unwrap();
        std::fs::write(
            &source,
            "[Desktop Entry]\nType=Application\nName=Changed\nExec=other\n",
        )
        .unwrap();
        assert_eq!(
            service.add(&preview).unwrap_err().kind(),
            ErrorKind::Conflict
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_reader_rejects_oversized_and_linked_entries() {
        let (root, environment) = environment();
        let oversized = environment.config_dirs[0].join("autostart/oversized.desktop");
        std::fs::write(
            &oversized,
            vec![b'x'; rmac_login_items::MAX_ENTRY_BYTES + 1],
        )
        .unwrap();
        let snapshot = discover(&environment).unwrap();
        assert!(snapshot.items.is_empty());
        assert_eq!(snapshot.issues[0].detail, "entry is too large");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let real = root.join("real.desktop");
            std::fs::write(
                &real,
                "[Desktop Entry]\nType=Application\nName=Real\nExec=real\n",
            )
            .unwrap();
            let linked = root.join("linked.desktop");
            symlink(&real, &linked).unwrap();
            assert_eq!(
                prepare_add(&environment, &linked).unwrap_err().kind(),
                ErrorKind::InvalidEntry
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_keeps_a_revealed_system_entry_disabled() {
        let (root, environment) = environment();
        let system = environment.config_dirs[0]
            .join("autostart")
            .join("demo.desktop");
        std::fs::write(
            &system,
            "[Desktop Entry]\nType=Application\nName=System Demo\nExec=system-demo\n",
        )
        .unwrap();
        let user = environment
            .config_home
            .join("autostart")
            .join("demo.desktop");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        let contents =
            "[Desktop Entry]\nType=Application\nName=User Demo\nHidden=true\nExec=user-demo\n";
        std::fs::write(&user, contents).unwrap();
        let service = SystemService { environment };
        let preview = service.prepare_remove("demo.desktop").unwrap();
        std::fs::remove_file(&user).unwrap();
        let revealed = service.snapshot().unwrap();
        assert!(revealed.items[0].enabled);
        let protected = preserve_disabled_after_removal(&service, &preview, revealed).unwrap();
        assert!(!protected.items[0].enabled);
        assert!(protected.items[0].managed_override);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_recovery_never_overwrites_a_concurrent_user_entry() {
        let (root, environment) = environment();
        let system = environment.config_dirs[0]
            .join("autostart")
            .join("demo.desktop");
        std::fs::write(
            &system,
            "[Desktop Entry]\nType=Application\nName=System Demo\nExec=system-demo\n",
        )
        .unwrap();
        let user = environment
            .config_home
            .join("autostart")
            .join("demo.desktop");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        std::fs::write(
            &user,
            "[Desktop Entry]\nType=Application\nName=Old User\nHidden=true\nExec=old\n",
        )
        .unwrap();
        let service = SystemService { environment };
        let preview = service.prepare_remove("demo.desktop").unwrap();
        std::fs::remove_file(&user).unwrap();
        let revealed = service.snapshot().unwrap();
        let replacement = "[Desktop Entry]\nType=Application\nName=New User\nExec=new\n";
        std::fs::write(&user, replacement).unwrap();
        assert_eq!(
            preserve_disabled_after_removal(&service, &preview, revealed)
                .unwrap_err()
                .kind(),
            ErrorKind::Conflict
        );
        assert_eq!(std::fs::read_to_string(&user).unwrap(), replacement);
        std::fs::remove_dir_all(root).unwrap();
    }
}
