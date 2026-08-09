//! Fail-closed Linux process wrapper for the installed lock provider.

use std::env;
use std::fmt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zbus::zvariant::OwnedObjectPath;

use crate::process::{
    execute_actions, ProcessEffects, ProcessLifecycle, ProcessTermination, WatchdogSchedule,
};
use crate::runtime::linux::LinuxRuntime;
use crate::runtime::ProviderExit;

const SYSTEMD_NOTIFY: &str = "/usr/bin/systemd-notify";
const MAX_SESSION_ID_BYTES: usize = 256;
const MAX_SEAT_NAME_BYTES: usize = 256;
const WAYLAND_SESSION_TYPE: &str = "wayland";
const POLL_WAIT: Duration = Duration::from_secs(1);

/// Run the provider until authenticated unlock or a fail-closed restart
/// condition.
pub fn run() -> Result<(), Error> {
    let session = SessionContext::connect()?;
    let watchdog_usec = env::var("WATCHDOG_USEC").ok();
    let watchdog_pid = env::var("WATCHDOG_PID").ok();
    let mut watchdog = WatchdogSchedule::from_environment_values(
        watchdog_usec.as_deref(),
        watchdog_pid.as_deref(),
        std::process::id(),
    )
    .map_err(|_| Error::new(Operation::ResolveWatchdog))?;
    let mut runtime = LinuxRuntime::connect(session.username.clone())
        .map_err(|_| Error::new(Operation::ConnectProvider))?;
    let mut lifecycle = ProcessLifecycle::default();

    loop {
        let status = runtime
            .poll(POLL_WAIT)
            .map_err(|_| Error::new(Operation::PollProvider))?;
        let exit = status.exit;
        let actions = lifecycle
            .apply(status.notify_ready, exit)
            .map_err(|_| Error::new(Operation::ApplyLifecycle))?;
        let became_ready = actions.notify_systemd_ready;

        let executed = execute_actions(actions, &mut SessionEffects(&session))
            .map_err(|_| Error::new(Operation::NotifySystemd))?;
        let now = Instant::now();
        if became_ready {
            watchdog
                .arm(now)
                .map_err(|_| Error::new(Operation::ResolveWatchdog))?;
        }
        if watchdog
            .heartbeat_due(now)
            .map_err(|_| Error::new(Operation::ResolveWatchdog))?
        {
            notify_watchdog()?;
        }
        if executed.locked_hint_failed {
            eprintln!("secure lock advisory state warning");
        }

        match executed.termination {
            ProcessTermination::Continue => {}
            ProcessTermination::AuthenticatedUnlock => return Ok(()),
            ProcessTermination::RestartRequired => {
                return Err(Error::new(match exit {
                    Some(ProviderExit::Denied) => Operation::ProviderDenied,
                    Some(ProviderExit::FailedLocked) => Operation::ProviderFailedLocked,
                    _ => Operation::ApplyLifecycle,
                }));
            }
        }
    }
}

struct SessionEffects<'a>(&'a SessionContext);

impl ProcessEffects for SessionEffects<'_> {
    fn set_locked_hint(&mut self, locked: bool) -> Result<(), ()> {
        self.0.set_locked_hint(locked).map_err(|_| ())
    }

    fn notify_ready(&mut self) -> Result<(), ()> {
        notify_ready().map_err(|_| ())
    }
}

struct SessionContext {
    connection: zbus::blocking::Connection,
    path: OwnedObjectPath,
    username: String,
}

impl SessionContext {
    fn connect() -> Result<Self, Error> {
        let session_id = env::var("XDG_SESSION_ID")
            .ok()
            .filter(|value| valid_session_id(value))
            .ok_or_else(|| Error::new(Operation::ResolveSession))?;
        let connection = zbus::blocking::Connection::system()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let manager = LoginManagerProxyBlocking::new(&connection)
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let path = manager
            .get_session(&session_id)
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let session = LoginSessionProxyBlocking::builder(&connection)
            .path(path.clone())
            .map_err(|_| Error::new(Operation::ResolveSession))?
            .build()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let (session_uid, _) = session
            .user()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        // SAFETY: `geteuid` has no pointer arguments and cannot violate Rust
        // memory invariants.
        let effective_uid = unsafe { libc::geteuid() };
        let remote = session
            .remote()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let session_type = session
            .session_type()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        let (seat, _) = session
            .seat()
            .map_err(|_| Error::new(Operation::ResolveSession))?;
        if !valid_session_identity(effective_uid, session_uid, remote, &session_type, &seat) {
            return Err(Error::new(Operation::SessionIdentity));
        }
        let username = session
            .name()
            .map_err(|_| Error::new(Operation::ResolveUsername))?;
        if !crate::runtime::valid_username(&username) {
            return Err(Error::new(Operation::ResolveUsername));
        }
        drop(session);

        Ok(Self {
            connection,
            path,
            username,
        })
    }

    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()> {
        LoginSessionProxyBlocking::builder(&self.connection)
            .path(self.path.clone())?
            .build()?
            .set_locked_hint(locked)
    }
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_ID_BYTES
        && !value.chars().any(char::is_control)
        && !value.chars().any(char::is_whitespace)
}

fn valid_session_identity(
    expected_uid: u32,
    session_uid: u32,
    remote: bool,
    session_type: &str,
    seat: &str,
) -> bool {
    expected_uid == session_uid
        && !remote
        && session_type == WAYLAND_SESSION_TYPE
        && !seat.is_empty()
        && seat.len() <= MAX_SEAT_NAME_BYTES
        && !seat.chars().any(char::is_control)
        && !seat.chars().any(char::is_whitespace)
}

fn notify_ready() -> Result<(), Error> {
    let status = Command::new(SYSTEMD_NOTIFY)
        .args([
            "--ready",
            "--pid=parent",
            "--status=rmac custom lock provider is securely locked",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|_| Error::new(Operation::NotifySystemd))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::new(Operation::NotifySystemd))
    }
}

fn notify_watchdog() -> Result<(), Error> {
    let status = Command::new(SYSTEMD_NOTIFY)
        .args(["--watchdog", "--pid=parent"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|_| Error::new(Operation::NotifyWatchdog))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::new(Operation::NotifyWatchdog))
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LoginSession {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn remote(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, OwnedObjectPath)>;

    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolveSession,
    SessionIdentity,
    ResolveUsername,
    ResolveWatchdog,
    ConnectProvider,
    PollProvider,
    ApplyLifecycle,
    NotifySystemd,
    NotifyWatchdog,
    ProviderDenied,
    ProviderFailedLocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    operation: Operation,
}

impl Error {
    fn new(operation: Operation) -> Self {
        Self { operation }
    }

    pub fn operation(self) -> Operation {
        self.operation
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("rmac custom lock provider failed")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_do_not_expose_session_or_username_data() {
        let error = Error::new(Operation::ResolveSession);
        assert_eq!(error.operation(), Operation::ResolveSession);
        assert_eq!(error.to_string(), "rmac custom lock provider failed");
        assert!(!format!("{error:?}").contains("XDG_SESSION_ID"));
    }

    #[test]
    fn routed_session_must_be_bounded_local_seated_wayland_and_same_uid() {
        assert!(valid_session_id("c2"));
        assert!(!valid_session_id("session 2"));
        assert!(!valid_session_id("session\n2"));
        assert!(!valid_session_id(&"x".repeat(MAX_SESSION_ID_BYTES + 1)));
        assert!(valid_session_identity(
            1_000, 1_000, false, "wayland", "seat0"
        ));
        for (uid, remote, session_type, seat) in [
            (1_001, false, "wayland", "seat0"),
            (1_000, true, "wayland", "seat0"),
            (1_000, false, "x11", "seat0"),
            (1_000, false, "wayland", ""),
            (1_000, false, "wayland", "seat 0"),
        ] {
            assert!(!valid_session_identity(
                1_000,
                uid,
                remote,
                session_type,
                seat
            ));
        }
    }
}
