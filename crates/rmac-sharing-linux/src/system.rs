use rmac_sharing::{Error, ErrorKind, Snapshot};
#[cfg(target_os = "linux")]
use rmac_sharing::{FileSharing, RemoteLogin};
#[cfg(any(target_os = "linux", test))]
use rmac_sharing::{FirewallState, Share};

use crate::service::ManagedService;

#[cfg(target_os = "linux")]
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
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
pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "sharing controls are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_set_service(unit: &str, enabled: bool, label: &str) -> Result<(), Error> {
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
pub(crate) fn restore_service(
    unit: &str,
    was_active: bool,
    was_enabled_at_boot: bool,
) -> Result<(), Error> {
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
pub(crate) fn system_set_service(_unit: &str, _enabled: bool, _label: &str) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "Sharing changes are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn restore_service(
    _unit: &str,
    _was_active: bool,
    _was_enabled_at_boot: bool,
) -> Result<(), Error> {
    system_snapshot().map(|_| ())
}

#[cfg(target_os = "linux")]
pub(crate) fn wait_for_state(service: ManagedService, enabled: bool) -> Result<Snapshot, Error> {
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
pub(crate) fn requested_state_reached(
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
pub(crate) fn wait_for_state(_service: ManagedService, _enabled: bool) -> Result<Snapshot, Error> {
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
pub(crate) enum FirewallService {
    Ssh,
    Samba,
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_ufw_status(text: &str, service: FirewallService) -> FirewallState {
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
pub(crate) fn parse_samba_shares(text: &str) -> (Vec<Share>, bool) {
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
