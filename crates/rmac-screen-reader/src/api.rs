use std::fmt;
use std::io;
use std::process::Command;

use crate::process::bounded_command_output;
use crate::watch::monitor_once;

pub(crate) const SCHEMA: &str = "org.gnome.desktop.a11y.applications";
pub(crate) const SCREEN_READER_KEY: &str = "screen-reader-enabled";
const KEYBINDING_SCHEMA: &str = "org.gnome.desktop.wm.keybindings";
const KEYBINDING_KEY: &str = "toggle-screen-reader";
const MAX_ERROR_BYTES: usize = 512;
const WATCH_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Available,
    Changed,
    Unavailable,
}

/// GSettings' `screen-reader-enabled` value and whether Orca is actually
/// running to match it. The two can disagree if Orca exits on its own (a
/// crash) or before the first sync after a fresh login; `detail` explains
/// that rather than the snapshot silently picking one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub writable: bool,
    pub enabled: bool,
    pub orca_running: bool,
    pub detail: Option<String>,
}

impl Snapshot {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            available: false,
            writable: false,
            enabled: false,
            orca_running: false,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: bounded_text(&detail.into()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub(crate) trait Runner {
    fn run(&self, arguments: &[&str]) -> io::Result<crate::process::CommandOutput>;
}

pub(crate) struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, arguments: &[&str]) -> io::Result<crate::process::CommandOutput> {
        bounded_command_output(Command::new("gsettings").args(arguments))
    }
}

/// Orca's process lifecycle, isolated behind a trait so the GSettings toggle
/// logic can be tested without ever spawning or signaling a real process.
pub(crate) trait OrcaControl {
    fn is_running(&self) -> bool;
    fn start(&self) -> Result<(), Error>;
    fn stop(&self) -> Result<(), Error>;
}

pub(crate) struct RealOrca;

impl OrcaControl for RealOrca {
    fn is_running(&self) -> bool {
        crate::orca::is_orca_running()
    }

    fn start(&self) -> Result<(), Error> {
        crate::orca::start_orca()
    }

    fn stop(&self) -> Result<(), Error> {
        crate::orca::stop_orca()
    }
}

/// Read GSettings' screen-reader flag and reconcile it against whether Orca
/// is actually running. Never starts or stops Orca: only `set_enabled` does.
pub fn snapshot() -> Result<Snapshot, Error> {
    snapshot_with(&RealRunner, &RealOrca)
}

/// Set GSettings' screen-reader flag and start or stop Orca to match, the
/// way GNOME's own accessibility settings daemon does. This is the only
/// place in rmac that starts or stops Orca, so nothing else in the Lulo
/// session can make it start talking unannounced.
pub fn set_enabled(enabled: bool) -> Result<Snapshot, Error> {
    set_enabled_with(&RealRunner, &RealOrca, enabled)
}

/// Follow authoritative GSettings changes without retaining subprocess
/// output. The monitor is restarted after failure and exits once the
/// receiver closes. This only keeps System Settings' switch in sync; it
/// never starts or stops Orca on its own.
pub fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    while !sender.is_closed() {
        if monitor_once(&sender).is_err() {
            let _ = sender.try_send(WatchEvent::Unavailable);
        }
        if !sender.is_closed() {
            std::thread::sleep(WATCH_RECONNECT_DELAY);
        }
    }
    Ok(())
}

/// Disable GNOME's own Super+Alt+S "toggle-screen-reader" keybinding for
/// this session. The Lulo session reacts only to its own Screen Reader
/// shortcut and the System Settings switch (both call `set_enabled`), never
/// to a GNOME-registered accelerator, so an owner who happens to hit
/// Super+Alt+S never hears Orca start unannounced.
pub fn disable_gnome_shortcut() -> Result<(), Error> {
    disable_gnome_shortcut_with(&RealRunner)
}

pub(crate) fn snapshot_with(
    runner: &impl Runner,
    orca: &impl OrcaControl,
) -> Result<Snapshot, Error> {
    let value = match runner.run(&["get", SCHEMA, SCREEN_READER_KEY]) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Snapshot::unavailable(
                "gsettings is not installed; the screen reader toggle is unavailable",
            ));
        }
        Err(error) => return Err(command_error("read the screen-reader setting", error)),
    };
    if !value.success {
        return Ok(Snapshot::unavailable(
            "the GNOME accessibility applications schema is unavailable",
        ));
    }
    let enabled = parse_bool(&value.stdout).ok_or_else(|| {
        Error::new(
            "read the screen-reader setting",
            "gsettings returned an invalid boolean",
        )
    })?;

    let writable = runner
        .run(&["writable", SCHEMA, SCREEN_READER_KEY])
        .map_err(|error| command_error("check the screen-reader setting policy", error))?;
    if !writable.success {
        return Err(Error::new(
            "check the screen-reader setting policy",
            "gsettings could not inspect the key",
        ));
    }
    let can_write = parse_bool(&writable.stdout).ok_or_else(|| {
        Error::new(
            "check the screen-reader setting policy",
            "gsettings returned an invalid writability result",
        )
    })?;
    let orca_running = orca.is_running();
    let detail = if !can_write {
        Some("The screen reader setting is locked by the current GSettings policy".to_string())
    } else if enabled != orca_running {
        Some(if enabled {
            "The screen reader is on, but Orca is not currently running.".to_string()
        } else {
            "The screen reader is off, but Orca is still running.".to_string()
        })
    } else {
        None
    };
    Ok(Snapshot {
        available: true,
        writable: can_write,
        enabled,
        orca_running,
        detail,
    })
}

pub(crate) fn set_enabled_with(
    runner: &impl Runner,
    orca: &impl OrcaControl,
    enabled: bool,
) -> Result<Snapshot, Error> {
    let before = snapshot_with(runner, orca)?;
    if !before.available {
        return Err(Error::new(
            "set the screen-reader setting",
            before
                .detail
                .unwrap_or_else(|| "the GSettings authority is unavailable".to_string()),
        ));
    }
    if !before.writable {
        return Err(Error::new(
            "set the screen-reader setting",
            before
                .detail
                .unwrap_or_else(|| "the GSettings key is read-only".to_string()),
        ));
    }
    if before.enabled != enabled {
        let output = runner
            .run(&[
                "set",
                SCHEMA,
                SCREEN_READER_KEY,
                if enabled { "true" } else { "false" },
            ])
            .map_err(|error| command_error("set the screen-reader setting", error))?;
        if !output.success {
            return Err(Error::new(
                "set the screen-reader setting",
                "gsettings rejected the value",
            ));
        }
        let after = snapshot_with(runner, orca)?;
        if after.enabled != enabled {
            return Err(Error::new(
                "confirm the screen-reader setting",
                "the authority did not accept the requested value",
            ));
        }
    }
    if enabled {
        orca.start()?;
    } else {
        orca.stop()?;
    }
    Ok(Snapshot {
        enabled,
        orca_running: orca.is_running(),
        detail: None,
        ..before
    })
}

pub(crate) fn disable_gnome_shortcut_with(runner: &impl Runner) -> Result<(), Error> {
    let output = runner
        .run(&["set", KEYBINDING_SCHEMA, KEYBINDING_KEY, "@as []"])
        .map_err(|error| command_error("disable GNOME's screen-reader shortcut", error))?;
    if !output.success {
        return Err(Error::new(
            "disable GNOME's screen-reader shortcut",
            "gsettings rejected the empty keybinding list",
        ));
    }
    Ok(())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn command_error(operation: &'static str, error: io::Error) -> Error {
    let detail = match error.kind() {
        io::ErrorKind::NotFound => "gsettings is not installed",
        io::ErrorKind::TimedOut => "gsettings did not finish before the bounded deadline",
        io::ErrorKind::InvalidData => "gsettings returned invalid or excessive output",
        io::ErrorKind::PermissionDenied => "permission was denied while running gsettings",
        _ => "gsettings could not be run",
    };
    Error::new(operation, detail)
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct ScriptedRunner {
        calls: RefCell<Vec<Vec<String>>>,
        responses: RefCell<Vec<io::Result<crate::process::CommandOutput>>>,
    }

    impl ScriptedRunner {
        fn new(responses: Vec<io::Result<crate::process::CommandOutput>>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(responses.into_iter().rev().collect()),
            }
        }
    }

    impl Runner for ScriptedRunner {
        fn run(&self, arguments: &[&str]) -> io::Result<crate::process::CommandOutput> {
            self.calls
                .borrow_mut()
                .push(arguments.iter().map(|value| value.to_string()).collect());
            self.responses
                .borrow_mut()
                .pop()
                .expect("no more scripted gsettings responses")
        }
    }

    #[derive(Default)]
    struct ScriptedOrca {
        running: Cell<bool>,
        starts: Cell<u32>,
        stops: Cell<u32>,
    }

    impl OrcaControl for ScriptedOrca {
        fn is_running(&self) -> bool {
            self.running.get()
        }

        fn start(&self) -> Result<(), Error> {
            self.starts.set(self.starts.get() + 1);
            self.running.set(true);
            Ok(())
        }

        fn stop(&self) -> Result<(), Error> {
            self.stops.set(self.stops.get() + 1);
            self.running.set(false);
            Ok(())
        }
    }

    fn ok(stdout: &str) -> io::Result<crate::process::CommandOutput> {
        Ok(crate::process::CommandOutput {
            success: true,
            stdout: stdout.to_string(),
        })
    }

    #[test]
    fn snapshot_reports_the_authority_value_and_writability() {
        let runner = ScriptedRunner::new(vec![ok("false\n"), ok("true\n")]);
        let orca = ScriptedOrca::default();
        let snapshot = snapshot_with(&runner, &orca).unwrap();
        assert!(snapshot.available);
        assert!(snapshot.writable);
        assert!(!snapshot.enabled);
    }

    #[test]
    fn snapshot_is_unavailable_when_gsettings_is_missing() {
        let runner = ScriptedRunner::new(vec![Err(io::Error::from(io::ErrorKind::NotFound))]);
        let orca = ScriptedOrca::default();
        let snapshot = snapshot_with(&runner, &orca).unwrap();
        assert!(!snapshot.available);
        assert!(!snapshot.enabled);
    }

    #[test]
    fn snapshot_flags_orca_disagreeing_with_the_authority() {
        let runner = ScriptedRunner::new(vec![ok("true\n"), ok("true\n")]);
        let orca = ScriptedOrca::default();
        orca.running.set(false);
        let snapshot = snapshot_with(&runner, &orca).unwrap();
        assert!(snapshot.enabled);
        assert!(!snapshot.orca_running);
        assert!(snapshot.detail.unwrap().contains("not currently running"));
    }

    #[test]
    fn a_read_only_key_refuses_a_write_and_never_touches_orca() {
        let runner = ScriptedRunner::new(vec![ok("false\n"), ok("false\n")]);
        let orca = ScriptedOrca::default();
        let result = set_enabled_with(&runner, &orca, true);
        assert!(result.is_err());
        assert_eq!(runner.calls.borrow().len(), 2);
        assert_eq!(orca.starts.get(), 0);
        assert_eq!(orca.stops.get(), 0);
    }

    #[test]
    fn an_unchanged_value_does_not_re_issue_a_write_but_still_syncs_orca() {
        let runner = ScriptedRunner::new(vec![ok("true\n"), ok("true\n")]);
        let orca = ScriptedOrca::default();
        let snapshot = set_enabled_with(&runner, &orca, true).unwrap();
        assert!(snapshot.enabled);
        assert!(snapshot.orca_running);
        assert_eq!(orca.starts.get(), 1);
        assert_eq!(runner.calls.borrow().len(), 2);
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.first().map(String::as_str) == Some("set")));
    }

    #[test]
    fn enabling_writes_the_key_and_starts_orca() {
        let runner = ScriptedRunner::new(vec![
            ok("false\n"),
            ok("true\n"),
            ok("true"),
            ok("true\n"),
            ok("true\n"),
        ]);
        let orca = ScriptedOrca::default();
        let snapshot = set_enabled_with(&runner, &orca, true).unwrap();
        assert!(snapshot.enabled);
        assert!(snapshot.orca_running);
        assert_eq!(orca.starts.get(), 1);
        assert_eq!(orca.stops.get(), 0);
    }

    #[test]
    fn disabling_writes_the_key_and_stops_orca() {
        let runner = ScriptedRunner::new(vec![
            ok("true\n"),
            ok("true\n"),
            ok("true"),
            ok("false\n"),
            ok("true\n"),
        ]);
        let orca = ScriptedOrca::default();
        orca.running.set(true);
        let snapshot = set_enabled_with(&runner, &orca, false).unwrap();
        assert!(!snapshot.enabled);
        assert!(!snapshot.orca_running);
        assert_eq!(orca.stops.get(), 1);
        assert_eq!(orca.starts.get(), 0);
    }

    #[test]
    fn bounded_text_strips_control_characters_and_length() {
        assert_eq!(bounded_text("a\nb\tc"), "a b c");
        assert_eq!(bounded_text(&"x".repeat(600)).len(), MAX_ERROR_BYTES);
    }

    #[test]
    fn parse_bool_is_an_exact_match() {
        assert_eq!(parse_bool("true\n"), Some(true));
        assert_eq!(parse_bool(" false "), Some(false));
        assert_eq!(parse_bool("1"), None);
    }
}
