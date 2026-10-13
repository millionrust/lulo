//! Calendar reminders on Windows (ADR 0023 phase 2f).
//!
//! Nothing stays resident. Calendar (and a login task) runs
//! `rmac-calendar-agent.exe`; it reads the local calendars, shows a toast and
//! plays the alert sound for every reminder now due, remembers what it has
//! shown (`alerts.json`, so a reminder never shows twice) and leaves exactly
//! one Task Scheduler task behind, set for the next reminder. That task runs
//! this same program when the time comes, whether or not Calendar is open.
//! The task is a one-off built from XML rather than `schtasks /sd`, whose date
//! format follows the PC's regional settings.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, Duration, Local, NaiveDateTime, Timelike, Utc};
use rmac_calendar_store::local::LocalStore;
use rmac_calendar_store::Calendar as IcalCalendar;

use crate::alarms::ActiveAlarm;
use crate::engine::{self, Evaluation};
use crate::state;

/// The task's name in Task Scheduler; Lulo owns this one task.
pub const TASK_NAME: &str = "LuloCalendarReminder";
const AUMID: &str = "Lulo.Calendar";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Point the XDG variables Lulo's crates read at the per-user Windows folders,
/// as `rmac_ui` does for the apps (this program is not a GPUI app). Variables
/// already set are left alone.
pub fn prepare_environment() {
    let folder = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let roaming = folder("APPDATA").map(|path| path.join("Lulo"));
    let local = folder("LOCALAPPDATA").map(|path| path.join("Lulo"));
    let wanted = [
        ("HOME", folder("USERPROFILE")),
        ("XDG_DATA_HOME", roaming.map(|path| path.join("Data"))),
        ("XDG_STATE_HOME", local.map(|path| path.join("State"))),
    ];
    for (name, value) in wanted {
        let unset = std::env::var_os(name).is_none_or(|current| current.is_empty());
        if let (true, Some(value)) = (unset, value) {
            std::env::set_var(name, value);
        }
    }
}

/// The next whole minute at or after `at`: Task Scheduler cannot start a
/// task between minutes, and starting early would find nothing due.
pub fn round_up_to_minute(at: DateTime<Utc>) -> DateTime<Utc> {
    let truncated = at
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .unwrap_or(at);
    if truncated < at {
        truncated + Duration::minutes(1)
    } else {
        truncated
    }
}

fn xml_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The one-off task: start `exe --check` at `start` (local time), also when
/// the PC was asleep or off at that moment, on battery too, never twice at once.
pub fn task_xml(start: NaiveDateTime, exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Lulo Calendar reminder</Description></RegistrationInfo>
  <Triggers>
    <TimeTrigger>
      <StartBoundary>{}</StartBoundary>
      <Enabled>true</Enabled>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT5M</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{}</Command><Arguments>--check</Arguments></Exec>
  </Actions>
</Task>
"#,
        start.format("%Y-%m-%dT%H:%M:%S"),
        xml_attr(exe)
    )
}

/// UTF-16LE with a byte-order mark, which Task Scheduler's XML import wants
/// for a `UTF-16` declaration.
fn utf16_file(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn schtasks(arguments: &[&str]) -> io::Result<bool> {
    use std::os::windows::process::CommandExt as _;
    Command::new("schtasks")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|status| status.success())
}

/// Leave one task set for `wake`, or none when no reminder is pending.
pub fn arm(wake: Option<DateTime<Utc>>, exe: &Path) -> io::Result<()> {
    let Some(wake) = wake else {
        // Nothing pending (or the task was never made): either way the end
        // state is no task.
        let _ = schtasks(&["/delete", "/tn", TASK_NAME, "/f"]);
        return Ok(());
    };
    let exe = exe
        .to_str()
        .filter(|path| !path.contains(['"', '%']))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path cannot be scheduled"))?;
    let start = round_up_to_minute(wake).with_timezone(&Local).naive_local();
    let directory = state::state_path()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state directory"))?;
    rmac_storage::create_dir_all_private(&directory)?;
    let file = directory.join("reminder-task.xml");
    rmac_storage::atomic_write_private(&file, &utf16_file(&task_xml(start, exe)))?;
    let path = file
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad state path"))?;
    if schtasks(&["/create", "/tn", TASK_NAME, "/xml", path, "/f"])? {
        Ok(())
    } else {
        Err(io::Error::other("Task Scheduler refused the reminder task"))
    }
}

fn load_calendars() -> io::Result<BTreeMap<String, IcalCalendar>> {
    let root = LocalStore::default_root()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let store = LocalStore::open(root);
    let mut calendars = BTreeMap::new();
    for source in store.sources()? {
        calendars.insert(source.uid.clone(), store.read(&source.uid)?);
    }
    Ok(calendars)
}

fn announce(alarm: &ActiveAlarm) {
    let when = if alarm.all_day {
        "All day".to_owned()
    } else {
        alarm
            .occurrence_start
            .with_timezone(&Local)
            .format("%H:%M")
            .to_string()
    };
    let title = if alarm.summary.trim().is_empty() {
        "Event"
    } else {
        alarm.summary.as_str()
    };
    let _ = rmac_win_toast::show(AUMID, title, &when);
    let _ = rmac_sound::play_blocking(rmac_sound::Cue::Notification);
}

/// Show what is due, remember it and re-arm the task. The whole of the
/// program.
pub fn run() -> io::Result<()> {
    prepare_environment();
    let calendars = load_calendars()?;
    let mut alerts = state::load().unwrap_or_default();
    let Evaluation { due, next_wake } =
        engine::evaluate(Utc::now(), &calendars, chrono_tz::UTC, &mut alerts)
            .map_err(io::Error::other)?;
    for alarm in &due {
        announce(alarm);
    }
    state::save(&alerts)?;
    arm(next_wake, &std::env::current_exe()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    #[test]
    fn a_wake_is_rounded_up_to_the_next_whole_minute() {
        let at = |s, ns| Utc.with_ymd_and_hms(2026, 10, 14, 9, 14, s).unwrap() + Duration::nanoseconds(ns);
        assert_eq!(round_up_to_minute(at(0, 0)), at(0, 0));
        assert_eq!(round_up_to_minute(at(1, 0)), at(0, 0) + Duration::minutes(1));
        assert_eq!(round_up_to_minute(at(30, 5)), at(0, 0) + Duration::minutes(1));
    }

    #[test]
    fn the_task_xml_starts_once_at_the_given_local_time_and_escapes_the_path() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap();
        let xml = task_xml(start, r"C:\Users\A&B\rmac-calendar-agent.exe");
        assert!(xml.contains("<StartBoundary>2026-10-14T09:15:00</StartBoundary>"));
        assert!(xml.contains(r"C:\Users\A&amp;B\rmac-calendar-agent.exe"));
        assert!(xml.contains("<Arguments>--check</Arguments>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
    }

    #[test]
    fn the_xml_file_is_utf16_with_a_byte_order_mark() {
        let bytes = utf16_file("<a/>");
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        assert_eq!(bytes.len(), 2 + 8);
    }
}
