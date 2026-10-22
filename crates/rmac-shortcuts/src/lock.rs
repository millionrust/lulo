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
    /// Lock Screen ▸ Turn display off on battery when inactive.
    #[serde(default)]
    pub display_off_on_battery_seconds: Option<u32>,
    /// Lock Screen ▸ Turn display off on power adapter when inactive.
    #[serde(default)]
    pub display_off_on_adapter_seconds: Option<u32>,
    /// Lock Screen ▸ Require password after the display is turned off:
    /// `Some(0)` is Immediately, `None` is Never.
    #[serde(default = "default_require_password")]
    pub require_password_after_seconds: Option<u32>,
    /// Battery ▸ Options… ▸ Prevent automatic sleeping on power adapter
    /// when the display is off.
    #[serde(default)]
    pub prevent_sleep_on_adapter: bool,
}

fn default_require_password() -> Option<u32> {
    Some(0)
}

/// The Mac's longest "Require password" delay, 8 hours.
pub const MAX_REQUIRE_PASSWORD_SECONDS: u32 = 8 * 60 * 60;

impl Default for IdlePolicy {
    fn default() -> Self {
        Self {
            version: 1,
            lock_after_seconds: Some(5 * 60),
            suspend_after_seconds: None,
            display_off_on_battery_seconds: None,
            display_off_on_adapter_seconds: None,
            require_password_after_seconds: Some(0),
            prevent_sleep_on_adapter: false,
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
            || [
                self.display_off_on_battery_seconds,
                self.display_off_on_adapter_seconds,
            ]
            .into_iter()
            .flatten()
            .any(|seconds| !(MIN_IDLE_SECONDS..=MAX_IDLE_SECONDS).contains(&seconds))
            || self
                .require_password_after_seconds
                .is_some_and(|seconds| seconds > MAX_REQUIRE_PASSWORD_SECONDS)
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
    #[cfg(windows)]
    {
        crate::lock_windows::lock_workstation().map_err(|_| Error {
            operation: Operation::Request,
            kind: io::ErrorKind::Other,
        })
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Err(Error {
            operation: Operation::Request,
            kind: io::ErrorKind::Unsupported,
        })
    }
}

/// Run swayidle for the policy. When its timeouts differ on battery and on
/// the power adapter, a power-source change (a UPower event, not a poll)
/// restarts swayidle with the other ones.
#[cfg(target_os = "linux")]
pub fn supervise_idle(policy_path: &Path) -> Result<(), Error> {
    let policy = read_idle_policy(policy_path)?;
    let follows_power = idle_arguments(policy, true) != idle_arguments(policy, false);
    let (changes, power_events) = std::sync::mpsc::channel::<()>();
    if follows_power {
        std::thread::spawn(move || {
            let (sender, receiver) = async_channel::bounded(4);
            std::thread::spawn(move || {
                let _ = async_io::block_on(rmac_power::watch(sender));
            });
            while receiver.recv_blocking().is_ok() {
                if changes.send(()).is_err() {
                    return;
                }
            }
        });
    } else {
        drop(changes);
    }
    let mut on_battery = current_on_battery();
    loop {
        let arguments = idle_arguments(policy, on_battery);
        if arguments.len() == 1 {
            // Nothing to time out: wait for the power source to matter.
            if power_events.recv().is_err() {
                loop {
                    std::thread::park();
                }
            }
            on_battery = current_on_battery();
            continue;
        }
        let mut child = Command::new(SWAYIDLE)
            .args(arguments)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| Error::io(Operation::SpawnIdleManager, error))?;
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| Error::io(Operation::WaitForIdleManager, error))?
            {
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::failed(Operation::WaitForIdleManager))
                };
            }
            if !follows_power {
                let status = child
                    .wait()
                    .map_err(|error| Error::io(Operation::WaitForIdleManager, error))?;
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::failed(Operation::WaitForIdleManager))
                };
            }
            // A power event, or a quiet minute to notice swayidle exiting.
            match power_events.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(()) => {
                    let now = current_on_battery();
                    if now != on_battery {
                        on_battery = now;
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let status = child
                        .wait()
                        .map_err(|error| Error::io(Operation::WaitForIdleManager, error))?;
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(Error::failed(Operation::WaitForIdleManager))
                    };
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn current_on_battery() -> bool {
    rmac_power::snapshot()
        .ok()
        .and_then(|snapshot| snapshot.battery)
        .is_some_and(|battery| battery.on_battery)
}

#[cfg(any(target_os = "linux", test))]
const DISPLAY_OFF_ACTION: &str = "/usr/bin/niri msg action power-off-monitors";

#[cfg(any(target_os = "linux", test))]
fn idle_arguments(policy: IdlePolicy, on_battery: bool) -> Vec<String> {
    let mut arguments = vec!["-w".to_owned()];
    // niri powers the displays back on at the next input.
    let display_off = if on_battery {
        policy.display_off_on_battery_seconds
    } else {
        policy.display_off_on_adapter_seconds
    };
    if let Some(timeout) = display_off {
        arguments.extend([
            "timeout".to_owned(),
            timeout.to_string(),
            DISPLAY_OFF_ACTION.to_owned(),
        ]);
    }
    // The Mac's "Require password after … display is turned off": lock the
    // delay after the display goes off, or at "Lock screen when inactive"
    // if that comes first.
    let after_display_off = display_off
        .zip(policy.require_password_after_seconds)
        .map(|(off, delay)| off.saturating_add(delay));
    let lock = match (policy.lock_after_seconds, after_display_off) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if let Some(timeout) = lock {
        arguments.extend([
            "timeout".to_owned(),
            timeout.to_string(),
            LOCK_ACTION.to_owned(),
        ]);
    }
    let suspend = policy
        .suspend_after_seconds
        .filter(|_| on_battery || !policy.prevent_sleep_on_adapter);
    if let Some(timeout) = suspend {
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

    let connection = rmac_dbus::system()
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
    // Screen On Usage (Settings ▸ Battery) is built from these events; a
    // missing shutdown stream only loses that chart's precision.
    let shutdown_changes = manager.receive_prepare_for_shutdown().await.ok();
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
    record_screen(true);
    futures_util::pin_mut!(lock_requests, sleep_changes);
    let mut shutdown_changes = shutdown_changes.map(Box::pin);

    loop {
        let lock_request = futures_util::FutureExt::fuse(lock_requests.next());
        let sleep_change = futures_util::FutureExt::fuse(sleep_changes.next());
        let shutdown = match shutdown_changes.as_mut() {
            Some(stream) => {
                futures_util::FutureExt::fuse(futures_util::future::Either::Left(stream.next()))
            }
            None => futures_util::FutureExt::fuse(futures_util::future::Either::Right(
                futures_util::future::pending(),
            )),
        };
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
        futures_util::pin_mut!(lock_request, sleep_change, press, deadline, shutdown);
        let mut shutdown_ended = false;
        futures_util::select! {
            signal = shutdown => {
                match signal {
                    Some(signal) => {
                        if signal.args().is_ok_and(|args| *args.start()) {
                            record_screen(false);
                        }
                    }
                    // The stream ended: stop listening rather than spin.
                    None => shutdown_ended = true,
                }
            },
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
                record_screen(!preparing);
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
        if shutdown_ended {
            shutdown_changes = None;
        }
    }
}

/// Note for Settings ▸ Battery ▸ Screen On Usage that the screen came on
/// or went off. A failure only loses that chart's sample.
#[cfg(target_os = "linux")]
fn record_screen(on: bool) {
    if let Err(error) = rmac_power::screen_usage::record(on) {
        eprintln!("screen-on usage record warning: {error}");
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
    let connection = rmac_dbus::system_blocking()
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

    #[zbus(signal)]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;
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
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
            }
            .validate(),
            Ok(IdlePolicy {
                version: 1,
                lock_after_seconds: None,
                suspend_after_seconds: None,
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
            })
        );
        for policy in [
            IdlePolicy {
                version: 0,
                lock_after_seconds: Some(300),
                suspend_after_seconds: None,
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(MIN_IDLE_SECONDS - 1),
                suspend_after_seconds: None,
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(MAX_IDLE_SECONDS + 1),
                suspend_after_seconds: None,
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
            },
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(300),
                suspend_after_seconds: Some(MIN_SUSPEND_SECONDS - 1),
                display_off_on_battery_seconds: None,
                display_off_on_adapter_seconds: None,
                require_password_after_seconds: Some(0),
                prevent_sleep_on_adapter: false,
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
            idle_arguments(
                IdlePolicy {
                    version: 1,
                    lock_after_seconds: Some(60),
                    suspend_after_seconds: Some(900),
                    display_off_on_battery_seconds: None,
                    display_off_on_adapter_seconds: None,
                    require_password_after_seconds: Some(0),
                    prevent_sleep_on_adapter: false,
                },
                false
            ),
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
    fn display_off_follows_the_power_source() {
        let policy = IdlePolicy {
            lock_after_seconds: None,
            display_off_on_battery_seconds: Some(120),
            display_off_on_adapter_seconds: Some(600),
            require_password_after_seconds: None,
            prevent_sleep_on_adapter: false,
            ..IdlePolicy::default()
        };
        assert_eq!(
            idle_arguments(policy, true),
            ["-w", "timeout", "120", DISPLAY_OFF_ACTION]
        );
        assert_eq!(
            idle_arguments(policy, false),
            ["-w", "timeout", "600", DISPLAY_OFF_ACTION]
        );
    }

    #[test]
    fn require_password_locks_after_the_display_goes_off() {
        let policy = IdlePolicy {
            lock_after_seconds: Some(900),
            display_off_on_battery_seconds: Some(120),
            display_off_on_adapter_seconds: Some(600),
            require_password_after_seconds: Some(60),
            ..IdlePolicy::default()
        };
        assert_eq!(
            idle_arguments(policy, true),
            [
                "-w",
                "timeout",
                "120",
                DISPLAY_OFF_ACTION,
                "timeout",
                "180",
                LOCK_ACTION
            ]
        );
        // Lock screen when inactive still wins when it comes first.
        let policy = IdlePolicy {
            display_off_on_adapter_seconds: Some(1200),
            ..policy
        };
        assert_eq!(
            idle_arguments(policy, false),
            [
                "-w",
                "timeout",
                "1200",
                DISPLAY_OFF_ACTION,
                "timeout",
                "900",
                LOCK_ACTION
            ]
        );
        let never = IdlePolicy {
            lock_after_seconds: None,
            require_password_after_seconds: None,
            ..policy
        };
        assert_eq!(
            idle_arguments(never, false),
            ["-w", "timeout", "1200", DISPLAY_OFF_ACTION]
        );
    }

    #[test]
    fn sleep_can_be_prevented_on_the_power_adapter() {
        let policy = IdlePolicy {
            lock_after_seconds: None,
            suspend_after_seconds: Some(1800),
            prevent_sleep_on_adapter: true,
            ..IdlePolicy::default()
        };
        assert_eq!(
            idle_arguments(policy, true),
            ["-w", "timeout", "1800", SUSPEND_ACTION]
        );
        assert_eq!(idle_arguments(policy, false), ["-w"]);
    }

    #[test]
    fn policies_from_before_require_password_read_as_immediately() {
        let policy: IdlePolicy =
            serde_json::from_str(r#"{"version":1,"lock_after_seconds":300}"#).unwrap();
        assert_eq!(policy.require_password_after_seconds, Some(0));
        assert!(!policy.prevent_sleep_on_adapter);
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
