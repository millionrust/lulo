//! Linux sharing adapter backed by systemd and read-only firewall inspection.

use rmac_sharing::{Error, ErrorKind, Service, Snapshot};
#[cfg(target_os = "linux")]
use rmac_sharing::{FileSharing, RemoteLogin};
#[cfg(any(target_os = "linux", test))]
use rmac_sharing::{FirewallState, Share};

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedService {
    RemoteLogin,
    FileSharing,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_remote_login(&self, enabled: bool) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let unit =
            current.remote_login.unit.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Unavailable, "OpenSSH server is not installed")
            })?;
        if current.remote_login.active == enabled && current.remote_login.enabled_at_boot == enabled
        {
            return Ok(current);
        }
        if let Err(error) = system_set_service(unit, enabled, "SSH") {
            let _ = restore_service(
                unit,
                current.remote_login.active,
                current.remote_login.enabled_at_boot,
            );
            return Err(error);
        }
        match wait_for_state(ManagedService::RemoteLogin, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                let _ = restore_service(
                    unit,
                    current.remote_login.active,
                    current.remote_login.enabled_at_boot,
                );
                Err(error)
            }
        }
    }

    fn set_file_sharing(&self, enabled: bool) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let unit = current.file_sharing.unit.as_deref().ok_or_else(|| {
            Error::new(ErrorKind::Unavailable, "Samba file server is not installed")
        })?;
        if current.file_sharing.active == enabled && current.file_sharing.enabled_at_boot == enabled
        {
            return Ok(current);
        }
        if let Err(error) = system_set_service(unit, enabled, "SMB") {
            let _ = restore_service(
                unit,
                current.file_sharing.active,
                current.file_sharing.enabled_at_boot,
            );
            return Err(error);
        }
        match wait_for_state(ManagedService::FileSharing, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                let _ = restore_service(
                    unit,
                    current.file_sharing.active,
                    current.file_sharing.enabled_at_boot,
                );
                Err(error)
            }
        }
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_remote_login(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_remote_login(enabled)
}

pub fn set_file_sharing(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_file_sharing(enabled)
}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_sharing::WatchEvent>) -> Result<(), Error> {
    let _configuration = match configuration_watcher(sender.clone()) {
        Ok(watcher) => watcher,
        Err(_) => {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            None
        }
    };
    loop {
        match watch_systemd_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_sharing::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_sharing::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "Sharing watcher closed"))
}

#[cfg(target_os = "linux")]
fn configuration_watcher(
    sender: async_channel::Sender<rmac_sharing::WatchEvent>,
) -> Result<Option<notify::RecommendedWatcher>, Error> {
    use notify::{RecursiveMode, Watcher as _};

    let roots = ["/etc/ufw", "/etc/samba"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .filter(|root| root.is_dir())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(None);
    }
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Unavailable);
            return;
        };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty()
            || event
                .paths
                .iter()
                .any(|path| firewall_path_relevant(path) || samba_path_relevant(path))
        {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Changed);
        }
    })
    .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
    for root in roots {
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
    }
    Ok(Some(watcher))
}

#[cfg(any(target_os = "linux", test))]
fn firewall_path_relevant(path: &std::path::Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("ufw.conf" | "user.rules" | "user6.rules")
    ) || path.ancestors().any(|ancestor| {
        ancestor.file_name().and_then(|name| name.to_str()) == Some("applications.d")
    })
}

#[cfg(any(target_os = "linux", test))]
fn samba_path_relevant(path: &std::path::Path) -> bool {
    path.ancestors()
        .any(|ancestor| ancestor == std::path::Path::new("/etc/samba"))
        && path.extension().and_then(|extension| extension.to_str()) == Some("conf")
}

#[cfg(target_os = "linux")]
async fn watch_systemd_once(
    sender: &async_channel::Sender<rmac_sharing::WatchEvent>,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system().await.map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "Sharing system event stream is unavailable",
        )
    })?;
    let properties_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd sender"))?
        .interface("org.freedesktop.DBus.Properties")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties interface"))?
        .member("PropertiesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid properties signal"))?
        .add_arg("org.freedesktop.systemd1.Unit")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid unit property filter"))?
        .build();
    let files_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd sender"))?
        .path("/org/freedesktop/systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd path"))?
        .interface("org.freedesktop.systemd1.Manager")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid manager interface"))?
        .member("UnitFilesChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid unit-files signal"))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus sender"))?
        .interface("org.freedesktop.DBus")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid D-Bus interface"))?
        .member("NameOwnerChanged")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid owner signal"))?
        .add_arg("org.freedesktop.systemd1")
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd owner filter"))?
        .build();
    let mut properties = MessageStream::for_match_rule(properties_rule, &connection, Some(16))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch system units"))?
        .fuse();
    let mut files = MessageStream::for_match_rule(files_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch unit files"))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not watch systemd restarts"))?
        .fuse();
    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let changed = futures_util::select! {
            message = properties.next() => stream_changed(message, "system unit")?,
            message = files.next() => stream_changed(message, "unit file")?,
            message = owners.next() => systemd_owner_reappeared(message)?,
            _ = closed => return Ok(()),
        };
        if changed {
            let _ = sender.try_send(rmac_sharing::WatchEvent::Changed);
        }
    }
}

#[cfg(target_os = "linux")]
fn stream_changed(
    message: Option<Result<zbus::Message, zbus::Error>>,
    name: &str,
) -> Result<bool, Error> {
    message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, format!("{name} stream ended")))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, format!("{name} stream failed")))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn systemd_owner_reappeared(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<bool, Error> {
    let message = message
        .ok_or_else(|| Error::new(ErrorKind::Unavailable, "systemd owner stream ended"))?
        .map_err(|_| Error::new(ErrorKind::Unavailable, "systemd owner stream failed"))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|_| Error::new(ErrorKind::Protocol, "invalid systemd owner change"))?;
    Ok(owner_change_reappeared(&name, &new_owner))
}

#[cfg(any(target_os = "linux", test))]
fn owner_change_reappeared(name: &str, new_owner: &str) -> bool {
    name == "org.freedesktop.systemd1" && !new_owner.is_empty()
}

#[cfg(target_os = "linux")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    let files = manager
        .call::<_, _, Vec<(String, String)>>("ListUnitFiles", &())
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let remote_service = service_state(
        &connection,
        &manager,
        &files,
        &["ssh.service", "sshd.service"],
    )?;
    let file_service = service_state(&connection, &manager, &files, &["smbd.service"])?;
    let (remote_firewall, remote_firewall_detail) = firewall_state(FirewallService::Ssh);
    let (file_firewall, file_firewall_detail) = firewall_state(FirewallService::Samba);
    let (shares, shares_truncated, configuration_error) = samba_shares();
    let file_sharing = match file_service {
        Some(service) => FileSharing {
            available: true,
            unit: Some(service.unit),
            active: service.active_state == "active",
            service_state: Some(service.active_state),
            enabled_at_boot: unit_enabled(&service.unit_file_state),
            unit_file_state: Some(service.unit_file_state),
            shares,
            shares_truncated,
            configuration_error,
            firewall: file_firewall,
            firewall_detail: file_firewall_detail,
        },
        None => FileSharing {
            shares,
            shares_truncated,
            configuration_error,
            firewall: file_firewall,
            firewall_detail: file_firewall_detail,
            ..FileSharing::default()
        },
    };
    Ok(Snapshot {
        remote_login: remote_service
            .map(|service| RemoteLogin {
                available: true,
                unit: Some(service.unit),
                active: service.active_state == "active",
                service_state: Some(service.active_state),
                enabled_at_boot: unit_enabled(&service.unit_file_state),
                unit_file_state: Some(service.unit_file_state),
                firewall: remote_firewall,
                firewall_detail: remote_firewall_detail.clone(),
            })
            .unwrap_or(RemoteLogin {
                firewall: remote_firewall,
                firewall_detail: remote_firewall_detail,
                ..RemoteLogin::default()
            }),
        file_sharing,
    })
}

#[cfg(target_os = "linux")]
struct SystemdServiceState {
    unit: String,
    unit_file_state: String,
    active_state: String,
}

#[cfg(target_os = "linux")]
fn service_state(
    connection: &zbus::blocking::Connection,
    manager: &zbus::blocking::Proxy<'_>,
    files: &[(String, String)],
    candidates: &[&str],
) -> Result<Option<SystemdServiceState>, Error> {
    let selected = candidates.iter().find_map(|candidate| {
        files.iter().find_map(|(name, state)| {
            let id = std::path::Path::new(name)
                .file_name()
                .and_then(|name| name.to_str())?;
            (id == *candidate).then(|| ((*candidate).to_owned(), state.clone()))
        })
    });
    let Some((unit, unit_file_state)) = selected else {
        return Ok(None);
    };
    let path: zbus::zvariant::OwnedObjectPath = manager
        .call("LoadUnit", &(unit.as_str(),))
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let unit_proxy = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        path.as_str(),
        "org.freedesktop.systemd1.Unit",
    )
    .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let active_state = unit_proxy
        .get_property("ActiveState")
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    Ok(Some(SystemdServiceState {
        unit,
        unit_file_state,
        active_state,
    }))
}

#[cfg(target_os = "linux")]
fn unit_enabled(state: &str) -> bool {
    matches!(
        state,
        "enabled" | "enabled-runtime" | "linked" | "linked-runtime"
    )
}

#[cfg(not(target_os = "linux"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "sharing controls are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_service(unit: &str, enabled: bool, label: &str) -> Result<(), Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    let files = vec![unit];
    if enabled {
        let (install, _changes): (bool, Vec<(String, String, String)>) = manager
            .call("EnableUnitFiles", &(files, false, false))
            .map_err(mutation_error)?;
        if !install {
            return Err(Error::new(
                ErrorKind::Mutation,
                format!("the {label} service has no persistent install information"),
            ));
        }
        manager
            .call::<_, _, ()>("Reload", &())
            .map_err(mutation_error)?;
        if let Err(error) =
            manager.call::<_, _, zbus::zvariant::OwnedObjectPath>("StartUnit", &(unit, "replace"))
        {
            let _ = manager.call::<_, _, Vec<(String, String, String)>>(
                "DisableUnitFiles",
                &(vec![unit], false),
            );
            return Err(mutation_error(error));
        }
    } else {
        manager
            .call::<_, _, zbus::zvariant::OwnedObjectPath>("StopUnit", &(unit, "replace"))
            .map_err(mutation_error)?;
        if let Err(error) =
            manager.call::<_, _, Vec<(String, String, String)>>("DisableUnitFiles", &(files, false))
        {
            let _ = manager
                .call::<_, _, zbus::zvariant::OwnedObjectPath>("StartUnit", &(unit, "replace"));
            return Err(mutation_error(error));
        }
        manager
            .call::<_, _, ()>("Reload", &())
            .map_err(mutation_error)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn restore_service(unit: &str, was_active: bool, was_enabled_at_boot: bool) -> Result<(), Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    if was_enabled_at_boot {
        let _: (bool, Vec<(String, String, String)>) = manager
            .call("EnableUnitFiles", &(vec![unit], false, false))
            .map_err(mutation_error)?;
    } else {
        let _: Vec<(String, String, String)> = manager
            .call("DisableUnitFiles", &(vec![unit], false))
            .map_err(mutation_error)?;
    }
    manager
        .call::<_, _, ()>("Reload", &())
        .map_err(mutation_error)?;
    let method = if was_active { "StartUnit" } else { "StopUnit" };
    manager
        .call::<_, _, zbus::zvariant::OwnedObjectPath>(method, &(unit, "replace"))
        .map(|_| ())
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_service(_unit: &str, _enabled: bool, _label: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "Sharing changes are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "linux"))]
fn restore_service(
    _unit: &str,
    _was_active: bool,
    _was_enabled_at_boot: bool,
) -> Result<(), Error> {
    system_snapshot().map(|_| ())
}

#[cfg(target_os = "linux")]
fn wait_for_state(service: ManagedService, enabled: bool) -> Result<Snapshot, Error> {
    for _ in 0..25 {
        let (service_state, enabled_at_boot) = managed_service_state(service)?;
        if requested_state_reached(service_state.as_deref(), enabled_at_boot, enabled) {
            return system_snapshot();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(Error::new(
        ErrorKind::Mutation,
        match service {
            ManagedService::RemoteLogin => "the SSH service did not reach the requested state",
            ManagedService::FileSharing => "the SMB service did not reach the requested state",
        },
    ))
}

#[cfg(any(target_os = "linux", test))]
fn requested_state_reached(
    service_state: Option<&str>,
    enabled_at_boot: bool,
    enabled: bool,
) -> bool {
    let runtime_reached = if enabled {
        service_state == Some("active")
    } else {
        matches!(service_state, Some("inactive" | "failed"))
    };
    runtime_reached && enabled_at_boot == enabled
}

#[cfg(target_os = "linux")]
fn managed_service_state(service: ManagedService) -> Result<(Option<String>, bool), Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    let files = manager
        .call::<_, _, Vec<(String, String)>>("ListUnitFiles", &())
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let candidates = match service {
        ManagedService::RemoteLogin => &["ssh.service", "sshd.service"][..],
        ManagedService::FileSharing => &["smbd.service"][..],
    };
    let Some(state) = service_state(&connection, &manager, &files, candidates)? else {
        return Ok((None, false));
    };
    Ok((
        Some(state.active_state),
        unit_enabled(&state.unit_file_state),
    ))
}

#[cfg(not(target_os = "linux"))]
fn wait_for_state(_service: ManagedService, _enabled: bool) -> Result<Snapshot, Error> {
    system_snapshot()
}

#[cfg(target_os = "linux")]
fn firewall_state(service: FirewallService) -> (FirewallState, Option<String>) {
    let output = std::process::Command::new("ufw").arg("status").output();
    let Ok(output) = output else {
        return (
            FirewallState::Unavailable,
            Some("UFW is not installed or could not be executed".into()),
        );
    };
    if !output.status.success() {
        return (
            FirewallState::Unavailable,
            Some("UFW status requires additional authorization".into()),
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let state = parse_ufw_status(&text, service);
    if state == FirewallState::ActiveUnverified {
        (
            state,
            Some(match service {
                FirewallService::Ssh => {
                    "No explicit OpenSSH or TCP port 22 allow rule was found".into()
                }
                FirewallService::Samba => {
                    "No explicit UFW Samba profile allow rule was found".into()
                }
            }),
        )
    } else {
        (state, None)
    }
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FirewallService {
    Ssh,
    Samba,
}

#[cfg(any(target_os = "linux", test))]
fn parse_ufw_status(text: &str, service: FirewallService) -> FirewallState {
    let lowercase = text.to_ascii_lowercase();
    if lowercase
        .lines()
        .any(|line| line.trim() == "status: inactive")
    {
        FirewallState::Inactive
    } else if lowercase.lines().any(|line| match service {
        FirewallService::Ssh => {
            (line.contains("openssh") || line.contains("22/tcp") || line.contains("ssh "))
                && line.contains("allow")
        }
        FirewallService::Samba => line.contains("samba") && line.contains("allow"),
    }) {
        FirewallState::Allows
    } else {
        FirewallState::ActiveUnverified
    }
}

#[cfg(target_os = "linux")]
fn samba_shares() -> (Vec<Share>, bool, Option<String>) {
    const MAX_OUTPUT_BYTES: usize = 512 * 1024;
    let output = std::process::Command::new("testparm").arg("-s").output();
    let Ok(output) = output else {
        return (
            Vec::new(),
            false,
            Some("Samba's testparm validator is not installed or could not be executed".into()),
        );
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        return (
            Vec::new(),
            false,
            Some(if detail.is_empty() {
                "Samba rejected the effective configuration".into()
            } else {
                format!("Samba rejected the effective configuration: {detail}")
            }),
        );
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return (
            Vec::new(),
            true,
            Some("Samba's effective configuration exceeded the safe read limit".into()),
        );
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return (
            Vec::new(),
            false,
            Some("Samba returned a non-UTF-8 effective configuration".into()),
        );
    };
    let (shares, truncated) = parse_samba_shares(&text);
    (shares, truncated, None)
}

#[cfg(any(target_os = "linux", test))]
fn parse_samba_shares(text: &str) -> (Vec<Share>, bool) {
    const MAX_SHARES: usize = 128;
    let mut names = text
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let name = line.strip_prefix('[')?.strip_suffix(']')?.trim();
            (!name.is_empty()
                && name.len() <= 128
                && !name.chars().any(char::is_control)
                && !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "global" | "printers" | "print$"
                ))
            .then(|| name.to_owned())
        })
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    let truncated = names.len() > MAX_SHARES;
    names.truncate(MAX_SHARES);
    (
        names.into_iter().map(|name| Share { name }).collect(),
        truncated,
    )
}

#[cfg(target_os = "linux")]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::Connection::system().map_err(|_| {
        Error::new(
            ErrorKind::Unavailable,
            "the system service manager is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
fn manager_proxy(
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
            "the system service manager is unavailable",
        )
    })
}

#[cfg(target_os = "linux")]
fn mutation_error(error: zbus::Error) -> Error {
    let detail = error.to_string();
    let lowercase = detail.to_ascii_lowercase();
    if lowercase.contains("accessdenied")
        || lowercase.contains("not authorized")
        || lowercase.contains("authentication")
        || lowercase.contains("polkit")
    {
        Error::new(
            ErrorKind::Authorization,
            "authorization was denied or cancelled",
        )
    } else {
        Error::new(ErrorKind::Mutation, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ufw_parser_requires_an_explicit_allow_rule() {
        assert_eq!(
            parse_ufw_status("Status: inactive\n", FirewallService::Ssh),
            FirewallState::Inactive
        );
        assert_eq!(
            parse_ufw_status(
                "Status: active\n22/tcp ALLOW Anywhere\n",
                FirewallService::Ssh
            ),
            FirewallState::Allows
        );
        assert_eq!(
            parse_ufw_status(
                "Status: active\n22/tcp DENY Anywhere\n",
                FirewallService::Ssh
            ),
            FirewallState::ActiveUnverified
        );
        assert_eq!(
            parse_ufw_status(
                "Status: active\nSamba ALLOW Anywhere\n",
                FirewallService::Samba
            ),
            FirewallState::Allows
        );
        assert_eq!(
            parse_ufw_status(
                "Status: active\n445/tcp ALLOW Anywhere\n",
                FirewallService::Samba
            ),
            FirewallState::ActiveUnverified
        );
    }

    #[test]
    fn samba_parser_exposes_only_bounded_file_share_names() {
        let (shares, truncated) = parse_samba_shares(
            "[global]\n[homes]\n[printers]\n[print$]\n[Team Files]\n[team files]\n\t[option value]\n",
        );
        assert!(!truncated);
        assert_eq!(
            shares
                .iter()
                .map(|share| share.name.as_str())
                .collect::<Vec<_>>(),
            ["homes", "Team Files"]
        );

        let input = (0..130)
            .map(|index| format!("[share-{index}]"))
            .collect::<Vec<_>>()
            .join("\n");
        let (shares, truncated) = parse_samba_shares(&input);
        assert!(truncated);
        assert_eq!(shares.len(), 128);
    }

    #[test]
    fn convergence_requires_both_runtime_and_boot_authorities() {
        assert!(requested_state_reached(Some("active"), true, true));
        assert!(!requested_state_reached(Some("active"), false, true));
        assert!(!requested_state_reached(Some("activating"), true, true));
        assert!(requested_state_reached(Some("inactive"), false, false));
        assert!(requested_state_reached(Some("failed"), false, false));
        assert!(!requested_state_reached(Some("inactive"), true, false));
        assert!(!requested_state_reached(None, false, false));
    }

    #[test]
    fn watcher_filters_firewall_files_and_systemd_idle_exit() {
        assert!(firewall_path_relevant(std::path::Path::new(
            "/etc/ufw/user.rules"
        )));
        assert!(!firewall_path_relevant(std::path::Path::new(
            "/etc/ufw/sysctl.conf"
        )));
        assert!(samba_path_relevant(std::path::Path::new(
            "/etc/samba/smb.conf"
        )));
        assert!(!samba_path_relevant(std::path::Path::new(
            "/etc/samba/private/secrets.tdb"
        )));
        assert!(!owner_change_reappeared("org.freedesktop.systemd1", ""));
        assert!(owner_change_reappeared("org.freedesktop.systemd1", ":1.42"));
    }
}
