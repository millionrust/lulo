//! Security boundary for starting and supervising the session locker.

use std::fmt;
use std::io;
use std::path::Path;
#[cfg(any(target_os = "linux", test))]
use std::process::ExitStatus;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use std::io::Read as _;
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "linux")]
const SYSTEMCTL: &str = "/usr/bin/systemctl";
#[cfg(target_os = "linux")]
const SYSTEMD_NOTIFY: &str = "/usr/bin/systemd-notify";
#[cfg(target_os = "linux")]
const SWAYIDLE: &str = "/usr/bin/swayidle";
#[cfg(any(target_os = "linux", test))]
const SWAYLOCK: &str = "/usr/bin/swaylock";
#[cfg(target_os = "linux")]
const LOCK_UNIT: &str = "rmac-lock.service";
#[cfg(target_os = "linux")]
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
#[cfg(any(target_os = "linux", test))]
const LOCK_ACTION: &str = "/usr/bin/systemctl --user start rmac-lock.service";
#[cfg(any(target_os = "linux", test))]
const SUSPEND_ACTION: &str = "/usr/bin/busctl --user call org.rmac.LockScreen1 /org/rmac/LockScreen1 org.rmac.LockScreen1 RequestSuspend";
#[cfg(target_os = "linux")]
const MAX_IDLE_POLICY_BYTES: u64 = 16 * 1024;
const MIN_IDLE_SECONDS: u32 = 60;
const MAX_IDLE_SECONDS: u32 = 24 * 60 * 60;
const MIN_SUSPEND_SECONDS: u32 = 5 * 60;
#[cfg(any(target_os = "linux", test))]
const MAX_SESSION_ID_BYTES: usize = 256;
#[cfg(any(target_os = "linux", test))]
const MAX_SEAT_NAME_BYTES: usize = 256;
#[cfg(any(target_os = "linux", test))]
const WAYLAND_SESSION_TYPE: &str = "wayland";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IdlePolicy {
    pub version: u32,
    pub lock_after_seconds: Option<u32>,
    #[serde(default)]
    pub suspend_after_seconds: Option<u32>,
}

impl Default for IdlePolicy {
    fn default() -> Self {
        Self {
            version: 1,
            lock_after_seconds: Some(5 * 60),
            suspend_after_seconds: None,
        }
    }
}

impl IdlePolicy {
    pub fn validate(self) -> Result<Self, Error> {
        if self.version != 1
            || self
                .lock_after_seconds
                .is_some_and(|seconds| !(MIN_IDLE_SECONDS..=MAX_IDLE_SECONDS).contains(&seconds))
            || self
                .suspend_after_seconds
                .is_some_and(|seconds| !(MIN_SUSPEND_SECONDS..=MAX_IDLE_SECONDS).contains(&seconds))
        {
            return Err(Error {
                operation: Operation::ValidateIdlePolicy,
                kind: io::ErrorKind::InvalidData,
            });
        }
        Ok(self)
    }
}

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
    ConnectLogind,
    ResolveSession,
    ValidateSession,
    InhibitSleep,
    SubscribeLogind,
    ReadSignal,
    ReadIdlePolicy,
    ValidateIdlePolicy,
    SpawnIdleManager,
    WaitForIdleManager,
    ServeSettings,
    SaveIdlePolicy,
    RestartIdleManager,
    InhibitPowerKey,
    WatchPowerKey,
    Suspend,
    ShowShutdownDialog,
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
pub fn supervise_idle(policy_path: &Path) -> Result<(), Error> {
    let policy = read_idle_policy(policy_path)?;
    if policy.lock_after_seconds.is_none() && policy.suspend_after_seconds.is_none() {
        loop {
            std::thread::park();
        }
    }
    let arguments = idle_arguments(policy);
    let status = Command::new(SWAYIDLE)
        .args(arguments)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| Error::io(Operation::SpawnIdleManager, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::failed(Operation::WaitForIdleManager))
    }
}

#[cfg(any(target_os = "linux", test))]
fn idle_arguments(policy: IdlePolicy) -> Vec<String> {
    let mut arguments = vec!["-w".to_owned()];
    if let Some(timeout) = policy.lock_after_seconds {
        arguments.extend([
            "timeout".to_owned(),
            timeout.to_string(),
            LOCK_ACTION.to_owned(),
        ]);
    }
    if let Some(timeout) = policy.suspend_after_seconds {
        arguments.extend([
            "timeout".to_owned(),
            timeout.to_string(),
            SUSPEND_ACTION.to_owned(),
        ]);
    }
    arguments
}

#[cfg(not(target_os = "linux"))]
pub fn supervise_idle(_policy_path: &Path) -> Result<(), Error> {
    Err(Error {
        operation: Operation::SpawnIdleManager,
        kind: io::ErrorKind::Unsupported,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn read_idle_policy(policy_path: &Path) -> Result<IdlePolicy, Error> {
    if !policy_path.is_absolute() {
        return Err(Error {
            operation: Operation::ValidateIdlePolicy,
            kind: io::ErrorKind::InvalidInput,
        });
    }
    let file = std::fs::File::open(policy_path)
        .map_err(|error| Error::io(Operation::ReadIdlePolicy, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| Error::io(Operation::ReadIdlePolicy, error))?;
    if !metadata.is_file() || metadata.len() > MAX_IDLE_POLICY_BYTES {
        return Err(Error {
            operation: Operation::ValidateIdlePolicy,
            kind: io::ErrorKind::InvalidData,
        });
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_IDLE_POLICY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(Operation::ReadIdlePolicy, error))?;
    if bytes.len() as u64 > MAX_IDLE_POLICY_BYTES {
        return Err(Error {
            operation: Operation::ValidateIdlePolicy,
            kind: io::ErrorKind::InvalidData,
        });
    }
    serde_json::from_slice::<IdlePolicy>(&bytes)
        .map_err(|_| Error::failed(Operation::ValidateIdlePolicy))?
        .validate()
}

#[cfg(target_os = "linux")]
pub(crate) fn write_idle_policy(policy_path: &Path, policy: IdlePolicy) -> Result<(), Error> {
    let policy = policy.validate()?;
    if !policy_path.is_absolute() {
        return Err(Error {
            operation: Operation::ValidateIdlePolicy,
            kind: io::ErrorKind::InvalidInput,
        });
    }
    let mut bytes =
        serde_json::to_vec_pretty(&policy).map_err(|_| Error::failed(Operation::SaveIdlePolicy))?;
    bytes.push(b'\n');
    rmac_storage::atomic_write_private(policy_path, &bytes)
        .map_err(|error| Error::io(Operation::SaveIdlePolicy, error))
}

#[cfg(target_os = "linux")]
pub(crate) fn restart_idle_manager() -> Result<(), Error> {
    let status = Command::new(SYSTEMCTL)
        .args(["--user", "restart", "rmac-idle-lock.service"])
        .status()
        .map_err(|error| Error::io(Operation::RestartIdleManager, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::failed(Operation::RestartIdleManager))
    }
}

#[cfg(target_os = "linux")]
pub async fn coordinate(policy_path: &Path) -> Result<(), Error> {
    use futures_util::StreamExt as _;

    let connection = zbus::Connection::system()
        .await
        .map_err(|_| Error::failed(Operation::ConnectLogind))?;
    let manager = LoginManagerProxy::new(&connection)
        .await
        .map_err(|_| Error::failed(Operation::ConnectLogind))?;
    let mut sleep_inhibitor = Some(acquire_sleep_inhibitor(&manager).await?);
    let session_id = session_id_from_environment()?;
    let session_path = manager
        .get_session(&session_id)
        .await
        .map_err(|_| Error::failed(Operation::ResolveSession))?;
    let session = LoginSessionProxy::builder(&connection)
        .path(session_path)
        .map_err(|_| Error::failed(Operation::ResolveSession))?
        .build()
        .await
        .map_err(|_| Error::failed(Operation::ResolveSession))?;
    validate_async_session(&session).await?;
    let lock_requests = session
        .receive_lock()
        .await
        .map_err(|_| Error::failed(Operation::SubscribeLogind))?;
    let sleep_changes = manager
        .receive_prepare_for_sleep()
        .await
        .map_err(|_| Error::failed(Operation::SubscribeLogind))?;
    let _settings = crate::lock_settings::serve(policy_path)
        .await
        .map_err(|_| Error::failed(Operation::ServeSettings))?;

    // The power button (docs/decisions/0020): listen for niri's presses
    // first, and only then ask logind to leave the button to us, so a press
    // is never swallowed while nothing can hear it.
    let (press_sender, presses) = async_channel::bounded(8);
    let (bound_sender, bound) = async_channel::bounded(1);
    let press_watch = futures_util::FutureExt::fuse(crate::watch_dispatches_ready(
        crate::ShortcutId(crate::power_key::POWER_KEY_SHORTCUT.into()),
        press_sender,
        bound_sender,
    ));
    futures_util::pin_mut!(press_watch);
    let mut power_key_inhibitor = None;
    {
        let bound = futures_util::FutureExt::fuse(bound.recv());
        futures_util::pin_mut!(bound);
        futures_util::select! {
            ready = bound => {
                if ready.is_ok() {
                    power_key_inhibitor = acquire_power_key_inhibitor(&manager).await;
                }
            },
            result = press_watch => report_power_key_watch_end(result),
        }
    }
    let mut power_key = crate::power_key::PowerKey::default();

    notify_systemd("Lulo OS lock coordinator is ready")?;
    futures_util::pin_mut!(lock_requests, sleep_changes);

    loop {
        let lock_request = futures_util::FutureExt::fuse(lock_requests.next());
        let sleep_change = futures_util::FutureExt::fuse(sleep_changes.next());
        // Once the listener has gone, `recv` would fail at once on every
        // turn; leave it out instead of spinning.
        let press = if presses.is_closed() {
            futures_util::future::Fuse::terminated()
        } else {
            futures_util::FutureExt::fuse(presses.recv())
        };
        let deadline = futures_util::FutureExt::fuse(match power_key.deadline() {
            Some(at) => async_io::Timer::at(at),
            None => async_io::Timer::never(),
        });
        futures_util::pin_mut!(lock_request, sleep_change, press, deadline);
        futures_util::select! {
            event = press => {
                if let Ok(crate::Event::Activated { .. }) = event {
                    let locked = session_locked(&session).await;
                    let action = power_key.press(std::time::Instant::now(), locked);
                    act_on_power_key(action, &manager).await;
                }
            },
            _ = deadline => {
                let locked = session_locked(&session).await;
                let action = power_key.deadline_passed(std::time::Instant::now(), locked);
                act_on_power_key(action, &manager).await;
            },
            result = press_watch => {
                // Nothing hears the button any more: hand it back to logind
                // rather than leave it dead.
                power_key_inhibitor.take();
                report_power_key_watch_end(result);
            },
            request_signal = lock_request => {
                if request_signal.is_none() {
                    return Err(Error::failed(Operation::ReadSignal));
                }
                if let Err(error) = request() {
                    eprintln!("secure session lock request warning ({:?})", error.operation);
                }
            },
            sleep_signal = sleep_change => {
                let signal = sleep_signal.ok_or_else(|| Error::failed(Operation::ReadSignal))?;
                let preparing = *signal
                    .args()
                    .map_err(|_| Error::failed(Operation::ReadSignal))?
                    .start();
                if preparing {
                    power_key.sleeping();
                    match request() {
                        Ok(()) => {
                            sleep_inhibitor.take();
                        }
                        Err(error) => {
                            eprintln!("secure pre-sleep lock warning ({:?})", error.operation);
                        }
                    }
                } else {
                    power_key.woke(std::time::Instant::now());
                    sleep_inhibitor = Some(acquire_sleep_inhibitor(&manager).await?);
                }
            },
        }
    }
}

/// Whether the session shows the lock screen now. An unreadable hint counts
/// as unlocked, so a press still locks before it sleeps.
#[cfg(target_os = "linux")]
async fn session_locked(session: &LoginSessionProxy<'_>) -> bool {
    session.locked_hint().await.unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn act_on_power_key(
    action: crate::power_key::PowerKeyAction,
    manager: &LoginManagerProxy<'_>,
) {
    use crate::power_key::PowerKeyAction;

    match action {
        PowerKeyAction::Ignore | PowerKeyAction::WaitUntil(_) => {}
        PowerKeyAction::ShowShutdownDialog => {
            let dialog = crate::ShortcutId(crate::power_key::SHUTDOWN_DIALOG_SHORTCUT.into());
            if let Err(error) = crate::dispatch(&dialog) {
                eprintln!(
                    "the power button could not show the shutdown dialog ({:?}): {error}; \
                     choose Shut Down in the Lulo OS menu instead",
                    Operation::ShowShutdownDialog
                );
            }
        }
        PowerKeyAction::Sleep { lock_first } => {
            // Lock first, so the screen that wakes is the lock screen.
            // `request` returns once the locker reports it is up.
            if lock_first {
                if let Err(error) = request() {
                    eprintln!(
                        "the power button could not lock before sleeping ({:?}); sleeping \
                         anyway, and the pre-sleep lock tries again",
                        error.operation
                    );
                }
            }
            if let Err(error) = manager.suspend(false).await {
                eprintln!(
                    "the power button could not put the computer to sleep ({:?}): {error}",
                    Operation::Suspend
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn report_power_key_watch_end(result: Result<(), crate::Error>) {
    match result {
        Ok(()) => eprintln!(
            "the power button listener stopped ({:?}); the button now does what logind is \
             configured to do",
            Operation::WatchPowerKey
        ),
        Err(error) => eprintln!(
            "the power button listener failed ({:?}): {error}; the button now does what logind \
             is configured to do",
            Operation::WatchPowerKey
        ),
    }
}

/// Take logind's `handle-power-key` block inhibitor, as GNOME does, so a
/// press reaches Lulo OS instead of powering off. Without it logind's own
/// action stays in place; that is reported but not fatal, since locking and
/// sleeping still work.
#[cfg(target_os = "linux")]
async fn acquire_power_key_inhibitor(
    manager: &LoginManagerProxy<'_>,
) -> Option<zbus::zvariant::OwnedFd> {
    match manager
        .inhibit(
            crate::power_key::INHIBIT_WHAT,
            "Lulo OS",
            crate::power_key::INHIBIT_WHY,
            "block",
        )
        .await
    {
        Ok(fd) => Some(fd),
        Err(error) => {
            eprintln!(
                "could not take over the power button ({:?}): {error}; a press does what logind \
                 is configured to do, which on Ubuntu is to shut down",
                Operation::InhibitPowerKey
            );
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn coordinate(_policy_path: &Path) -> Result<(), Error> {
    Err(Error {
        operation: Operation::ConnectLogind,
        kind: io::ErrorKind::Unsupported,
    })
}

#[cfg(target_os = "linux")]
async fn acquire_sleep_inhibitor(
    manager: &LoginManagerProxy<'_>,
) -> Result<zbus::zvariant::OwnedFd, Error> {
    manager
        .inhibit("sleep", "Lulo OS", "lock the session before sleep", "delay")
        .await
        .map_err(|_| Error::failed(Operation::InhibitSleep))
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
    notify_systemd("Lulo OS session is securely locked")
}

#[cfg(target_os = "linux")]
fn notify_systemd(status: &str) -> Result<(), Error> {
    let status_argument = format!("--status={status}");
    let status = Command::new(SYSTEMD_NOTIFY)
        .args(["--ready", "--pid=parent", status_argument.as_str()])
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
    let session_id = session_id_from_environment()?;
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))?;
    let manager = LoginManagerProxyBlocking::new(&connection)
        .map_err(|error| Error::io(Operation::ResolveSession, io::Error::other(error)))?;
    let path = manager
        .get_session(&session_id)
        .map_err(|error| Error::io(Operation::ResolveSession, io::Error::other(error)))?;
    let proxy = LoginSessionProxyBlocking::builder(&connection)
        .path(path)
        .map_err(|error| Error::io(Operation::ResolveSession, io::Error::other(error)))?
        .build()
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))?;
    validate_blocking_session(&proxy)?;
    proxy
        .set_locked_hint(locked)
        .map_err(|error| Error::io(Operation::UpdateLockedHint, io::Error::other(error)))
}

#[cfg(any(target_os = "linux", test))]
fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_ID_BYTES
        && !value.chars().any(char::is_control)
        && !value.chars().any(char::is_whitespace)
}

#[cfg(any(target_os = "linux", test))]
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

#[cfg(target_os = "linux")]
fn session_id_from_environment() -> Result<String, Error> {
    std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|session_id| valid_session_id(session_id))
        .ok_or_else(|| Error::failed(Operation::ResolveSession))
}

#[cfg(target_os = "linux")]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no arguments and cannot violate Rust memory
    // invariants.
    unsafe { libc::geteuid() }
}

#[cfg(target_os = "linux")]
async fn validate_async_session(session: &LoginSessionProxy<'_>) -> Result<(), Error> {
    let (session_uid, _) = session
        .user()
        .await
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let remote = session
        .remote()
        .await
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let session_type = session
        .session_type()
        .await
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let (seat, _) = session
        .seat()
        .await
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    if valid_session_identity(effective_uid(), session_uid, remote, &session_type, &seat) {
        Ok(())
    } else {
        Err(Error::failed(Operation::ValidateSession))
    }
}

#[cfg(target_os = "linux")]
fn validate_blocking_session(session: &LoginSessionProxyBlocking<'_>) -> Result<(), Error> {
    let (session_uid, _) = session
        .user()
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let remote = session
        .remote()
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let session_type = session
        .session_type()
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    let (seat, _) = session
        .seat()
        .map_err(|_| Error::failed(Operation::ValidateSession))?;
    if valid_session_identity(effective_uid(), session_uid, remote, &session_type, &seat) {
        Ok(())
    } else {
        Err(Error::failed(Operation::ValidateSession))
    }
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LoginSession {
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn remote(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, zbus::zvariant::OwnedObjectPath)>;

    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, zbus::zvariant::OwnedObjectPath)>;
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn get_session(&self, session_id: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    /// Manual check on a Lulo OS session: holds the power-button inhibitor
    /// for `RMAC_POWER_KEY_HOLD_SECONDS` (default 5) so `systemd-inhibit
    /// --list` can show it. Never presses the button. polkit grants
    /// `inhibit-handle-power-key` only to local sessions, so run it as a
    /// user service (`systemd-run --user`), like the coordinator, not over
    /// SSH, where logind answers AccessDenied.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a logind session; run by hand"]
    fn power_key_inhibitor_is_a_block_on_handle_power_key() {
        let seconds = std::env::var("RMAC_POWER_KEY_HOLD_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);
        async_io::block_on(async {
            let connection = zbus::Connection::system().await.unwrap();
            let manager = LoginManagerProxy::new(&connection).await.unwrap();
            let held = acquire_power_key_inhibitor(&manager).await;
            assert!(held.is_some(), "logind refused the inhibitor");
            println!("holding handle-power-key for {seconds} s");
            async_io::Timer::after(std::time::Duration::from_secs(seconds)).await;
        });
    }

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

    #[test]
    fn idle_policy_has_bounded_versioned_timeouts() {
        assert_eq!(IdlePolicy::default().validate(), Ok(IdlePolicy::default()));
        assert_eq!(
            IdlePolicy {
                version: 1,
                lock_after_seconds: None,
                suspend_after_seconds: None,
            }
            .validate(),
            Ok(IdlePolicy {
                version: 1,
                lock_after_seconds: None,
                suspend_after_seconds: None,
            })
        );
        for policy in [
            IdlePolicy {
                version: 0,
                lock_after_seconds: Some(300),
                suspend_after_seconds: None,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(MIN_IDLE_SECONDS - 1),
                suspend_after_seconds: None,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(MAX_IDLE_SECONDS + 1),
                suspend_after_seconds: None,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(300),
                suspend_after_seconds: Some(MIN_SUSPEND_SECONDS - 1),
            },
        ] {
            assert_eq!(
                policy.validate(),
                Err(Error {
                    operation: Operation::ValidateIdlePolicy,
                    kind: io::ErrorKind::InvalidData,
                })
            );
        }
    }

    #[test]
    fn idle_policy_rejects_unrecognized_authority() {
        assert!(serde_json::from_str::<IdlePolicy>(
            r#"{"version":1,"lock_after_seconds":300,"command":"other"}"#
        )
        .is_err());
    }

    #[test]
    fn legacy_idle_policy_defaults_to_never_suspend() {
        let policy: IdlePolicy =
            serde_json::from_str(r#"{"version":1,"lock_after_seconds":300}"#).unwrap();
        assert_eq!(policy.lock_after_seconds, Some(300));
        assert_eq!(policy.suspend_after_seconds, None);
        assert_eq!(policy.validate(), Ok(policy));
    }

    #[test]
    fn idle_commands_are_fixed_and_timeouts_are_numeric_arguments() {
        assert_eq!(
            idle_arguments(IdlePolicy {
                version: 1,
                lock_after_seconds: Some(60),
                suspend_after_seconds: Some(900),
            }),
            vec![
                "-w".to_owned(),
                "timeout".to_owned(),
                "60".to_owned(),
                LOCK_ACTION.to_owned(),
                "timeout".to_owned(),
                "900".to_owned(),
                SUSPEND_ACTION.to_owned(),
            ]
        );
    }

    #[test]
    fn session_route_is_bounded_and_never_accepts_control_or_whitespace() {
        assert!(valid_session_id("c2"));
        assert!(valid_session_id("wayland-session_42"));
        for invalid in [
            "",
            "session with spaces",
            "session\n2",
            &"x".repeat(MAX_SESSION_ID_BYTES + 1),
        ] {
            assert!(!valid_session_id(invalid));
        }
    }

    #[test]
    fn only_the_same_users_local_seated_wayland_session_is_accepted() {
        assert!(valid_session_identity(
            1_000, 1_000, false, "wayland", "seat0"
        ));
        for (uid, remote, session_type, seat) in [
            (1_001, false, "wayland", "seat0"),
            (1_000, true, "wayland", "seat0"),
            (1_000, false, "x11", "seat0"),
            (1_000, false, "tty", "seat0"),
            (1_000, false, "wayland", ""),
            (1_000, false, "wayland", "seat 0"),
            (1_000, false, "wayland", "seat\n0"),
        ] {
            assert!(!valid_session_identity(
                1_000,
                uid,
                remote,
                session_type,
                seat
            ));
        }
        assert!(!valid_session_identity(
            1_000,
            1_000,
            false,
            "wayland",
            &"x".repeat(MAX_SEAT_NAME_BYTES + 1)
        ));
    }
}
