//! Runs Message ▸ Send Later and Message ▸ Remind Me on time.
//!
//! One timer waits for the earliest entry in Mail's schedule; nothing polls.
//! Every change, and every time a Mail window comes to the front (a laptop
//! that slept past a deadline), re-reads the file off the UI thread, does
//! whatever is due and re-arms the timer. A newer check retires the timer
//! of an older one.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{App, Global};
use rmac_mail::schedule::Schedule;

/// Serialises every read-modify-write of the schedule file across the
/// background threads that touch it.
static FILE: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct Armed {
    generation: u64,
    /// The messages with a Remind Me waiting, for the menu's Clear Reminder.
    reminders: HashSet<String>,
}

impl Global for Armed {}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether `message_id` has a Remind Me waiting.
pub fn has_reminder(message_id: &str, cx: &App) -> bool {
    cx.try_global::<Armed>()
        .is_some_and(|armed| armed.reminders.contains(message_id))
}

/// Changes the schedule (off the UI thread), then checks it.
pub fn change(cx: &mut App, edit: impl FnOnce(&mut Schedule) + Send + 'static) {
    let Some(root) = crate::delivery::schedule_root() else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor()
            .spawn(async move {
                let _guard = FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut schedule = Schedule::load(&root);
                edit(&mut schedule);
                let _ = schedule.save(&root);
            })
            .await;
        let _ = cx.update(check);
    })
    .detach();
}

/// Does what is due now and arms the timer for the next entry.
pub fn check(cx: &mut App) {
    let Some(root) = crate::delivery::schedule_root() else {
        return;
    };
    let generation = {
        let armed = cx.default_global::<Armed>();
        armed.generation += 1;
        armed.generation
    };
    cx.spawn(async move |cx| {
        let (sends, reminders, waiting, next) = cx
            .background_executor()
            .spawn(async move {
                let _guard = FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut schedule = Schedule::load(&root);
                let (sends, reminders) = schedule.take_due(now());
                if !sends.is_empty() || !reminders.is_empty() {
                    let _ = schedule.save(&root);
                }
                let waiting: HashSet<String> = schedule
                    .reminders
                    .iter()
                    .map(|reminder| reminder.message_id.clone())
                    .collect();
                (sends, reminders, waiting, schedule.next_due())
            })
            .await;
        if !sends.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    let accounts = crate::delivery::accounts();
                    for entry in &sends {
                        let _ = crate::delivery::send_scheduled(&accounts, entry);
                    }
                })
                .detach();
        }
        let _ = cx.update(|cx| {
            cx.default_global::<Armed>().reminders = waiting;
            for view in crate::view::viewers() {
                view.update(cx, |view, cx| view.reminders_due(&reminders, cx));
            }
        });
        let Some(next) = next else {
            return;
        };
        // Wake at the deadline (at most a day ahead; a check then re-arms).
        let wait = (next - now()).clamp(1, 86_400) as u64;
        cx.background_executor()
            .timer(Duration::from_secs(wait))
            .await;
        let _ = cx.update(|cx| {
            if cx.global::<Armed>().generation == generation {
                check(cx);
            }
        });
    })
    .detach();
}
