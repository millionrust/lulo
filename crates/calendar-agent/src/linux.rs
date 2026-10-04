//! Linux runtime: reads alarms from EDS through `rmac-calendar-eds` and
//! `rmac-calendar-runtime`, arms one `CLOCK_REALTIME` timerfd for the next
//! alarm, and posts Lulo notifications with Mac-like actions through the
//! existing freedesktop `Notifications` service (ADR 0022 §6/§7).
//!
//! Not unit tested: this module is the thin, mostly-untestable I/O shell
//! around the pure logic in `crate::engine`. It is verified on the
//! reference laptop (0 wakeups between alerts, a banner on time) per
//! `docs/design/calendar-mail.md` CAL-7.
//!
//! Architecture, in one paragraph: a watcher thread per data source
//! (one per enabled calendar's EDS view, one for EDS source add/remove/
//! enable/disable, one for logind `PrepareForSleep`, one for notification
//! `ActionInvoked`, and one blocking on the timerfd) all funnel into a
//! single channel the main loop drains. A source-list change is rare and
//! EDS's blocking API gives no clean way to tear down a live view watcher
//! mid-process, so the simplest correct response is to exit and let
//! systemd (`Restart=on-success`) start a fresh process with a fresh
//! source list -- still zero wakeups between alerts, just like a normal
//! re-arm.

use std::collections::BTreeMap;
use std::io;
use std::process::Stdio;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use rmac_calendar_eds::Eds;
use rmac_calendar_runtime::CalendarRuntime;
use rmac_calendar_store::Calendar as IcalCalendar;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::Value;

use crate::alarms::ActiveAlarm;
use crate::engine::{self, Evaluation};
use crate::state::{self, AlertState};

const APP_NAME: &str = "Calendar";
const APP_ID: &str = "org.rmac.Calendar";
const CALENDAR_EXECUTABLE: &str = "rmac-calendar";
/// The Mac's exact snooze duration needs verifying there (ADR 0022 §6 says
/// "verify on Mac"); 5 minutes matches the classic Reminders default.
const SNOOZE_DURATION: chrono::Duration = chrono::Duration::minutes(5);
const RECONNECT_DELAY: StdDuration = StdDuration::from_secs(5);

enum Event {
    CalendarSnapshot {
        calendar_uid: String,
        objects: BTreeMap<String, String>,
    },
    SourcesChanged,
    Resumed,
    TimerFired,
    ClockChanged,
    Action {
        notification_id: u32,
        action: String,
    },
}

/// What a posted notification's id maps back to, so an action on it can
/// find the exact alarm again.
struct Target {
    calendar_uid: String,
    event_uid: String,
    occurrence_start: DateTime<Utc>,
    trigger_at: DateTime<Utc>,
}

pub fn run() -> io::Result<()> {
    let eds = Eds::session().map_err(|_| io::Error::other("Calendar service unavailable"))?;
    eds.check_available()
        .map_err(|_| io::Error::other("Calendar service unavailable"))?;

    let (tx, rx) = mpsc::channel::<Event>();
    spawn_source_watch(eds.clone(), tx.clone());
    spawn_resume_watch(tx.clone());
    spawn_action_watch(tx.clone());
    let timer = Timer::create()?;
    spawn_timer_watch(timer.fd.clone(), tx.clone());
    start_calendar_watchers(&eds, &tx).map_err(io::Error::other)?;

    let mut calendars: BTreeMap<String, IcalCalendar> = BTreeMap::new();
    let mut state = state::load().unwrap_or_default();
    let mut targets: BTreeMap<u32, Target> = BTreeMap::new();

    loop {
        match rx.recv() {
            Ok(Event::CalendarSnapshot {
                calendar_uid,
                objects,
            }) => {
                calendars.insert(calendar_uid, parse_objects(&objects));
            }
            Ok(Event::SourcesChanged) => return Ok(()),
            Ok(Event::Resumed) | Ok(Event::ClockChanged) | Ok(Event::TimerFired) => {}
            Ok(Event::Action {
                notification_id,
                action,
            }) => {
                handle_action(&mut state, &mut targets, notification_id, &action);
            }
            // Every watcher thread holds a clone of `tx`; the channel only
            // closes once this process is already tearing down.
            Err(_) => return Ok(()),
        }
        let now = Utc::now();
        let Evaluation { due, next_wake } =
            engine::evaluate(now, &calendars, chrono_tz::UTC, &mut state)
                .map_err(io::Error::other)?;
        for alarm in due {
            if let Some(id) = post_notification(&alarm) {
                targets.insert(
                    id,
                    Target {
                        calendar_uid: alarm.calendar_uid,
                        event_uid: alarm.event_uid,
                        occurrence_start: alarm.occurrence_start,
                        trigger_at: alarm.trigger_at,
                    },
                );
            }
        }
        if let Err(error) = state::save(&state) {
            eprintln!("rmac-calendar-agent: could not save alert state: {error}");
        }
        timer.arm(next_wake)?;
    }
}

fn parse_objects(objects: &BTreeMap<String, String>) -> IcalCalendar {
    let mut events = Vec::new();
    for raw in objects.values() {
        let wrapped = if raw.contains("BEGIN:VCALENDAR") {
            raw.clone()
        } else {
            format!("BEGIN:VCALENDAR\nVERSION:2.0\n{raw}\nEND:VCALENDAR")
        };
        if let Ok(parsed) = IcalCalendar::parse(&wrapped) {
            events.extend(parsed.events);
        }
    }
    IcalCalendar { events }
}

/// One thread per enabled calendar source, each with its own
/// `CalendarRuntime` so no lock is needed across threads.
fn start_calendar_watchers(eds: &Eds, tx: &Sender<Event>) -> Result<(), String> {
    let mut runtime = CalendarRuntime::new(eds.clone());
    let sources = runtime.reload_sources()?.to_vec();
    for source in sources.into_iter().filter(|source| source.enabled) {
        let eds = eds.clone();
        let tx = tx.clone();
        thread::spawn(move || watch_calendar(eds, source.uid, tx));
    }
    Ok(())
}

fn watch_calendar(eds: Eds, uid: String, tx: Sender<Event>) {
    let mut runtime = CalendarRuntime::new(eds);
    if runtime.reload_sources().is_err() {
        return;
    }
    let objects = match runtime.open(&uid) {
        Ok(snapshot) => snapshot.objects.clone(),
        Err(_) => return,
    };
    if tx
        .send(Event::CalendarSnapshot {
            calendar_uid: uid.clone(),
            objects,
        })
        .is_err()
    {
        return;
    }
    loop {
        let objects = match runtime.next_event(&uid) {
            Ok(Some(snapshot)) => snapshot.objects.clone(),
            Ok(None) | Err(_) => return,
        };
        if tx
            .send(Event::CalendarSnapshot {
                calendar_uid: uid.clone(),
                objects,
            })
            .is_err()
        {
            return;
        }
    }
}

/// EDS source add/remove/enable/disable: see this module's doc comment for
/// why the response is a clean process exit rather than live reconfiguration.
fn spawn_source_watch(eds: Eds, tx: Sender<Event>) {
    thread::spawn(move || loop {
        match eds.source_changes() {
            Ok(stream) => {
                for () in stream {
                    if tx.send(Event::SourcesChanged).is_err() {
                        return;
                    }
                }
                return;
            }
            Err(_) => thread::sleep(RECONNECT_DELAY),
        }
    });
}

/// logind's `PrepareForSleep(false)`: the moment resume finishes. A missed
/// alarm during suspend is shown the instant this fires, not whenever the
/// timerfd next happens to expire on its own.
fn spawn_resume_watch(tx: Sender<Event>) {
    thread::spawn(move || loop {
        if let Ok(connection) = Connection::system() {
            let rule = "type='signal',sender='org.freedesktop.login1',\
                        path='/org/freedesktop/login1',\
                        interface='org.freedesktop.login1.Manager',\
                        member='PrepareForSleep'";
            if let Ok(signals) = MessageIterator::for_match_rule(rule, &connection, Some(8)) {
                for signal in signals {
                    let Ok(signal) = signal else { break };
                    if let Ok(preparing) = signal.body().deserialize::<bool>() {
                        if !preparing && tx.send(Event::Resumed).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        thread::sleep(RECONNECT_DELAY);
    });
}

fn spawn_action_watch(tx: Sender<Event>) {
    thread::spawn(move || loop {
        if let Ok(connection) = Connection::session() {
            let rule = "type='signal',sender='org.freedesktop.Notifications',\
                        path='/org/freedesktop/Notifications',\
                        interface='org.freedesktop.Notifications',\
                        member='ActionInvoked'";
            if let Ok(signals) = MessageIterator::for_match_rule(rule, &connection, Some(64)) {
                for signal in signals {
                    let Ok(signal) = signal else { break };
                    if let Ok((notification_id, action)) =
                        signal.body().deserialize::<(u32, String)>()
                    {
                        if tx
                            .send(Event::Action {
                                notification_id,
                                action,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
        thread::sleep(RECONNECT_DELAY);
    });
}

fn handle_action(
    state: &mut AlertState,
    targets: &mut BTreeMap<u32, Target>,
    id: u32,
    action: &str,
) {
    let Some(target) = targets.remove(&id) else {
        return;
    };
    match action {
        "snooze" => engine::snooze(
            state,
            &target.calendar_uid,
            &target.event_uid,
            target.occurrence_start,
            target.trigger_at,
            Utc::now() + SNOOZE_DURATION,
        ),
        "close" => engine::close(
            state,
            &target.calendar_uid,
            &target.event_uid,
            target.occurrence_start,
            target.trigger_at,
        ),
        "default" => {
            engine::close(
                state,
                &target.calendar_uid,
                &target.event_uid,
                target.occurrence_start,
                target.trigger_at,
            );
            open_calendar();
        }
        _ => {}
    }
}

/// A transient unit so the agent's own service lifecycle never owns
/// Calendar's window (mirrors `rmac-clock`'s `--ring-due` -> `open_clock`).
fn open_calendar() {
    let _ = std::process::Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet", "--"])
        .arg(CALENDAR_EXECUTABLE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// The event's own time (as the Mac's alert banner shows it), not the
/// alarm's earlier trigger time.
fn alarm_body(alarm: &ActiveAlarm) -> String {
    if alarm.all_day {
        "All day".to_owned()
    } else {
        alarm.occurrence_start.format("%-I:%M %p").to_string()
    }
}

/// Posts through the same freedesktop service Lulo's notification daemon
/// owns, with Mac-like actions: Snooze, Close, and a default action (click
/// the banner) that opens the event in Calendar. The banner daemon already
/// queues notifications silently while the screen is locked (SR-38), so
/// posting plainly here is enough.
fn post_notification(alarm: &ActiveAlarm) -> Option<u32> {
    let connection = Connection::session().ok()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .ok()?;
    let mut hints: std::collections::HashMap<&str, Value<'_>> = std::collections::HashMap::new();
    hints.insert("desktop-entry", Value::from(APP_ID));
    hints.insert("category", Value::from("calendar.alarm"));
    let body = alarm_body(alarm);
    let actions = vec!["default", "Open", "snooze", "Snooze", "close", "Close"];
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                APP_NAME,
                0_u32,
                APP_ID,
                alarm.summary.as_str(),
                body.as_str(),
                actions,
                hints,
                0_i32,
            ),
        )
        .ok()?;
    Some(id)
}

/// One `CLOCK_REALTIME` timerfd, re-armed by whichever thread computed the
/// next deadline. `TFD_TIMER_CANCEL_ON_SET` wakes the watcher thread (with
/// `ECANCELED`) on a discontinuous clock change instead of sleeping past it.
struct Timer {
    fd: Arc<rustix::fd::OwnedFd>,
}

impl Timer {
    fn create() -> io::Result<Self> {
        use rustix::time::{timerfd_create, TimerfdClockId, TimerfdFlags};
        let fd = timerfd_create(TimerfdClockId::Realtime, TimerfdFlags::CLOEXEC)?;
        Ok(Self { fd: Arc::new(fd) })
    }

    /// `None` disarms the timer: the watcher thread's blocking read then
    /// never returns until the next `arm`, which is the zero-wakeups-while-
    /// idle state this agent is budgeted for.
    fn arm(&self, deadline: Option<DateTime<Utc>>) -> io::Result<()> {
        use rustix::time::{timerfd_settime, Itimerspec, TimerfdTimerFlags, Timespec};
        let it_value = match deadline {
            Some(at) => Timespec {
                tv_sec: at.timestamp(),
                tv_nsec: i64::from(at.timestamp_subsec_nanos()),
            },
            None => Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };
        timerfd_settime(
            &*self.fd,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &Itimerspec {
                it_interval: Timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                },
                it_value,
            },
        )
        .map(|_| ())
        .map_err(io::Error::from)
    }
}

fn spawn_timer_watch(fd: Arc<rustix::fd::OwnedFd>, tx: Sender<Event>) {
    thread::spawn(move || loop {
        let mut buffer = [0_u8; 8];
        match rustix::io::read(&*fd, &mut buffer) {
            Ok(_) => {
                if tx.send(Event::TimerFired).is_err() {
                    return;
                }
            }
            Err(rustix::io::Errno::CANCELED) => {
                if tx.send(Event::ClockChanged).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    });
}
