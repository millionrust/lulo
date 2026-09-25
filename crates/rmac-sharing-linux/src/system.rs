#[cfg(any(target_os = "linux", test))]
use rmac_sharing::Share;
use rmac_sharing::{Error, ErrorKind, Snapshot};
#[cfg(target_os = "linux")]
use rmac_sharing::{FileSharing, FirewallState, RemoteLogin};

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

// Ubuntu 26.04 (the reference laptop) ships `ufw` 0.36.2 with no D-Bus
// service (`busctl list` shows nothing under `org.fedoraproject.FirewallD1`
// or any UFW-owned name) and `firewalld` is not installed. `ufw status`
// itself refuses to run unprivileged ("ERROR: You need to be root to run
// this script"), so the previous `Command::new("ufw").arg("status")` call
// always failed on this machine and silently degraded to `Unavailable`.
// UFW's own on/off switch is a stable `ENABLED=yes|no` key in
// `/etc/ufw/ufw.conf`, which stays world-readable (0644) even though the
// per-rule file `/etc/ufw/user.rules` is root-only (0640) — so this can
// report Active/Inactive without root, but can no longer see individual
// allow rules (that needs the same root ufw status always needed).
#[cfg(target_os = "linux")]
const UFW_CONF_PATH: &str = "/etc/ufw/ufw.conf";

#[cfg(target_os = "linux")]
fn firewall_state(service: FirewallService) -> (FirewallState, Option<String>) {
    match read_bounded_config(UFW_CONF_PATH) {
        Ok(text) => match parse_ufw_conf_enabled(&text) {
            Some(true) => (
                FirewallState::ActiveUnverified,
                Some(match service {
                    FirewallService::Ssh => "UFW is active; whether it explicitly allows OpenSSH or TCP port 22 cannot be read without root (per-rule config is not world-readable)".into(),
                    FirewallService::Samba => "UFW is active; whether it explicitly allows Samba cannot be read without root (per-rule config is not world-readable)".into(),
                }),
            ),
            Some(false) => (FirewallState::Inactive, None),
            None => (
                FirewallState::Unavailable,
                Some(format!("{UFW_CONF_PATH} did not contain an ENABLED setting")),
            ),
        },
        Err(detail) => (FirewallState::Unavailable, Some(detail)),
    }
}

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirewallService {
    Ssh,
    Samba,
}

/// Parse UFW's own `ENABLED=yes|no` setting out of `/etc/ufw/ufw.conf`
/// (or the same key in a similarly-shaped file). This is stable key=value
/// config UFW itself writes and reads — not command output — so comments
/// (`#...`), blank lines, and optional quoting around the value are the
/// only syntax handled.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_ufw_conf_enabled(text: &str) -> Option<bool> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        if key.trim() != "ENABLED" {
            return None;
        }
        let value = value.trim().trim_matches(['"', '\'']).trim();
        match value.to_ascii_lowercase().as_str() {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        }
    })
}

// Ubuntu's `samba` metapackage (not installed on the reference laptop,
// which only carries `samba-libs`) puts the running server's configuration
// at `/etc/samba/smb.conf`, a plain public config file `smbd` reads
// directly, plus any shares a user created with `net usershare` as one
// file per share under `/var/lib/samba/usershares` (root:sambashare,
// group-readable by that group; the filename *is* the share name).
// `testparm -s` merely resolves and prints that same config; reading the
// files directly avoids spawning it and parsing prose output.
#[cfg(target_os = "linux")]
const SMB_CONF_PATH: &str = "/etc/samba/smb.conf";
#[cfg(target_os = "linux")]
const SAMBA_USERSHARE_DIR: &str = "/var/lib/samba/usershares";

#[cfg(target_os = "linux")]
fn samba_shares() -> (Vec<Share>, bool, Option<String>) {
    let mut names = usershare_names();
    let configuration_error = match read_bounded_config(SMB_CONF_PATH) {
        Ok(text) => {
            names.extend(parse_smb_conf_shares(&text));
            None
        }
        Err(detail) => {
            // A host can offer only `net usershare` shares with no
            // system-wide smb.conf at all; only report an error if there is
            // truly nothing to show.
            if names.is_empty() {
                Some(detail)
            } else {
                None
            }
        }
    };
    let (shares, truncated) = bounded_shares(names);
    (shares, truncated, configuration_error)
}

/// List share names from Samba's user-share directory, where the file name
/// itself is the share name (see `net usershare(8)`). Directory entries are
/// used only as names, never opened or followed.
#[cfg(target_os = "linux")]
fn usershare_names() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(SAMBA_USERSHARE_DIR) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| sanitize_share_name(name).is_some())
        .collect()
}

/// Parse `[section]` share names out of a Samba `smb.conf`-shaped file,
/// skipping the reserved `global`/`printers`/`print$` sections and any
/// section explicitly marked `available = no`. This is a small INI reader,
/// not a full Samba config evaluator: it does not follow `include =`
/// directives or apply `[global]` defaults to per-share parameters, so a
/// share whose availability is only set globally will not be filtered.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_smb_conf_shares(text: &str) -> Vec<String> {
    let mut shares = Vec::new();
    let mut current: Option<(String, bool)> = None;
    let flush = |current: Option<(String, bool)>, shares: &mut Vec<String>| {
        if let Some((name, available)) = current {
            if available {
                shares.push(name);
            }
        }
    };
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            flush(current.take(), &mut shares);
            let name = name.trim();
            current = sanitize_share_name(name).map(|name| (name, true));
            continue;
        }
        let Some((_, available)) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase().replace([' ', '_'], "");
        if key == "available" {
            let value = value.trim().to_ascii_lowercase();
            *available = !matches!(value.as_str(), "no" | "false" | "0");
        }
    }
    flush(current, &mut shares);
    shares
}

#[cfg(any(target_os = "linux", test))]
fn sanitize_share_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && name.len() <= 128
        && !name.chars().any(char::is_control)
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "global" | "printers" | "print$"
        ))
    .then(|| name.to_owned())
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn bounded_shares(names: Vec<String>) -> (Vec<Share>, bool) {
    const MAX_SHARES: usize = 128;
    let mut names = names;
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    let truncated = names.len() > MAX_SHARES;
    names.truncate(MAX_SHARES);
    (
        names.into_iter().map(|name| Share { name }).collect(),
        truncated,
    )
}

/// Read a small, bounded, UTF-8 configuration file. Used only for the
/// stable key=value/INI config files UFW and Samba write and read
/// themselves — never for command output.
#[cfg(target_os = "linux")]
fn read_bounded_config(path: &str) -> Result<String, String> {
    const MAX_CONFIG_BYTES: u64 = 512 * 1024;
    let metadata = std::fs::metadata(path)
        .map_err(|_| format!("{path} is not installed or could not be read"))?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!("{path} exceeded the safe read limit"));
    }
    std::fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidData {
            format!("{path} is not valid UTF-8")
        } else {
            format!("{path} could not be read: {error}")
        }
    })
}

#[cfg(target_os = "linux")]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    rmac_dbus::system_blocking().map_err(|_| {
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
