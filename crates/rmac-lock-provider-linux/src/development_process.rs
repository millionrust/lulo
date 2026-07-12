//! Feature-gated Linux process wrapper for recovery testing.
//!
//! This module is not part of the installed session. It exists so the custom
//! provider can enter the dedicated nested-compositor and reference-PC evidence
//! gates without weakening the accepted swaylock fallback.

use std::env;
use std::fmt;
use std::process::{Command, Stdio};
use std::time::Duration;

use zbus::zvariant::OwnedObjectPath;

use crate::process::{execute_actions, ProcessEffects, ProcessLifecycle, ProcessTermination};
use crate::runtime::linux::LinuxRuntime;
use crate::runtime::ProviderExit;

const SYSTEMD_NOTIFY: &str = "/usr/bin/systemd-notify";
const MAX_SESSION_ID_BYTES: usize = 256;
const POLL_WAIT: Duration = Duration::from_secs(1);

/// Run the opt-in custom provider until authenticated unlock or a fail-closed
/// restart condition.
pub fn run() -> Result<(), Error> {
    let session = SessionContext::connect()?;
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

        let executed = execute_actions(actions, &mut SessionEffects(&session))
            .map_err(|_| Error::new(Operation::NotifySystemd))?;
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
            .filter(|value| !value.is_empty() && value.len() <= MAX_SESSION_ID_BYTES)
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
        if session_uid != effective_uid {
            return Err(Error::new(Operation::SessionOwnership));
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
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolveSession,
    SessionOwnership,
    ResolveUsername,
    ConnectProvider,
    PollProvider,
    ApplyLifecycle,
    NotifySystemd,
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
}
