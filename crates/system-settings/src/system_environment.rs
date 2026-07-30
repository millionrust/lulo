use std::process::Command;

/// Read-only system data that is slow enough to keep off the first-frame path.
pub(super) struct SystemSnapshot {
    pub(super) account: String,
    pub(super) sysinfo: std::result::Result<rmac_system_info::Snapshot, rmac_system_info::Error>,
    pub(super) storage: std::result::Result<Vec<rmac_mounts::Volume>, rmac_mounts::Error>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ScreenReaderCapability {
    pub(super) niri_session: bool,
    pub(super) x11_display: bool,
    pub(super) xwayland_satellite_installed: bool,
    pub(super) orca_installed: bool,
}

impl ScreenReaderCapability {
    pub(super) fn prerequisites_present(&self, enabled_output: bool) -> bool {
        self.niri_session && self.x11_display && self.orca_installed && enabled_output
    }

    pub(super) fn limitation(&self, enabled_output: bool) -> Option<&'static str> {
        if !self.niri_session {
            Some("Start the desktop through a full niri-session")
        } else if !enabled_output {
            Some("Connect and enable a display before testing Orca")
        } else if !self.x11_display {
            Some("An exported Xwayland DISPLAY is required by Orca with niri")
        } else if !self.orca_installed {
            Some("Install Orca to enable screen-reader support")
        } else {
            None
        }
    }
}

pub(super) fn gather_system_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        account: account_name(),
        sysinfo: rmac_system_info::snapshot(),
        storage: rmac_mounts::volumes(),
    }
}

pub(super) fn gather_screen_reader_capability() -> ScreenReaderCapability {
    let niri_desktop = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .any(|value| {
            value
                .split([':', ';'])
                .any(|desktop| desktop.eq_ignore_ascii_case("niri"))
        });
    let wayland_session =
        std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value.eq_ignore_ascii_case("wayland"));
    let niri_socket = std::env::var_os("NIRI_SOCKET").is_some_and(|value| !value.is_empty());
    ScreenReaderCapability {
        niri_session: niri_desktop && wayland_session && niri_socket,
        x11_display: std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()),
        xwayland_satellite_installed: executable_in_path("xwayland-satellite"),
        orca_installed: executable_in_path("orca"),
    }
}

fn executable_in_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .take(128)
            .map(|directory| directory.join(program))
            .any(|candidate| is_executable_file(&candidate))
    })
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

fn account_name() -> String {
    command_output("id", &["-F"])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "User".into())
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|output| !output.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_reader_readiness_requires_every_niri_orca_authority() {
        let mut capability = ScreenReaderCapability::default();
        assert_eq!(
            capability.limitation(false),
            Some("Start the desktop through a full niri-session")
        );
        capability.niri_session = true;
        assert_eq!(
            capability.limitation(false),
            Some("Connect and enable a display before testing Orca")
        );
        assert_eq!(
            capability.limitation(true),
            Some("An exported Xwayland DISPLAY is required by Orca with niri")
        );
        capability.x11_display = true;
        assert_eq!(
            capability.limitation(true),
            Some("Install Orca to enable screen-reader support")
        );
        capability.orca_installed = true;
        assert!(capability.prerequisites_present(true));
        assert!(!capability.prerequisites_present(false));
        assert_eq!(capability.limitation(true), None);
    }
}
