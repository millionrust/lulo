//! Who may call the service (ADR 0024 §4 "Caller checks"): the same user,
//! from one of the Lulo programs that use it. The program check reads the
//! caller's executable (`/proc/<pid>/exe`); it is defence in depth, not a
//! security boundary (a same-user process could always run a Lulo program
//! itself).
//!
//! Reading another process's `/proc/<pid>/exe` is a ptrace "read" access.
//! The kernel grants it to a same-user process only when the reader is in
//! the same user namespace as the target, or in an ancestor namespace that
//! owns the target's. So the service must run in the session's own user
//! namespace: its unit must not use any option that makes systemd's user
//! manager create one (`PrivateTmp=`, `PrivateUsers=`, `ProtectSystem=` and
//! the other mount-namespace options), or every caller reads as "cannot be
//! checked" (the 2026-10-08 production bug). Callers may run in their own
//! namespaces: Spotlight does (`rmac-launcher.service` uses `PrivateTmp=`).

use std::path::{Path, PathBuf};

/// The programs that call the service, by file name: Spotlight's "Lulo can
/// do this" rows and System Settings ▸ Lulo Intelligence (Prepare and
/// Calibrate).
pub const CALLERS: &[&str] = &["rmac-launcher", "rmac-system-settings"];

/// Measures the service on the reference laptop. Not packaged, so trusted
/// only beside the service itself.
pub const DEVELOPMENT_CALLERS: &[&str] = &["rmac-intelligence-bench"];

/// Where the package installs [`CALLERS`].
const INSTALLED_CALLERS: &[&str] = &[
    "/usr/libexec/rmac/rmac-launcher",
    "/usr/bin/rmac-system-settings",
];

/// The exact programs allowed to call: the installed ones, and the same
/// names beside this service (development installs under
/// `~/.local/libexec/rmac`, and the private test sessions, keep every
/// program side by side). Nothing else in `/usr/bin` or anywhere else.
pub fn trusted_programs() -> Vec<PathBuf> {
    let own = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    trusted_programs_beside(own.as_deref())
}

/// [`trusted_programs`] for a service installed in `own`.
pub fn trusted_programs_beside(own: Option<&Path>) -> Vec<PathBuf> {
    let mut programs: Vec<PathBuf> = INSTALLED_CALLERS.iter().map(PathBuf::from).collect();
    if let Some(own) = own {
        for name in CALLERS.iter().chain(DEVELOPMENT_CALLERS) {
            let path = own.join(name);
            if !programs.contains(&path) {
                programs.push(path);
            }
        }
    }
    programs
}

/// Whether `executable`, as `/proc/<pid>/exe` reads, is one of `trusted`.
/// A program replaced on disk since it started (a package upgrade while
/// Spotlight runs) reads back as `<path> (deleted)`: it is still the
/// program that was installed at that path.
pub fn executable_allowed(executable: &Path, trusted: &[PathBuf]) -> bool {
    let path = executable
        .to_str()
        .and_then(|text| text.strip_suffix(" (deleted)"))
        .map(Path::new)
        .unwrap_or(executable);
    trusted.iter().any(|program| program == path)
}

/// Why a caller was refused. The text names the reason for the log line;
/// it never contains the request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The bus did not say who the caller is.
    NoCredentials(&'static str),
    /// The caller runs as another user.
    OtherUser(u32),
    /// The bus's pid and pidfd for the caller disagree.
    ProcessMismatch { reported: u32, pidfd: u32 },
    /// The caller's process ended (or its pid was reused) during the check.
    Exited(u32),
    /// The caller's executable could not be read.
    Unreadable { pid: u32, error: String },
    /// The caller is not one of [`trusted_programs`].
    NotLulo { pid: u32, executable: PathBuf },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCredentials(what) => write!(formatter, "the caller cannot be checked: {what}"),
            Self::OtherUser(uid) => write!(formatter, "the caller is another user (uid {uid})"),
            Self::ProcessMismatch { reported, pidfd } => write!(
                formatter,
                "the caller cannot be checked: the bus reported pid {reported} but its pidfd is pid {pidfd}"
            ),
            Self::Exited(pid) => write!(
                formatter,
                "the caller cannot be checked: process {pid} ended during the check"
            ),
            Self::Unreadable { pid, error } => write!(
                formatter,
                "the caller cannot be checked: /proc/{pid}/exe: {error} \
                 (does the service run in its own user namespace?)"
            ),
            Self::NotLulo { pid, executable } => write!(
                formatter,
                "the caller is not a Lulo program (pid {pid}, {})",
                executable.display()
            ),
        }
    }
}

/// This process's user id: the owner of `/proc/self`.
#[cfg(target_os = "linux")]
pub fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

/// The pid a pidfd refers to, from `/proc/self/fdinfo`. `None` when the fd
/// is not a pidfd, or its process has ended (the kernel then shows -1).
#[cfg(target_os = "linux")]
pub fn pidfd_pid(pidfd: std::os::fd::BorrowedFd<'_>) -> Option<u32> {
    use std::os::fd::AsRawFd as _;
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok()?;
    info.lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|pid| pid.trim().parse::<i64>().ok())
        .filter(|pid| *pid > 0)
        .and_then(|pid| u32::try_from(pid).ok())
}

/// Check a caller from its bus credentials (`GetConnectionCredentials`):
/// same user, and one of `trusted`. With a `ProcessFD` (dbus-daemon 1.16,
/// dbus-broker) the pid cannot be reused while the executable is read: the
/// pidfd is checked again afterwards. Older buses give only the pid.
/// Fails closed: anything unknown or unreadable refuses.
#[cfg(target_os = "linux")]
pub fn check(
    uid: Option<u32>,
    pid: Option<u32>,
    pidfd: Option<std::os::fd::BorrowedFd<'_>>,
    trusted: &[PathBuf],
) -> Result<PathBuf, Refusal> {
    let own = own_uid().ok_or(Refusal::NoCredentials("this service's own user is unknown"))?;
    let uid = uid.ok_or(Refusal::NoCredentials("the bus gave no user id"))?;
    if uid != own {
        return Err(Refusal::OtherUser(uid));
    }
    let pid = match (pidfd, pid) {
        (Some(pidfd), reported) => {
            let live = pidfd_pid(pidfd).ok_or(Refusal::Exited(reported.unwrap_or(0)))?;
            if let Some(reported) = reported.filter(|reported| *reported != live) {
                return Err(Refusal::ProcessMismatch {
                    reported,
                    pidfd: live,
                });
            }
            live
        }
        (None, Some(pid)) => pid,
        (None, None) => return Err(Refusal::NoCredentials("the bus gave no process id")),
    };
    let executable =
        std::fs::read_link(format!("/proc/{pid}/exe")).map_err(|error| Refusal::Unreadable {
            pid,
            error: error.to_string(),
        })?;
    if pidfd.is_some_and(|pidfd| pidfd_pid(pidfd) != Some(pid)) {
        return Err(Refusal::Exited(pid));
    }
    if !executable_allowed(&executable, trusted) {
        return Err(Refusal::NotLulo { pid, executable });
    }
    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_lulo_programs_are_trusted() {
        let trusted = trusted_programs_beside(Some(Path::new("/home/u/.local/libexec/rmac")));
        for allowed in [
            "/usr/libexec/rmac/rmac-launcher",
            "/usr/bin/rmac-system-settings",
            "/home/u/.local/libexec/rmac/rmac-launcher",
            "/home/u/.local/libexec/rmac/rmac-system-settings",
            "/home/u/.local/libexec/rmac/rmac-intelligence-bench",
            // Replaced by an upgrade while it runs.
            "/usr/libexec/rmac/rmac-launcher (deleted)",
        ] {
            assert!(
                executable_allowed(Path::new(allowed), &trusted),
                "{allowed} should be trusted"
            );
        }
        for refused in [
            // Any other program in /usr/bin, even with an rmac- name.
            "/usr/bin/python3",
            "/usr/bin/busctl",
            "/usr/bin/gdbus",
            "/usr/bin/rmac-terminal",
            "/usr/bin/rmac-launcher",
            // Other Lulo programs that never call the service.
            "/usr/libexec/rmac/rmac-dock",
            "/usr/libexec/rmac/rmac-shortcut-dispatch",
            // The bench only beside the service, never from the package dir.
            "/usr/libexec/rmac/rmac-intelligence-bench",
            "/tmp/rmac-launcher",
            "/usr/libexec/rmac/evil",
            "/usr/libexec/rmac/sub/rmac-launcher",
            "/usr/libexec/rmac/rmac-launcher.bak",
            "/usr/libexec/rmac/rmac-launcher (deleted) (deleted)",
            "rmac-launcher",
        ] {
            assert!(
                !executable_allowed(Path::new(refused), &trusted),
                "{refused} should be refused"
            );
        }
    }

    #[test]
    fn a_packaged_service_trusts_only_the_package_paths() {
        let trusted = trusted_programs_beside(Some(Path::new("/usr/libexec/rmac")));
        assert_eq!(
            trusted,
            [
                "/usr/libexec/rmac/rmac-launcher",
                "/usr/bin/rmac-system-settings",
                "/usr/libexec/rmac/rmac-system-settings",
                "/usr/libexec/rmac/rmac-intelligence-bench",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn refusals_name_the_reason() {
        let unreadable = Refusal::Unreadable {
            pid: 42,
            error: "Permission denied (os error 13)".into(),
        };
        assert!(unreadable
            .to_string()
            .contains("/proc/42/exe: Permission denied"));
        let other = Refusal::NotLulo {
            pid: 7,
            executable: PathBuf::from("/usr/bin/python3"),
        };
        assert_eq!(
            other.to_string(),
            "the caller is not a Lulo program (pid 7, /usr/bin/python3)"
        );
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use std::os::fd::{AsFd as _, FromRawFd as _, OwnedFd};

        fn pidfd_open(pid: u32) -> OwnedFd {
            // SAFETY: pidfd_open takes a pid and flags and returns a new fd.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
            assert!(fd >= 0, "pidfd_open failed");
            // SAFETY: the kernel just returned this fd and nothing else owns it.
            unsafe { OwnedFd::from_raw_fd(fd as i32) }
        }

        #[test]
        fn a_pidfd_resolves_to_its_live_process_only() {
            let mut child = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("sleep starts");
            let pidfd = pidfd_open(child.id());
            assert_eq!(pidfd_pid(pidfd.as_fd()), Some(child.id()));
            child.kill().expect("kill");
            child.wait().expect("reap");
            assert_eq!(pidfd_pid(pidfd.as_fd()), None);
            // A plain file is not a pidfd.
            let file = std::fs::File::open("/proc/self/stat").expect("open");
            assert_eq!(pidfd_pid(file.as_fd()), None);
        }

        #[test]
        fn the_check_follows_the_pidfd_and_fails_closed() {
            let own = own_uid();
            let me = std::process::id();
            let exe = std::env::current_exe().expect("exe");
            let pidfd = pidfd_open(me);
            // This test binary is trusted only when listed.
            assert_eq!(
                check(
                    own,
                    Some(me),
                    Some(pidfd.as_fd()),
                    std::slice::from_ref(&exe)
                ),
                Ok(exe.clone())
            );
            assert_eq!(
                check(own, None, Some(pidfd.as_fd()), std::slice::from_ref(&exe)),
                Ok(exe.clone())
            );
            assert_eq!(
                check(own, Some(me), None, std::slice::from_ref(&exe)),
                Ok(exe.clone())
            );
            assert!(matches!(
                check(own, Some(me), Some(pidfd.as_fd()), &trusted_programs()),
                Err(Refusal::NotLulo { .. })
            ));
            assert_eq!(
                check(
                    own.map(|uid| uid + 1),
                    Some(me),
                    Some(pidfd.as_fd()),
                    std::slice::from_ref(&exe)
                ),
                Err(Refusal::OtherUser(own.unwrap_or(0) + 1))
            );
            assert!(matches!(
                check(None, Some(me), None, std::slice::from_ref(&exe)),
                Err(Refusal::NoCredentials(_))
            ));
            assert!(matches!(
                check(own, None, None, std::slice::from_ref(&exe)),
                Err(Refusal::NoCredentials(_))
            ));
            assert_eq!(
                check(
                    own,
                    Some(me + 1),
                    Some(pidfd.as_fd()),
                    std::slice::from_ref(&exe)
                ),
                Err(Refusal::ProcessMismatch {
                    reported: me + 1,
                    pidfd: me
                })
            );
            // A caller that has gone is refused, never guessed at.
            let mut child = std::process::Command::new("true").spawn().expect("true");
            let gone = pidfd_open(child.id());
            child.wait().expect("reap");
            assert_eq!(
                check(own, Some(child.id()), Some(gone.as_fd()), &[exe]),
                Err(Refusal::Exited(child.id()))
            );
        }
    }
}
