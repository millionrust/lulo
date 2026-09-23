//! Alarms and timers ring through a user systemd timer, so they sound even
//! when Clock is not running and survive logout and reboot.
//!
//! Clock writes one timer unit whose `OnCalendar=` lines are every enabled
//! alarm, snooze and running countdown, and one service that runs
//! `rmac-clock --ring-due`. The ring process re-reads the state, rings what
//! is due, updates the state and rewrites the timer. Nothing stays resident.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::format::WallTime;
use crate::store::State;

pub const TIMER_UNIT: &str = "rmac-clock-alarms.timer";
pub const SERVICE_UNIT: &str = "rmac-clock-ring.service";
/// Argument that makes the Clock binary ring and exit without a window.
pub const RING_ARGUMENT: &str = "--ring-due";

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

/// Write the units for `state` and (re)arm or disarm the timer.
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
}
