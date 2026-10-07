//! Alarms and timers ring through an OS scheduler, so they sound even when
//! Clock is not running and survive logout and reboot: a user systemd timer
//! on Linux, Task Scheduler on Windows.
//!
//! On Linux, Clock writes one timer unit whose `OnCalendar=` lines are
//! every enabled alarm, snooze and running countdown, and one service that
//! runs `rmac-clock --ring-due`. On Windows, each `OnCalendar=`-shaped entry
//! becomes one Task Scheduler task (`schtasks`) that runs the same
//! `--ring-due` argument. Either way the ring process re-reads the state,
//! rings what is due, updates the state and rewrites the schedule. Nothing
//! stays resident.

use std::io;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", windows))]
use std::process::{Command, Stdio};

use crate::format::WallTime;
use crate::store::State;

pub const TIMER_UNIT: &str = "rmac-clock-alarms.timer";
pub const SERVICE_UNIT: &str = "rmac-clock-ring.service";
/// Argument that makes the Clock binary ring and exit without a window.
pub const RING_ARGUMENT: &str = "--ring-due";
/// Every Windows Task Scheduler task Clock creates starts with this, so a
/// re-schedule can find and clear only its own tasks.
#[cfg(any(windows, test))]
const TASK_PREFIX: &str = "RmacClockAlarm-";

/// systemd `OnCalendar=` expressions for everything that should ring after
/// `now` (Unix milliseconds), in local time.
pub fn calendar_entries(state: &State, now: u64, offset_at: &impl Fn(i64) -> i32) -> Vec<String> {
    let now_seconds = (now / 1000) as i64;
    let exact = |utc: i64| {
        let wall = WallTime::at(utc, offset_at(utc));
        let (year, month, day) = wall.date();
        format!(
            "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
            wall.hour, wall.minute, wall.second
        )
    };
    let mut entries = Vec::new();
    for alarm in state.alarms.iter().filter(|alarm| alarm.enabled) {
        let days = alarm.repeat.calendar();
        let time = format!(
            "*-*-* {:02}:{:02}:00",
            alarm.hour.min(23),
            alarm.minute.min(59)
        );
        entries.push(if days.is_empty() {
            time
        } else {
            format!("{days} {time}")
        });
        if let Some(until) = alarm.snoozed_until.filter(|&until| until > now_seconds) {
            entries.push(exact(until));
        }
    }
    for timer in &state.timers {
        if let Some(ends_at) = timer.ends_at().filter(|&ends_at| ends_at > now) {
            // Round up so the ring never starts before the timer reaches 0.
            entries.push(exact(ends_at.div_ceil(1000) as i64));
        }
    }
    entries.sort();
    entries.dedup();
    entries
}

pub fn timer_unit(entries: &[String]) -> String {
    let mut unit = String::from(
        "# Written by rmac Clock; rewritten whenever alarms or timers change.\n\
         [Unit]\n\
         Description=rmac Clock alarms and timers\n\
         \n\
         [Timer]\n",
    );
    for entry in entries {
        unit.push_str("OnCalendar=");
        unit.push_str(entry);
        unit.push('\n');
    }
    unit.push_str(&format!(
        "AccuracySec=1s\nPersistent=false\nUnit={SERVICE_UNIT}\n\n[Install]\nWantedBy=timers.target\n"
    ));
    unit
}

/// The service that rings. `None` when the executable path cannot be
/// written into a unit safely.
pub fn service_unit(executable: &Path) -> Option<String> {
    let path = executable.to_str()?;
    if !path.starts_with('/')
        || path.chars().any(|character| {
            character.is_whitespace() || matches!(character, '%' | '"' | '\\' | '$')
        })
    {
        return None;
    }
    Some(format!(
        "# Written by rmac Clock.\n\
         [Unit]\n\
         Description=Ring rmac Clock alarms and timers\n\
         \n\
         [Service]\n\
         Type=exec\n\
         ExecStart={path} {RING_ARGUMENT}\n"
    ))
}

pub fn unit_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join("systemd/user"))
}

/// One alarm/snooze/timer entry, parsed off the shared `calendar_entries`
/// strings into the shape Task Scheduler understands. Pure and
/// platform-neutral so it is unit-tested on every target, even though only
/// `cfg(windows)` ever calls `apply` with it.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskSchedule {
    /// `*-*-* HH:MM:SS` with no day list: every day.
    Daily { time: String },
    /// `Mon,Wed *-*-* HH:MM:SS`: specific weekdays.
    Weekly { days: String, time: String },
    /// `YYYY-MM-DD HH:MM:SS`: a single snooze or timer, not repeated.
    Once { date: String, time: String },
}

#[cfg(any(windows, test))]
fn task_name(index: usize) -> String {
    format!("{TASK_PREFIX}{index:02}")
}

/// `Days::calendar()`'s `"Mon,Wed"` (or `""` for every day) as the
/// upper-case, comma-joined day list `schtasks /d` expects.
#[cfg(any(windows, test))]
fn windows_days(days: &str) -> Option<String> {
    days.split(',')
        .map(|day| {
            Some(
                match day {
                    "Mon" => "MON",
                    "Tue" => "TUE",
                    "Wed" => "WED",
                    "Thu" => "THU",
                    "Fri" => "FRI",
                    "Sat" => "SAT",
                    "Sun" => "SUN",
                    _ => return None,
                }
                .to_string(),
            )
        })
        .collect::<Option<Vec<_>>>()
        .map(|days| days.join(","))
}

/// Parse one `calendar_entries` line into a `TaskSchedule`, or `None` when
/// it is not one of the shapes `calendar_entries`/`exact` ever produce.
#[cfg(any(windows, test))]
fn plan_task(entry: &str) -> Option<TaskSchedule> {
    let tokens: Vec<&str> = entry.split(' ').collect();
    let (days, date, time) = match tokens.as_slice() {
        [date, time] => (None, *date, *time),
        [days, date, time] => (Some(*days), *date, *time),
        _ => return None,
    };
    if date == "*-*-*" {
        return match days {
            None => Some(TaskSchedule::Daily {
                time: time.to_string(),
            }),
            Some(days) => Some(TaskSchedule::Weekly {
                days: windows_days(days)?,
                time: time.to_string(),
            }),
        };
    }
    // A one-time entry: `YYYY-MM-DD`, reformatted to `schtasks`'s `/sd
    // MM/DD/YYYY`.
    let (year, month, day) = match date.split('-').collect::<Vec<_>>().as_slice() {
        [year, month, day] => (*year, *month, *day),
        _ => return None,
    };
    Some(TaskSchedule::Once {
        date: format!("{month}/{day}/{year}"),
        time: time.to_string(),
    })
}

#[cfg(windows)]
fn schtasks(arguments: &[&str]) -> io::Result<()> {
    let output = Command::new("schtasks")
        .args(arguments)
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "schtasks {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Remove every Task Scheduler task this module created, ignoring ones
/// that are already gone. Run before writing the new set, since the
/// number of due entries (and so of tasks) changes from one call to the
/// next.
#[cfg(windows)]
fn clear_scheduled_tasks() -> io::Result<()> {
    let output = Command::new("schtasks")
        .args(["/query", "/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        // No tasks registered at all is reported as a failure by some
        // `schtasks` builds; either way there is nothing of ours to clear.
        return Ok(());
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    for line in listing.lines() {
        let Some(field) = line.split(',').next() else {
            continue;
        };
        let name = field.trim_matches('"').trim_start_matches('\\');
        if name.starts_with(TASK_PREFIX) {
            let _ = schtasks(&["/delete", "/tn", name, "/f"]);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn create_task(name: &str, command: &str, plan: &TaskSchedule) -> io::Result<()> {
    let mut arguments = vec!["/create", "/tn", name, "/tr", command, "/f"];
    match plan {
        TaskSchedule::Daily { time } => {
            arguments.extend(["/sc", "daily", "/st", time.as_str()]);
        }
        TaskSchedule::Weekly { days, time } => {
            arguments.extend(["/sc", "weekly", "/d", days.as_str(), "/st", time.as_str()]);
        }
        TaskSchedule::Once { date, time } => {
            arguments.extend(["/sc", "once", "/sd", date.as_str(), "/st", time.as_str()]);
        }
    }
    schtasks(&arguments)
}

/// Write the units for `state` and (re)arm or disarm the timer. Windows has
/// no systemd user units (ADR 0023): each due alarm, snooze and running
/// countdown becomes one Task Scheduler task that runs `rmac-clock
/// --ring-due`, mirroring the Linux design above with `schtasks` in place
/// of `systemctl`. The task fires (and rings, with sound — see `ring.rs`)
/// whether or not Clock itself is running at the time.
#[cfg(windows)]
pub fn apply(state: &State, now: u64, offset_at: &impl Fn(i64) -> i32) -> io::Result<()> {
    clear_scheduled_tasks()?;
    let entries = calendar_entries(state, now, offset_at);
    if entries.is_empty() {
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let path = executable.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Clock's path cannot be scheduled",
        )
    })?;
    if path.chars().any(|character| matches!(character, '"' | '%')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Clock's path cannot be scheduled",
        ));
    }
    let command = format!("\"{path}\" {RING_ARGUMENT}");
    for (index, entry) in entries.iter().enumerate() {
        if let Some(plan) = plan_task(entry) {
            create_task(&task_name(index), &command, &plan)?;
        }
    }
    Ok(())
}

/// Write the units for `state` and (re)arm or disarm the timer. Every other
/// target (macOS, used only for development) has neither systemd nor Task
/// Scheduler: an honest, quiet "not available" error, never a faked ring.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn apply(_state: &State, _now: u64, _offset_at: &impl Fn(i64) -> i32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "alarms and timers are not available on this platform yet",
    ))
}

/// Write the units for `state` and (re)arm or disarm the timer.
#[cfg(target_os = "linux")]
pub fn apply(state: &State, now: u64, offset_at: &impl Fn(i64) -> i32) -> io::Result<()> {
    let directory = unit_directory()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no configuration directory"))?;
    let timer_path = directory.join(TIMER_UNIT);
    let service_path = directory.join(SERVICE_UNIT);
    let entries = calendar_entries(state, now, offset_at);
    if entries.is_empty() {
        if timer_path.exists() {
            let _ = systemctl(&["disable", "--now", TIMER_UNIT]);
            let _ = std::fs::remove_file(&timer_path);
            let _ = std::fs::remove_file(&service_path);
            systemctl(&["daemon-reload"])?;
        }
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let service = service_unit(&executable).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Clock's path cannot be scheduled",
        )
    })?;
    let timer = timer_unit(&entries);
    std::fs::create_dir_all(&directory)?;
    let unchanged = std::fs::read_to_string(&timer_path).is_ok_and(|text| text == timer)
        && std::fs::read_to_string(&service_path).is_ok_and(|text| text == service);
    if unchanged {
        return Ok(());
    }
    rmac_storage::atomic_write(&service_path, service.as_bytes())?;
    rmac_storage::atomic_write(&timer_path, timer.as_bytes())?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", TIMER_UNIT])?;
    // Restart so systemd recomputes the next elapse from the new lines.
    systemctl(&["restart", TIMER_UNIT])
}

#[cfg(target_os = "linux")]
fn systemctl(arguments: &[&str]) -> io::Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "systemctl --user {} failed",
            arguments.join(" ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alarms::{Alarm, Days};
    use crate::countdown::Countdown;

    const IST: i32 = 19_800;
    // 2026-09-23 12:14:30 IST in milliseconds.
    const NOW: u64 = ((20_719 * 86_400 + 6 * 3600 + 44 * 60 + 30) as u64) * 1000;

    fn ist(_: i64) -> i32 {
        IST
    }

    #[test]
    fn entries_cover_alarms_snoozes_and_timers() {
        let state = State {
            alarms: vec![
                Alarm {
                    id: 1,
                    hour: 7,
                    minute: 5,
                    ..Alarm::default()
                },
                Alarm {
                    id: 2,
                    hour: 6,
                    minute: 30,
                    repeat: Days::WEEKDAYS,
                    snoozed_until: Some((NOW / 1000) as i64 + 540),
                    ..Alarm::default()
                },
                Alarm {
                    id: 3,
                    enabled: false,
                    ..Alarm::default()
                },
            ],
            timers: vec![Countdown::start(4, 900_000, NOW - 500), {
                let mut paused = Countdown::start(5, 60_000, NOW);
                paused.pause(NOW);
                paused
            }],
            ..State::default()
        };
        assert_eq!(
            calendar_entries(&state, NOW, &ist),
            [
                "*-*-* 07:05:00",
                // The timer started 0.5 s ago ends at 12:29:29.5 → 12:29:30.
                "2026-09-23 12:29:30",
                // Snoozed nine minutes from 12:14:30.
                "2026-09-23 12:23:30",
                "Mon,Tue,Wed,Thu,Fri *-*-* 06:30:00",
            ]
            .iter()
            .map(|entry| entry.to_string())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn past_items_schedule_nothing() {
        let state = State {
            timers: vec![Countdown::start(1, 1_000, NOW - 5_000)],
            ..State::default()
        };
        assert!(calendar_entries(&state, NOW, &ist).is_empty());
    }

    #[test]
    fn units_are_well_formed() {
        let timer = timer_unit(&["*-*-* 07:00:00".into()]);
        assert!(timer.contains("\nOnCalendar=*-*-* 07:00:00\n"));
        assert!(timer.contains("\nAccuracySec=1s\n"));
        assert!(timer.contains(&format!("\nUnit={SERVICE_UNIT}\n")));
        assert!(timer.ends_with("WantedBy=timers.target\n"));
        let service = service_unit(Path::new("/usr/bin/rmac-clock")).unwrap();
        assert!(service.contains("\nExecStart=/usr/bin/rmac-clock --ring-due\n"));
        assert!(service_unit(Path::new("/home/me/my apps/rmac-clock")).is_none());
        assert!(service_unit(Path::new("relative/rmac-clock")).is_none());
        assert!(service_unit(Path::new("/tmp/%h")).is_none());
    }

    #[test]
    fn plan_task_covers_daily_weekly_and_once_entries() {
        assert_eq!(
            plan_task("*-*-* 07:05:00"),
            Some(TaskSchedule::Daily {
                time: "07:05:00".into()
            })
        );
        assert_eq!(
            plan_task("Mon,Tue,Wed,Thu,Fri *-*-* 06:30:00"),
            Some(TaskSchedule::Weekly {
                days: "MON,TUE,WED,THU,FRI".into(),
                time: "06:30:00".into()
            })
        );
        assert_eq!(
            plan_task("2026-09-23 12:29:30"),
            Some(TaskSchedule::Once {
                date: "09/23/2026".into(),
                time: "12:29:30".into()
            })
        );
        assert_eq!(plan_task("garbage"), None);
        assert_eq!(plan_task(""), None);
    }

    #[test]
    fn task_names_are_distinct_and_tagged() {
        assert_eq!(task_name(0), "RmacClockAlarm-00");
        assert_ne!(task_name(0), task_name(1));
    }
}
