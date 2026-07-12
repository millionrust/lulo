//! Security boundary for starting and supervising the session locker.

use std::fmt;
use std::io;
use std::path::Path;
#[cfg(any(target_os = "linux", test))]
use std::process::ExitStatus;

#[cfg(target_os = "linux")]
use std::io::Read as _;
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "linux")]
const SYSTEMCTL: &str = "/usr/bin/systemctl";
#[cfg(target_os = "linux")]
const SYSTEMD_NOTIFY: &str = "/usr/bin/systemd-notify";
#[cfg(any(target_os = "linux", test))]
const SWAYLOCK: &str = "/usr/bin/swaylock";
#[cfg(target_os = "linux")]
const LOCK_UNIT: &str = "rmac-lock.service";
#[cfg(target_os = "linux")]
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Request,
    ValidateConfig,
    ReadConfig,
    SpawnLocker,
    WaitForLock,
    NotifySupervisor,
    UpdateLockedHint,
    WaitForUnlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: io::ErrorKind,
}

impl Error {
    #[cfg(any(target_os = "linux", test))]
    fn io(operation: Operation, error: io::Error) -> Self {
        Self {
            operation,
            kind: error.kind(),
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn failed(operation: Operation) -> Self {
        Self {
            operation,
            kind: io::ErrorKind::Other,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "secure session lock failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

pub fn request() -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        let status = Command::new(SYSTEMCTL)
            .args(["--user", "start", LOCK_UNIT])
            .status()
            .map_err(|error| Error::io(Operation::Request, error))?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::failed(Operation::Request))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Request,
            kind: io::ErrorKind::Unsupported,
        })
    }
}

#[cfg(target_os = "linux")]
pub fn supervise(config: &Path) -> Result<(), Error> {
    validate_config(config)?;
    let mut locker = Command::new(SWAYLOCK)
        .arg("--config")
        .arg(config)
        .arg("--ready-fd=1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| Error::io(Operation::SpawnLocker, error))?;
    if !wait_for_ready(&mut locker)? {
        return Err(Error::failed(Operation::WaitForLock));
    }

    if let Err(error) = set_locked_hint(true) {
        eprintln!("secure session lock warning ({:?})", error.operation);
    }
    if let Err(error) = notify_ready() {
        terminate(&mut locker);
        return Err(error);
    }

    let status = locker
        .wait()
        .map_err(|error| Error::io(Operation::WaitForUnlock, error))?;
    classify_locker_exit(status)?;
    if let Err(error) = set_locked_hint(false) {
        eprintln!("secure session unlock warning ({:?})", error.operation);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_config(config: &Path) -> Result<(), Error> {
    if !config.is_absolute() {
        return Err(Error {
            operation: Operation::ValidateConfig,
            kind: io::ErrorKind::InvalidInput,
        });
    }
    let metadata =
        std::fs::metadata(config).map_err(|error| Error::io(Operation::ReadConfig, error))?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return Err(Error {
            operation: Operation::ValidateConfig,
            kind: io::ErrorKind::InvalidData,
        });
    }
    let contents =
        std::fs::read_to_string(config).map_err(|error| Error::io(Operation::ReadConfig, error))?;
    validate_config_contents(&contents)
}

#[cfg(any(target_os = "linux", test))]
fn validate_config_contents(contents: &str) -> Result<(), Error> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_once('=').map_or(line, |(key, _)| key).trim();
        if matches!(key, "daemonize" | "ready-fd" | "config") {
            return Err(Error {
                operation: Operation::ValidateConfig,
                kind: io::ErrorKind::InvalidData,
            });
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn supervise(_config: &Path) -> Result<(), Error> {
    Err(Error {
        operation: Operation::SpawnLocker,
        kind: io::ErrorKind::Unsupported,
    })
}

#[cfg(target_os = "linux")]
fn wait_for_ready(locker: &mut Child) -> Result<bool, Error> {
    let mut stdout = locker.stdout.take().ok_or(Error {
        operation: Operation::WaitForLock,
        kind: io::ErrorKind::BrokenPipe,
    })?;
    let mut byte = [0_u8; 1];
    match stdout.read_exact(&mut byte) {
        Ok(()) if byte == [b'\n'] => Ok(true),
        Ok(()) => {
            terminate(locker);
            Ok(false)
        }
        Err(error) => {
            terminate(locker);
            Err(Error::io(Operation::WaitForLock, error))
        }
    }
}

#[cfg(target_os = "linux")]
fn notify_ready() -> Result<(), Error> {
    let status = Command::new(SYSTEMD_NOTIFY)
        .args([
            "--ready",
            "--pid=parent",
            "--status=rmac session is securely locked",
        ])
        .status()
        .map_err(|error| Error::io(Operation::NotifySupervisor, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::failed(Operation::NotifySupervisor))
    }
}

#[cfg(any(target_os = "linux", test))]
fn classify_locker_exit(status: ExitStatus) -> Result<(), Error> {
    if status.success() {
        Ok(())
    } else {
        Err(Error::failed(Operation::WaitForUnlock))
    }
}

#[cfg(target_os = "linux")]
fn terminate(locker: &mut Child) {
    let _ = locker.kill();
    let _ = locker.wait();
}

#[cfg(target_os = "linux")]
fn set_locked_hint(locked: bool) -> Result<(), Error> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))?;
    let proxy = LoginSessionProxyBlocking::new(&connection)
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))?;
    proxy
        .set_locked_hint(locked)
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/auto"
)]
trait LoginSession {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    #[test]
    fn only_successful_locker_exit_means_authenticated_unlock() {
        assert_eq!(classify_locker_exit(ExitStatus::from_raw(0)), Ok(()));
        assert_eq!(
            classify_locker_exit(ExitStatus::from_raw(1 << 8)),
            Err(Error::failed(Operation::WaitForUnlock))
        );
        assert_eq!(
            classify_locker_exit(ExitStatus::from_raw(9)),
            Err(Error::failed(Operation::WaitForUnlock))
        );
    }

    #[test]
    fn errors_are_bounded_and_do_not_expose_paths_or_commands() {
        let error = Error::io(
            Operation::SpawnLocker,
            io::Error::new(io::ErrorKind::PermissionDenied, "/private/path"),
        );
        let display = error.to_string();
        assert!(!display.contains("/private/path"));
        assert!(!display.contains(SWAYLOCK));
    }

    #[test]
    fn config_cannot_detach_or_replace_the_readiness_channel() {
        assert_eq!(
            validate_config_contents("color=1c1c1e\nfont=Inter\n"),
            Ok(())
        );
        for unsafe_config in [
            "daemonize\n",
            "daemonize=true\n",
            "ready-fd=9\n",
            "config=/tmp/other\n",
        ] {
            assert_eq!(
                validate_config_contents(unsafe_config),
                Err(Error {
                    operation: Operation::ValidateConfig,
                    kind: io::ErrorKind::InvalidData,
                })
            );
        }
    }
}
