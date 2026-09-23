//! `rmac-clock --ring-due`: started by the user systemd timer (see
//! `rmac_clock::schedule`). It rings every alarm and timer that is due,
//! records that in the shared state, re-arms the timer and exits. No window
//! and no GPUI: the process lives only while something rings.

use std::fs::OpenOptions;
use std::process::ExitCode;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use rmac_clock::alarms::Ring;
use rmac_clock::format::{time_12h, WallTime};
use rmac_clock::{now_millis, schedule, store, tz};

/// How long a ring sounds before the notification is left to sit quietly
/// (S: the Mac keeps ringing until dismissed; one minute bounds the noise
/// for a machine left unattended).
const RING_FOR: Duration = Duration::from_secs(60);
/// Pause between repeats of the alert sound.
const REPEAT_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
enum Ringing {
    Alarm {
        id: u64,
        title: String,
        time: String,
        snooze: bool,
    },
    Timer {
        label: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Stop,
    Snooze,
    Open,
}

pub fn run() -> ExitCode {
    // One ringer at a time: a second start while ringing just exits, and the
    // running one re-checks for new work before it leaves.
    let Some(path) = store::state_path() else {
        return ExitCode::FAILURE;
    };
    let Some(parent) = path.parent() else {
        return ExitCode::FAILURE;
    };
    if rmac_storage::create_dir_all_private(parent).is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("ring.lock"))
    else {
        return ExitCode::FAILURE;
    };
    if lock.try_lock().is_err() {
        return ExitCode::SUCCESS;
    }
    loop {
        let due = match collect_due() {
            Ok(due) => due,
            Err(error) => {
                eprintln!("rmac-clock: could not read alarms: {error}");
                return ExitCode::FAILURE;
            }
        };
        if due.is_empty() {
            break;
        }
        for ringing in due {
            let outcome = ring(&ringing);
            match (outcome, &ringing) {
                (Some(Outcome::Snooze), Ringing::Alarm { id, .. }) => snooze(*id),
                (Some(Outcome::Open), _) => open_clock(),
                _ => {}
            }
        }
    }
    let _ = lock.unlock();
    ExitCode::SUCCESS
}

/// Mark everything due as rung (one-time alarms switch off, finished timers
/// go away), re-arm the systemd timer and return what to ring.
fn collect_due() -> std::io::Result<Vec<Ringing>> {
    let zone = tz::local_zone();
    let offset = |utc: i64| zone.offset_at(utc);
    let now = now_millis();
    let now_seconds = (now / 1000) as i64;
    let (state, due) = store::update(|state| {
        let mut due = Vec::new();
        for alarm in &mut state.alarms {
            if let Some(ring) = alarm.due(now_seconds, &offset) {
                let at = match ring {
                    Ring::Scheduled(time) | Ring::Snoozed(time) => time,
                };
                let wall = WallTime::at(at, offset(at));
                due.push(Ringing::Alarm {
                    id: alarm.id,
                    title: alarm.title().to_owned(),
                    time: time_12h(wall.hour, wall.minute),
                    snooze: alarm.snooze,
                });
                alarm.mark_rung(ring);
            }
        }
        state.timers.retain(|timer| {
            if timer.is_done(now) {
                due.push(Ringing::Timer {
                    label: timer.label.clone(),
                });
                false
            } else {
                true
            }
        });
        due
    })?;
    if let Err(error) = schedule::apply(&state, now, &offset) {
        eprintln!("rmac-clock: could not re-arm alarms: {error}");
    }
    Ok(due)
}

fn snooze(id: u64) {
    let zone = tz::local_zone();
    let now = now_millis();
    let saved = store::update(|state| {
        if let Some(alarm) = state.alarms.iter_mut().find(|alarm| alarm.id == id) {
            alarm.snooze_from((now / 1000) as i64);
        }
    });
    match saved {
        Ok((state, ())) => {
            if let Err(error) = schedule::apply(&state, now, &|utc| zone.offset_at(utc)) {
                eprintln!("rmac-clock: could not schedule the snooze: {error}");
            }
        }
        Err(error) => eprintln!("rmac-clock: could not save the snooze: {error}"),
    }
}

/// Open the Clock window in its own transient unit: a child of this service
/// would be stopped with it.
fn open_clock() {
    if let Ok(executable) = std::env::current_exe() {
        let _ = std::process::Command::new("systemd-run")
            .args(["--user", "--collect", "--quiet", "--"])
            .arg(executable)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// Post the notification and repeat the alert sound until it is answered
/// or [`RING_FOR`] passes.
fn ring(ringing: &Ringing) -> Option<Outcome> {
    let (summary, body, snooze) = match ringing {
        Ringing::Alarm {
            title,
            time,
            snooze,
            ..
        } => (title.clone(), time.clone(), *snooze),
        Ringing::Timer { label } => (
            if label.trim().is_empty() {
                "Timer".to_owned()
            } else {
                label.clone()
            },
            "Timer Done".to_owned(),
            false,
        ),
    };
    let posted = notification::post(&summary, &body, snooze);
    let started = Instant::now();
    let mut outcome = None;
    while started.elapsed() < RING_FOR {
        rmac_sound::play_blocking(rmac_sound::Cue::Alert);
        match posted
            .as_ref()
            .map(|(_, answers)| answers.recv_timeout(REPEAT_EVERY))
        {
            Some(Ok(answer)) => {
                outcome = Some(answer);
                break;
            }
            Some(Err(RecvTimeoutError::Timeout)) => {}
            // No notification service, or it went away: keep the rhythm.
            Some(Err(RecvTimeoutError::Disconnected)) | None => std::thread::sleep(REPEAT_EVERY),
        }
    }
    outcome
}

#[cfg(target_os = "linux")]
mod notification {
    use std::collections::HashMap;
    use std::sync::mpsc::{channel, Receiver};

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::Value;

    use super::Outcome;

    const NAME: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";

    /// Post one ringing notification; returns its id and the user's answer.
    pub fn post(summary: &str, body: &str, snooze: bool) -> Option<(u32, Receiver<Outcome>)> {
        let connection = Connection::session().ok()?;
        let proxy = Proxy::new(&connection, NAME, PATH, NAME).ok()?;
        // Subscribe before posting so a fast answer is never missed.
        let invoked = proxy.receive_signal("ActionInvoked").ok()?;
        let closed = proxy.receive_signal("NotificationClosed").ok()?;
        let mut actions = vec!["default", "Open", "stop", "Stop"];
        if snooze {
            actions.extend(["snooze", "Snooze"]);
        }
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        hints.insert("urgency", Value::from(2u8));
        hints.insert("category", Value::from("alarm.ringing"));
        hints.insert("resident", Value::from(true));
        hints.insert("suppress-sound", Value::from(true));
        hints.insert("desktop-entry", Value::from("org.rmac.Clock"));
        let id: u32 = proxy
            .call(
                "Notify",
                &(
                    "Clock",
                    0u32,
                    "org.rmac.Clock",
                    summary,
                    body,
                    actions,
                    hints,
                    0i32,
                ),
            )
            .ok()?;
        let (sender, answers) = channel();
        let invoked_sender = sender.clone();
        std::thread::spawn(move || {
            for message in invoked {
                let Ok((notification, action)) = message.body().deserialize::<(u32, String)>()
                else {
                    continue;
                };
                if notification != id {
                    continue;
                }
                let outcome = match action.as_str() {
                    "snooze" => Outcome::Snooze,
                    "default" => Outcome::Open,
                    _ => Outcome::Stop,
                };
                let _ = invoked_sender.send(outcome);
                break;
            }
        });
        std::thread::spawn(move || {
            for message in closed {
                if let Ok((notification, _reason)) = message.body().deserialize::<(u32, u32)>() {
                    if notification == id {
                        let _ = sender.send(Outcome::Stop);
                        break;
                    }
                }
            }
        });
        Some((id, answers))
    }
}

#[cfg(not(target_os = "linux"))]
mod notification {
    use std::sync::mpsc::Receiver;

    use super::Outcome;

    pub fn post(_summary: &str, _body: &str, _snooze: bool) -> Option<(u32, Receiver<Outcome>)> {
        None
    }
}
