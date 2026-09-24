use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use rmac_gtk_settings::ToolkitFollower;
use rmac_session::{safe_mode_notice, DiagnosticReport, Supervisor, COMPONENT_UNITS};
use rmac_shell_settings::ShellSettingsStore;

/// The monitor sleeps until systemd reports a component job finishing. This
/// slow reconcile only catches what no signal announces (an appearance change
/// made outside System Settings, or a lost bus connection).
const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
/// A restart is several jobs; coalesce the burst into one health snapshot.
const EVENT_SETTLE: Duration = Duration::from_millis(250);

const USAGE: &str = "usage: rmac-session-supervisor <status|diagnostics|restore-last-good-settings|write-health|monitor|hold-power-key|observe-failure UNIT|begin-login|notify-safe-mode|leave-safe-mode|clear-safe-mode>";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    match arguments.as_slice() {
        [command] if command == "status" => {
            let supervisor = Supervisor::from_environment()?;
            println!("{}", serde_json::to_string_pretty(&supervisor.snapshot()?)?);
        }
        [command] if command == "diagnostics" => {
            let supervisor = Supervisor::from_environment()?;
            let settings = ShellSettingsStore::from_environment()?;
            let report =
                DiagnosticReport::from_health(supervisor.snapshot()?, settings.recovery_state());
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        [command] if command == "restore-last-good-settings" => {
            ShellSettingsStore::from_environment()?.restore_last_good()?;
            println!("Restored the last-known-good shell settings.");
        }
        [command] if command == "write-health" => {
            let supervisor = Supervisor::from_environment()?;
            supervisor.write_health()?;
        }
        [command] if command == "monitor" => monitor(&Supervisor::from_environment()?),
        [command] if command == "hold-power-key" => power_key::hold()?,
        [command, unit] if command == "observe-failure" => {
            let supervisor = Supervisor::from_environment()?;
            if supervisor.observe_failure(unit)? {
                systemctl(&["stop", "rmac-session.target"])?;
                systemctl(&["start", "rmac-safe-mode.target"])?;
            }
        }
        [command] if command == "begin-login" => {
            // The login wrapper reads exactly one word: `normal` or `safe`.
            let mode = Supervisor::from_environment()?.begin_login()?;
            println!("{}", mode.as_str());
        }
        [command] if command == "notify-safe-mode" => {
            notify_safe_mode(&Supervisor::from_environment()?)?;
        }
        [command] if command == "leave-safe-mode" => {
            // rmac-session-start --clear-safe-mode runs this, then starts the
            // normal target with the full rmac session identity.
            leave_safe_mode(&Supervisor::from_environment()?)?;
        }
        [command] if command == "clear-safe-mode" => {
            leave_safe_mode(&Supervisor::from_environment()?)?;
            systemctl(&["stop", "rmac-safe-mode.target"])?;
            systemctl(&["start", "rmac-session.target"])?;
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn monitor(supervisor: &Supervisor) -> ! {
    let (sender, receiver) = mpsc::channel();
    events::watch_components(sender);
    // Third-party GTK, libadwaita, Qt and browser windows follow the rmac
    // Appearance choice; System Settings applies a change at once and this
    // catches every other path, including login.
    let mut toolkits = ToolkitFollower::default();
    let mut reported_toolkit_error: Option<String> = None;
    let mut reconcile = true;
    loop {
        if let Err(error) = supervisor.write_health() {
            eprintln!("{error}");
        }
        if reconcile {
            match toolkits.poll() {
                Ok(_) => reported_toolkit_error = None,
                Err(error) => {
                    let message = error.to_string();
                    if reported_toolkit_error.as_deref() != Some(message.as_str()) {
                        eprintln!("{message}");
                        reported_toolkit_error = Some(message);
                    }
                }
            }
        }
        match receiver.recv_timeout(RECONCILE_INTERVAL) {
            Ok(()) => {
                thread::sleep(EVENT_SETTLE);
                while receiver.try_recv().is_ok() {}
                reconcile = false;
            }
            Err(RecvTimeoutError::Timeout) => reconcile = true,
            Err(RecvTimeoutError::Disconnected) => {
                // No event source at all: keep the slow reconcile, never spin.
                thread::sleep(RECONCILE_INTERVAL);
                reconcile = true;
            }
        }
    }
}

fn notify_safe_mode(supervisor: &Supervisor) -> Result<(), Box<dyn std::error::Error>> {
    let Some((unit, context)) = supervisor.notice_context()? else {
        return Ok(());
    };
    let text = safe_mode_notice(unit.as_deref(), context);
    // Always leave the explanation in the journal, whatever else is shown.
    eprintln!("{} {}", text.summary, text.body);
    match notice::present(&text) {
        Ok(true) => {
            // Restart Normally: the next login must not re-enter safe mode.
            supervisor.clear_safe_mode()?;
            niri_action(&["quit", "--skip-confirmation"])
                .map_err(|detail| format!("could not log out: {detail}"))?;
        }
        Ok(false) => {}
        Err(detail) => eprintln!("the safe-mode notice could not be shown: {detail}"),
    }
    Ok(())
}

/// Clears safe mode and points niri back at the rmac configuration, so the
/// normal target can start without logging out. When niri cannot switch its
/// configuration live, the marker is still cleared and the error says that a
/// new login is required.
fn leave_safe_mode(supervisor: &Supervisor) -> Result<(), Box<dyn std::error::Error>> {
    supervisor.clear_safe_mode()?;
    let mut reset = vec!["reset-failed"];
    reset.extend(COMPONENT_UNITS);
    systemctl(&reset)?;
    let relogin = |detail: String| {
        format!("Safe mode is cleared, but {detail}. Log out and back in to start normally.")
    };
    let config = rmac_niri_config().map_err(relogin)?;
    let config = config
        .to_str()
        .ok_or_else(|| relogin("the rmac niri configuration path is not UTF-8".into()))?
        .to_owned();
    systemctl(&["set-environment", &format!("NIRI_CONFIG={config}")])?;
    niri_action(&["load-config-file", "--path", &config]).map_err(|detail| {
        relogin(format!(
            "niri could not switch to the rmac configuration live ({detail})"
        ))
    })?;
    Ok(())
}

/// The session entry point the login wrapper writes, in safe mode too.
fn rmac_niri_config() -> Result<PathBuf, String> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "HOME is not set".to_owned())?;
    let path = config_home.join("rmac/niri/session.kdl");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(path),
        _ => Err("the rmac niri configuration has not been prepared by an rmac login".into()),
    }
}

fn niri_action(arguments: &[&str]) -> Result<(), String> {
    let socket = std::env::var("NIRI_SOCKET")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| manager_environment_value("NIRI_SOCKET"))
        .ok_or_else(|| "niri is not running in this session".to_owned())?;
    let output = Command::new("niri")
        .args(["msg", "action"])
        .args(arguments)
        .env("NIRI_SOCKET", socket)
        .output()
        .map_err(|error| format!("niri msg could not run: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// One `KEY=value` entry of the systemd user manager environment, which is
/// how a TTY finds the graphical session's niri socket.
fn manager_environment_value(key: &str) -> Option<String> {
    let output = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn systemctl(arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl --user {} failed", arguments.join(" ")).into())
    }
}

#[cfg(target_os = "linux")]
mod events {
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    use rmac_session::COMPONENT_UNITS;
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::{message::Type, MatchRule};

    const RECONNECT_DELAY: Duration = Duration::from_secs(30);

    /// Sends one wake-up per finished systemd job of an rmac component: every
    /// start, stop, crash restart and failure passes through one.
    pub fn watch_components(sender: Sender<()>) {
        let spawned = std::thread::Builder::new()
            .name("systemd-events".into())
            .spawn(move || loop {
                match watch_once(&sender) {
                    Ok(()) => return,
                    Err(error) => eprintln!("systemd unit events are unavailable: {error}"),
                }
                std::thread::sleep(RECONNECT_DELAY);
            });
        if let Err(error) = spawned {
            eprintln!("could not watch systemd unit events: {error}");
        }
    }

    /// Returns `Ok` only when the monitor stopped listening.
    fn watch_once(sender: &Sender<()>) -> zbus::Result<()> {
        let connection = Connection::session()?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.systemd1")?
            .path("/org/freedesktop/systemd1")?
            .interface("org.freedesktop.systemd1.Manager")?
            .member("JobRemoved")?
            .build();
        let messages = MessageIterator::for_match_rule(rule, &connection, Some(64))?;
        // The manager only emits job signals once a client subscribes.
        connection.call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "Subscribe",
            &(),
        )?;
        for message in messages {
            let message = message?;
            let Ok((_, _, unit, _)) =
                message
                    .body()
                    .deserialize::<(u32, zbus::zvariant::OwnedObjectPath, String, String)>()
            else {
                continue;
            };
            if COMPONENT_UNITS.contains(&unit.as_str()) && sender.send(()).is_err() {
                return Ok(());
            }
        }
        Err(zbus::Error::Failure(
            "the systemd signal stream ended".into(),
        ))
    }
}

#[cfg(not(target_os = "linux"))]
mod events {
    /// Development hosts have no systemd user manager; the slow reconcile
    /// interval is the only wake-up. Dropping the sender reports that.
    pub fn watch_components(_sender: std::sync::mpsc::Sender<()>) {}
}

#[cfg(target_os = "linux")]
mod notice {
    use std::collections::HashMap;
    use std::process::Command;

    use rmac_session::{SafeModeNotice, RESTART_NORMALLY_ACTION};
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::zvariant::Value;
    use zbus::{message::Type, MatchRule};

    const BUS_NAME: &str = "org.freedesktop.Notifications";
    const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

    /// Shows the notice and returns whether the user chose Restart Normally.
    /// Safe mode does not run the rmac notification authority, so this uses
    /// whichever notification server the plain niri session has, then the
    /// desktop's dialog tool; with neither, the journal line is all there is.
    pub fn present(notice: &SafeModeNotice) -> Result<bool, String> {
        match notify(notice) {
            Ok(choice) => Ok(choice),
            Err(bus_error) => ask_with_dialog(notice).map_err(|dialog_error| {
                format!("no notification server ({bus_error}) and no dialog tool ({dialog_error})")
            }),
        }
    }

    fn notify(notice: &SafeModeNotice) -> zbus::Result<bool> {
        let connection = Connection::session()?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(OBJECT_PATH)?
            .interface(BUS_NAME)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, &connection, Some(16))?;
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("urgency", Value::U8(2));
        hints.insert("resident", Value::Bool(true));
        let reply = connection.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(BUS_NAME),
            "Notify",
            &(
                "rmac",
                0_u32,
                "dialog-warning",
                notice.summary.as_str(),
                notice.body.as_str(),
                vec![RESTART_NORMALLY_ACTION, notice.action_label],
                hints,
                // Never expire: the notice matters until the user decides.
                0_i32,
            ),
        )?;
        let id: u32 = reply.body().deserialize()?;
        // Only the server that answered Notify may answer the notice: any
        // other session peer can emit a signal with the same path, interface
        // and id.
        let server = reply
            .header()
            .sender()
            .map(|name| name.as_str().to_owned())
            .ok_or_else(|| zbus::Error::Failure("the Notify reply has no sender".into()))?;
        for message in signals {
            let message = message?;
            let header = message.header();
            if !signal_from(header.sender().map(|name| name.as_str()), &server) {
                continue;
            }
            let member = header.member().map(|name| name.as_str().to_owned());
            match member.as_deref() {
                Some("ActionInvoked") => {
                    let (signal_id, key): (u32, String) = message.body().deserialize()?;
                    if signal_id == id {
                        return Ok(key == RESTART_NORMALLY_ACTION);
                    }
                }
                Some("NotificationClosed") => {
                    let (signal_id, _reason): (u32, u32) = message.body().deserialize()?;
                    if signal_id == id {
                        return Ok(false);
                    }
                }
                _ => {}
            }
        }
        Err(zbus::Error::Failure(
            "the notification server went away".into(),
        ))
    }

    /// Whether a signal came from the notification server's unique name.
    fn signal_from(sender: Option<&str>, server: &str) -> bool {
        sender == Some(server)
    }

    fn ask_with_dialog(notice: &SafeModeNotice) -> Result<bool, String> {
        let status = Command::new("zenity")
            .args([
                "--question",
                "--title",
                notice.summary.as_str(),
                "--text",
                notice.body.as_str(),
                "--ok-label",
                notice.action_label,
                "--cancel-label",
                "Not Now",
            ])
            .status()
            .map_err(|error| error.to_string())?;
        // zenity: 0 accepted, 1 declined or closed, 5 timed out.
        match status.code() {
            Some(0) => Ok(true),
            Some(1 | 5) => Ok(false),
            _ => Err(format!("zenity exited with {status}")),
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn only_the_answering_server_can_answer_the_safe_mode_notice() {
            assert!(super::signal_from(Some(":1.42"), ":1.42"));
            assert!(!super::signal_from(Some(":1.43"), ":1.42"));
            assert!(!super::signal_from(None, ":1.42"));
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod notice {
    pub fn present(_notice: &rmac_session::SafeModeNotice) -> Result<bool, String> {
        Err("safe-mode notices are shown only on Linux".into())
    }
}

/// The session-long fail-safe for the power button (ADR 0020, item 7).
///
/// The login wrapper starts `hold-power-key` before niri and keeps it for the
/// whole login. It holds logind's `handle-power-key` block inhibitor, so a
/// press while the lock coordinator is starting, restarting or crash-looping
/// is ignored instead of falling back to Ubuntu's `HandlePowerKey=poweroff`.
/// It never handles the key itself; the coordinator does. Holding the
/// button for about four seconds still forces the machine off in firmware.
#[cfg(target_os = "linux")]
mod power_key {
    use zbus::zvariant::OwnedFd;

    pub const WHAT: &str = "handle-power-key";
    pub const WHO: &str = "Lulo OS session";
    pub const WHY: &str = "Keeps the power button from shutting down while Lulo OS handles it";

    /// Takes the inhibitor and holds it until the wrapper that started this
    /// process exits (or stops it). Returns only on failure or once the
    /// wrapper is gone.
    pub fn hold() -> Result<(), String> {
        let parent = end_with_parent()?;
        let connection = zbus::blocking::Connection::system()
            .map_err(|error| format!("could not reach the system bus: {error}"))?;
        let reply = connection
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &(WHAT, WHO, WHY, "block"),
            )
            .map_err(|error| {
                format!(
                    "logind refused the power-button inhibitor: {error}; while the lock \
                     coordinator is down a press does what logind is configured to do"
                )
            })?;
        let inhibitor: OwnedFd = reply
            .body()
            .deserialize()
            .map_err(|error| format!("logind sent an unreadable inhibitor: {error}"))?;
        // Parked, not polling. The descriptor is released when this process
        // ends, which the parent-death signal ties to the wrapper.
        loop {
            std::thread::park();
            if parent_gone(parent) {
                drop(inhibitor);
                return Ok(());
            }
        }
    }

    /// SIGTERM this process when the wrapper dies, however it dies.
    fn end_with_parent() -> Result<libc::pid_t, String> {
        // SAFETY: getppid has no preconditions.
        let parent = unsafe { libc::getppid() };
        // SAFETY: PR_SET_PDEATHSIG only changes this process's own
        // parent-death signal.
        if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0) } != 0 {
            return Err(format!(
                "could not tie the power-button inhibitor to the session: {}",
                std::io::Error::last_os_error()
            ));
        }
        if parent_gone(parent) {
            return Err("the session ended before the power-button inhibitor was taken".into());
        }
        Ok(parent)
    }

    fn parent_gone(parent: libc::pid_t) -> bool {
        // SAFETY: getppid has no preconditions.
        unsafe { libc::getppid() != parent }
    }
}

#[cfg(not(target_os = "linux"))]
mod power_key {
    pub fn hold() -> Result<(), String> {
        Err("the power-button inhibitor exists only on Linux".into())
    }
}
