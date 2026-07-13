//! Linux sharing adapter backed by systemd and read-only firewall inspection.

#[cfg(any(target_os = "linux", test))]
use rmac_sharing::FirewallState;
use rmac_sharing::{Error, ErrorKind, RemoteLogin, Service, Snapshot};

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
        if let Err(error) = system_set_remote_login(unit, enabled) {
            let _ = restore_remote_login(unit, &current.remote_login);
            return Err(error);
        }
        match wait_for_state(enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                let _ = restore_remote_login(unit, &current.remote_login);
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

#[cfg(target_os = "linux")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    let files = manager
        .call::<_, _, Vec<(String, String)>>("ListUnitFiles", &())
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let selected = ["ssh.service", "sshd.service"]
        .into_iter()
        .find_map(|candidate| {
            files.iter().find_map(|(name, state)| {
                let id = std::path::Path::new(name)
                    .file_name()
                    .and_then(|name| name.to_str())?;
                (id == candidate).then(|| (candidate.to_owned(), state.clone()))
            })
        });
    let Some((unit, unit_file_state)) = selected else {
        let (firewall, firewall_detail) = firewall_state();
        return Ok(Snapshot {
            remote_login: RemoteLogin {
                firewall,
                firewall_detail,
                ..RemoteLogin::default()
            },
        });
    };
    let path: zbus::zvariant::OwnedObjectPath = manager
        .call("LoadUnit", &(unit.as_str(),))
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let unit_proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        path.as_str(),
        "org.freedesktop.systemd1.Unit",
    )
    .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let active_state: String = unit_proxy
        .get_property("ActiveState")
        .map_err(|error| Error::new(ErrorKind::Protocol, error.to_string()))?;
    let (firewall, firewall_detail) = firewall_state();
    Ok(Snapshot {
        remote_login: RemoteLogin {
            available: true,
            unit: Some(unit),
            active: active_state == "active",
            service_state: Some(active_state),
            enabled_at_boot: matches!(
                unit_file_state.as_str(),
                "enabled" | "enabled-runtime" | "linked" | "linked-runtime"
            ),
            unit_file_state: Some(unit_file_state),
            firewall,
            firewall_detail,
        },
    })
}

#[cfg(not(target_os = "linux"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "sharing controls are available in the supported Linux session",
    ))
}

#[cfg(target_os = "linux")]
fn system_set_remote_login(unit: &str, enabled: bool) -> Result<(), Error> {
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
                "the SSH service has no persistent install information",
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
fn restore_remote_login(unit: &str, previous: &RemoteLogin) -> Result<(), Error> {
    let connection = system_connection()?;
    let manager = manager_proxy(&connection)?;
    if previous.enabled_at_boot {
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
    let method = if previous.active {
        "StartUnit"
    } else {
        "StopUnit"
    };
    manager
        .call::<_, _, zbus::zvariant::OwnedObjectPath>(method, &(unit, "replace"))
        .map(|_| ())
        .map_err(mutation_error)
}

#[cfg(not(target_os = "linux"))]
fn system_set_remote_login(_unit: &str, _enabled: bool) -> Result<(), Error> {
    Err(Error::new(
        ErrorKind::Unavailable,
        "Remote Login changes are available in the supported Linux session",
    ))
}

#[cfg(not(target_os = "linux"))]
fn restore_remote_login(_unit: &str, _previous: &RemoteLogin) -> Result<(), Error> {
    system_snapshot().map(|_| ())
}

#[cfg(target_os = "linux")]
fn wait_for_state(enabled: bool) -> Result<Snapshot, Error> {
    for _ in 0..25 {
        let snapshot = system_snapshot()?;
        let service_reached = if enabled {
            snapshot.remote_login.service_state.as_deref() == Some("active")
        } else {
            matches!(
                snapshot.remote_login.service_state.as_deref(),
                Some("inactive" | "failed")
            )
        };
        if service_reached && snapshot.remote_login.enabled_at_boot == enabled {
            return Ok(snapshot);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(Error::new(
        ErrorKind::Mutation,
        "the SSH service did not reach the requested state",
    ))
}

#[cfg(not(target_os = "linux"))]
fn wait_for_state(_enabled: bool) -> Result<Snapshot, Error> {
    system_snapshot()
}

#[cfg(target_os = "linux")]
fn firewall_state() -> (FirewallState, Option<String>) {
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
    let state = parse_ufw_status(&text);
    if state == FirewallState::ActiveUnverified {
        (
            state,
            Some("No explicit OpenSSH or TCP port 22 allow rule was found".into()),
        )
    } else {
        (state, None)
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_ufw_status(text: &str) -> FirewallState {
    let lowercase = text.to_ascii_lowercase();
    if lowercase
        .lines()
        .any(|line| line.trim() == "status: inactive")
    {
        FirewallState::Inactive
    } else if lowercase.lines().any(|line| {
        (line.contains("openssh") || line.contains("22/tcp") || line.contains("ssh "))
            && line.contains("allow")
    }) {
        FirewallState::AllowsSsh
    } else {
        FirewallState::ActiveUnverified
    }
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
            parse_ufw_status("Status: inactive\n"),
            FirewallState::Inactive
        );
        assert_eq!(
            parse_ufw_status("Status: active\n22/tcp ALLOW Anywhere\n"),
            FirewallState::AllowsSsh
        );
        assert_eq!(
            parse_ufw_status("Status: active\n22/tcp DENY Anywhere\n"),
            FirewallState::ActiveUnverified
        );
    }
}
