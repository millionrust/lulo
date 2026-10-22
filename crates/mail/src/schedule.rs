//! Message ▸ Send Later and Message ▸ Remind Me: what is due when.
//!
//! Both live in one small JSON file in Mail's data folder. Mail arms a
//! single timer for the earliest entry (no polling) and checks again when
//! a window comes to the front, so a computer that slept past a deadline
//! catches up as soon as it is used. The scheduled message itself is kept
//! as a ready-to-send `.eml` beside the file (`delivery::schedule`), and
//! its draft stays in Drafts until it goes, as on the Mac.

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One message waiting for its Send Later time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledSend {
    pub id: Uuid,
    pub account: Uuid,
    /// When to send, in Unix seconds.
    pub at: i64,
    pub subject: String,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    /// The autosaved draft to remove once it is sent: (mailbox id, uid).
    pub draft: Option<(i64, i64)>,
}

/// One Remind Me: the message comes back to the top of the list, unread,
/// at `at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub message_id: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub sends: Vec<ScheduledSend>,
    #[serde(default)]
    pub reminders: Vec<Reminder>,
}

impl Schedule {
    pub fn path(root: &Path) -> PathBuf {
        root.join("schedule.json")
    }

    /// The scheduled message's source, kept beside the file.
    pub fn message_path(root: &Path, id: Uuid) -> PathBuf {
        root.join("send-later").join(format!("{id}.eml"))
    }

    /// A missing or unreadable file is an empty schedule.
    pub fn load(root: &Path) -> Self {
        std::fs::read(Self::path(root))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let path = Self::path(root);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)
    }

    /// The earliest deadline, if anything is waiting.
    pub fn next_due(&self) -> Option<i64> {
        self.sends
            .iter()
            .map(|send| send.at)
            .chain(self.reminders.iter().map(|reminder| reminder.at))
            .min()
    }

    /// Removes and returns everything due at `now`.
    pub fn take_due(&mut self, now: i64) -> (Vec<ScheduledSend>, Vec<Reminder>) {
        let (due_sends, sends) = std::mem::take(&mut self.sends)
            .into_iter()
            .partition(|send| send.at <= now);
        let (due_reminders, reminders) = std::mem::take(&mut self.reminders)
            .into_iter()
            .partition(|reminder| reminder.at <= now);
        self.sends = sends;
        self.reminders = reminders;
        (due_sends, due_reminders)
    }

    /// Sets (or moves) the reminder for `message_id`.
    pub fn remind(&mut self, message_id: &str, at: i64) {
        self.clear_reminder(message_id);
        self.reminders.push(Reminder {
            message_id: message_id.to_owned(),
            at,
        });
    }

    /// Message ▸ Remind Me ▸ Clear Reminder. True when there was one.
    pub fn clear_reminder(&mut self, message_id: &str) -> bool {
        let before = self.reminders.len();
        self.reminders
            .retain(|reminder| reminder.message_id != message_id);
        self.reminders.len() != before
    }

    pub fn reminder_for(&self, message_id: &str) -> Option<i64> {
        self.reminders
            .iter()
            .find(|reminder| reminder.message_id == message_id)
            .map(|reminder| reminder.at)
    }
}

/// The preset times the Mac offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Remind Me in 1 Hour.
    InOneHour,
    /// 9:00 PM tonight (tomorrow night once it is past nine).
    Tonight,
    /// 8:00 AM tomorrow.
    TomorrowMorning,
}

impl Preset {
    pub fn at(self, now: i64) -> i64 {
        let Some(local) = Local.timestamp_opt(now, 0).single() else {
            return now + 3600;
        };
        let at_time = |date: NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap()))
                .earliest()
                .map(|when| when.timestamp())
        };
        let today = local.date_naive();
        match self {
            Self::InOneHour => now + 3600,
            Self::Tonight => {
                let tonight = at_time(today, 21).unwrap_or(now + 3600);
                if tonight > now {
                    tonight
                } else {
                    at_time(today + Duration::days(1), 21).unwrap_or(now + 86_400)
                }
            }
            Self::TomorrowMorning => at_time(today + Duration::days(1), 8).unwrap_or(now + 86_400),
        }
    }
}

/// The date and time typed into a "Later…" sheet, such as "23 Oct 2026
/// 9:00", "2026-10-23 21:30" or "23/10/2026 9:00 PM", as Unix seconds.
pub fn parse_when(text: &str) -> Option<i64> {
    let text = text.trim().replace(',', " ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const FORMATS: &[&str] = &[
        "%d %b %Y %H:%M",
        "%d %b %Y %I:%M %p",
        "%d %B %Y %H:%M",
        "%d %B %Y %I:%M %p",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %I:%M %p",
        "%d/%m/%Y %H:%M",
        "%d/%m/%Y %I:%M %p",
    ];
    FORMATS.iter().find_map(|format| {
        NaiveDateTime::parse_from_str(&text, format)
            .ok()
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .map(|when| when.timestamp())
    })
}

/// How a "Later…" sheet first fills its field: `at` as "23 Oct 2026 08:00".
pub fn format_when(at: i64) -> String {
    Local
        .timestamp_opt(at, 0)
        .single()
        .map(|when| when.format("%-d %b %Y %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn presets_follow_the_mac() {
        let afternoon = local(2026, 10, 22, 15, 0);
        assert_eq!(Preset::InOneHour.at(afternoon), afternoon + 3600);
        assert_eq!(Preset::Tonight.at(afternoon), local(2026, 10, 22, 21, 0));
        assert_eq!(
            Preset::TomorrowMorning.at(afternoon),
            local(2026, 10, 23, 8, 0)
        );
        let late = local(2026, 10, 22, 22, 30);
        assert_eq!(Preset::Tonight.at(late), local(2026, 10, 23, 21, 0));
    }

    #[test]
    fn typed_times_parse_in_the_usual_spellings() {
        let expected = local(2026, 10, 23, 21, 30);
        for text in [
            "23 Oct 2026 21:30",
            "23 Oct 2026, 9:30 PM",
            "23 October 2026 21:30",
            "2026-10-23 21:30",
            "23/10/2026 9:30 PM",
        ] {
            assert_eq!(parse_when(text), Some(expected), "{text}");
        }
        assert_eq!(parse_when("next week"), None);
        assert_eq!(parse_when(&format_when(expected)), Some(expected));
    }

    #[test]
    fn the_earliest_entry_is_next_and_due_ones_are_taken_once() {
        let mut schedule = Schedule::default();
        assert_eq!(schedule.next_due(), None);
        schedule.remind("a", 300);
        schedule.remind("b", 100);
        schedule.sends.push(ScheduledSend {
            id: Uuid::nil(),
            account: Uuid::nil(),
            at: 200,
            subject: "Hello".into(),
            envelope_from: "me@example.test".into(),
            envelope_to: vec!["you@example.test".into()],
            draft: None,
        });
        assert_eq!(schedule.next_due(), Some(100));
        let (sends, reminders) = schedule.take_due(200);
        assert_eq!(sends.len(), 1);
        assert_eq!(
            reminders,
            vec![Reminder {
                message_id: "b".into(),
                at: 100
            }]
        );
        assert_eq!(schedule.next_due(), Some(300));
        assert_eq!(schedule.take_due(200), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_message_has_one_reminder_and_it_can_be_cleared() {
        let mut schedule = Schedule::default();
        schedule.remind("a", 100);
        schedule.remind("a", 500);
        assert_eq!(schedule.reminders.len(), 1);
        assert_eq!(schedule.reminder_for("a"), Some(500));
        assert!(schedule.clear_reminder("a"));
        assert!(!schedule.clear_reminder("a"));
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let root = std::env::temp_dir().join(format!("rmac-mail-schedule-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(Schedule::load(&root), Schedule::default());
        let mut schedule = Schedule::default();
        schedule.remind("a", 42);
        schedule.save(&root).unwrap();
        assert_eq!(Schedule::load(&root), schedule);
        let _ = std::fs::remove_dir_all(&root);
    }
}
