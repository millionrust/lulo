//! Mailbox ▸ New Smart Mailbox…: saved searches that show every message
//! matching their conditions, in any account, as Mac Mail's smart
//! mailboxes do. They live on this computer only
//! (`smart-mailboxes.json` in Mail's data folder), never on a server.
//!
//! Also the in-memory side of New, Rename and Delete Mailbox, which the
//! window applies at once while `rmac-mail-storage` queues the change for
//! the server.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{MailState, Mailbox, Message, RealMailbox, SpecialUse};

/// The file a person's smart mailboxes are kept in, inside Mail's data
/// folder.
pub const SMART_MAILBOXES_FILE: &str = "smart-mailboxes.json";

/// Most conditions one smart mailbox may hold (the sheet's + button stops
/// there), and most smart mailboxes a person may keep.
pub const MAX_CONDITIONS: usize = 12;
pub const MAX_SMART_MAILBOXES: usize = 200;

/// Which text a "contains" condition searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartField {
    From,
    To,
    Subject,
    EntireMessage,
}

/// One row of the smart mailbox sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SmartCondition {
    /// "From/To/Subject/Entire Message contains …".
    Contains { field: SmartField, text: String },
    /// "Message is Unread".
    Unread,
    /// "Message is Flagged".
    Flagged,
    /// "Contains Attachments".
    HasAttachment,
    /// "Date Received is in the last N days".
    ReceivedWithinDays { days: u32 },
}

impl SmartCondition {
    /// The first row a new smart mailbox starts with, as on the Mac.
    pub fn default_row() -> Self {
        Self::Contains {
            field: SmartField::From,
            text: String::new(),
        }
    }

    pub fn matches(&self, message: &Message, now: i64) -> bool {
        match self {
            Self::Contains { field, text } => {
                let needle = text.trim().to_lowercase();
                if needle.is_empty() {
                    return false;
                }
                let contains = |haystack: &str| haystack.to_lowercase().contains(&needle);
                match field {
                    SmartField::From => {
                        contains(&message.sender) || contains(&message.sender_address)
                    }
                    SmartField::To => {
                        contains(&message.to)
                            || contains(&message.to_addresses)
                            || contains(&message.cc)
                            || contains(&message.cc_addresses)
                    }
                    SmartField::Subject => contains(&message.subject),
                    SmartField::EntireMessage => {
                        contains(&message.sender)
                            || contains(&message.sender_address)
                            || contains(&message.to)
                            || contains(&message.cc)
                            || contains(&message.subject)
                            || contains(&message.preview)
                            || contains(&message.body.plain_text())
                    }
                }
            }
            Self::Unread => message.unread,
            Self::Flagged => message.flagged,
            Self::HasAttachment => message.attachment.is_some(),
            Self::ReceivedWithinDays { days } => {
                now.saturating_sub(message.received_at) <= i64::from(*days) * 86_400
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartMailbox {
    pub id: u64,
    pub name: String,
    /// "Contains messages that match all (or any) of the following".
    pub match_all: bool,
    pub conditions: Vec<SmartCondition>,
}

impl SmartMailbox {
    /// Whether `message` belongs. Like the Mac's defaults, a smart mailbox
    /// leaves out messages in Bin and Junk, and one with no usable
    /// condition shows nothing.
    pub fn matches(&self, message: &Message, now: i64) -> bool {
        if matches!(
            message.mailbox.special_use(),
            Some(SpecialUse::Trash | SpecialUse::Junk)
        ) || self.conditions.is_empty()
        {
            return false;
        }
        if self.match_all {
            self.conditions
                .iter()
                .all(|condition| condition.matches(message, now))
        } else {
            self.conditions
                .iter()
                .any(|condition| condition.matches(message, now))
        }
    }
}

/// The current time in Unix seconds, for "Date Received" conditions.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Reads the saved smart mailboxes. A missing or unreadable file is an
/// empty list. Blocking: call off the UI thread.
pub fn load_smart_mailboxes(path: &Path) -> Vec<SmartMailbox> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let mut list: Vec<SmartMailbox> = serde_json::from_slice(&bytes).unwrap_or_default();
    list.truncate(MAX_SMART_MAILBOXES);
    for smart in &mut list {
        smart.conditions.truncate(MAX_CONDITIONS);
    }
    list
}

/// Writes the smart mailboxes, replacing the file in one step. Blocking.
pub fn save_smart_mailboxes(path: &Path, list: &[SmartMailbox]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(list).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// `name` with the last component replaced by `leaf`, keeping its parent.
pub fn renamed_path(name: &str, leaf: &str, delimiter: char) -> String {
    match name.rsplit_once(delimiter) {
        Some((parent, _)) => format!("{parent}{delimiter}{leaf}"),
        None => leaf.to_owned(),
    }
}

/// The last component of a mailbox path, as the sidebar and the Rename
/// sheet show it.
pub fn leaf_name(name: &str, delimiter: char) -> &str {
    name.rsplit_once(delimiter).map_or(name, |(_, leaf)| leaf)
}

impl MailState {
    /// The smart mailboxes in the sidebar, in order.
    pub fn smart_mailboxes(&self) -> Vec<SmartMailbox> {
        self.mailboxes
            .iter()
            .filter_map(|mailbox| match mailbox {
                Mailbox::Smart(smart) => Some(smart.clone()),
                _ => None,
            })
            .collect()
    }

    /// Replaces every smart mailbox, placed after the Favourites.
    pub fn set_smart_mailboxes(&mut self, list: Vec<SmartMailbox>) {
        self.mailboxes
            .retain(|mailbox| !matches!(mailbox, Mailbox::Smart(_)));
        let at = self
            .mailboxes
            .iter()
            .position(|mailbox| mailbox.is_real())
            .unwrap_or(self.mailboxes.len());
        self.mailboxes
            .splice(at..at, list.into_iter().map(Mailbox::Smart));
        if let Mailbox::Smart(current) = &self.mailbox {
            let id = current.id;
            match self.find_smart(id) {
                Some(updated) => self.mailbox = Mailbox::Smart(updated),
                None => self.mailbox = Mailbox::AllInboxes,
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }

    fn find_smart(&self, id: u64) -> Option<SmartMailbox> {
        self.mailboxes.iter().find_map(|mailbox| match mailbox {
            Mailbox::Smart(smart) if smart.id == id => Some(smart.clone()),
            _ => None,
        })
    }

    /// Adds or updates one smart mailbox, keeping its place.
    pub fn save_smart_mailbox(&mut self, smart: SmartMailbox) {
        let mut list = self.smart_mailboxes();
        match list.iter().position(|existing| existing.id == smart.id) {
            Some(index) => list[index] = smart,
            None if list.len() < MAX_SMART_MAILBOXES => list.push(smart),
            None => {}
        }
        self.set_smart_mailboxes(list);
    }

    pub fn remove_smart_mailbox(&mut self, id: u64) {
        let list = self
            .smart_mailboxes()
            .into_iter()
            .filter(|smart| smart.id != id)
            .collect();
        self.set_smart_mailboxes(list);
    }

    /// An id no smart mailbox uses yet.
    pub fn next_smart_id(&self) -> u64 {
        self.smart_mailboxes()
            .iter()
            .map(|smart| smart.id)
            .max()
            .map_or(1, |id| id + 1)
    }

    /// "Smart Mailbox 1", "Smart Mailbox 2", … for a new one's name.
    pub fn next_smart_name(&self) -> String {
        let taken = self.smart_mailboxes();
        (1..)
            .map(|index| format!("Smart Mailbox {index}"))
            .find(|name| !taken.iter().any(|smart| &smart.name == name))
            .unwrap_or_default()
    }

    /// Mailbox ▸ Duplicate Smart Mailbox: a copy named "… Copy", placed
    /// after the original and selected.
    pub fn duplicate_smart_mailbox(&mut self, id: u64) -> Option<SmartMailbox> {
        let original = self.find_smart(id)?;
        let mut list = self.smart_mailboxes();
        if list.len() >= MAX_SMART_MAILBOXES {
            return None;
        }
        let copy = SmartMailbox {
            id: self.next_smart_id(),
            name: format!("{} Copy", original.name),
            ..original
        };
        let at = list
            .iter()
            .position(|smart| smart.id == id)
            .map_or(list.len(), |index| index + 1);
        list.insert(at, copy.clone());
        self.set_smart_mailboxes(list);
        Some(copy)
    }

    /// The mailbox Rename and Delete Mailbox act on: the open one, if it is
    /// a smart mailbox or a real mailbox of the person's own (never Inbox,
    /// Drafts, Sent, Junk, Bin or Archive).
    pub fn editable_mailbox(&self) -> Option<&Mailbox> {
        match &self.mailbox {
            Mailbox::Smart(_) => Some(&self.mailbox),
            Mailbox::Real(real)
                if real.special_use.is_none() && !real.name.eq_ignore_ascii_case("INBOX") =>
            {
                Some(&self.mailbox)
            }
            _ => None,
        }
    }

    /// New Mailbox…: shows `mailbox` at once, after the rest of its
    /// account's mailboxes, and opens it.
    pub fn add_real_mailbox(&mut self, mailbox: RealMailbox) {
        let account = mailbox.account;
        let at = self
            .mailboxes
            .iter()
            .rposition(
                |existing| matches!(existing, Mailbox::Real(real) if real.account == account),
            )
            .map_or(self.mailboxes.len(), |index| index + 1);
        let mailbox = Mailbox::Real(mailbox);
        self.mailboxes.insert(at, mailbox.clone());
        self.select_mailbox(mailbox);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Rename Mailbox…: the mailbox and its submailboxes take the new
    /// path, and so does every message in them.
    pub fn rename_real_mailbox(
        &mut self,
        account: Uuid,
        mailbox_id: i64,
        renamed: &str,
        delimiter: char,
    ) {
        let Some(old) = self.mailboxes.iter().find_map(|mailbox| match mailbox {
            Mailbox::Real(real) if real.account == account && real.mailbox_id == mailbox_id => {
                Some(real.name.clone())
            }
            _ => None,
        }) else {
            return;
        };
        let child_prefix = format!("{old}{delimiter}");
        let rename = |real: &mut RealMailbox| {
            if real.account != account {
                return;
            }
            if real.name == old {
                real.name = renamed.to_owned();
            } else if let Some(rest) = real.name.strip_prefix(&child_prefix) {
                real.name = format!("{renamed}{delimiter}{rest}");
            }
        };
        for mailbox in &mut self.mailboxes {
            if let Mailbox::Real(real) = mailbox {
                rename(real);
            }
        }
        for message in &mut self.messages {
            if let Mailbox::Real(real) = &mut message.mailbox {
                rename(real);
            }
            if let Some(Mailbox::Real(real)) = &mut message.junk_origin {
                rename(real);
            }
        }
        if let Mailbox::Real(real) = &mut self.mailbox {
            rename(real);
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// Delete Mailbox…: the mailbox, its submailboxes and their messages
    /// leave the window; Inbox opens if it was showing.
    pub fn remove_real_mailbox(&mut self, account: Uuid, mailbox_id: i64, delimiter: char) {
        let Some(old) = self.mailboxes.iter().find_map(|mailbox| match mailbox {
            Mailbox::Real(real) if real.account == account && real.mailbox_id == mailbox_id => {
                Some(real.name.clone())
            }
            _ => None,
        }) else {
            return;
        };
        let child_prefix = format!("{old}{delimiter}");
        let doomed = |mailbox: &Mailbox| {
            matches!(mailbox, Mailbox::Real(real)
                if real.account == account
                    && (real.name == old || real.name.starts_with(&child_prefix)))
        };
        let showing = doomed(&self.mailbox);
        self.mailboxes.retain(|mailbox| !doomed(mailbox));
        self.messages.retain(|message| !doomed(&message.mailbox));
        if showing {
            self.select_mailbox(Mailbox::AllInboxes);
        } else if self
            .selected
            .as_ref()
            .is_some_and(|id| !self.messages.iter().any(|message| &message.id == id))
        {
            self.selected = None;
        }
        self.revision = self.revision.wrapping_add(1);
    }

    /// The real mailboxes of `account` New Mailbox… can put a mailbox
    /// inside (Inbox included, as on the Mac).
    pub fn mailbox_parents(&self, account: Uuid) -> Vec<RealMailbox> {
        self.mailboxes
            .iter()
            .filter_map(|mailbox| match mailbox {
                Mailbox::Real(real) if real.account == account => Some(real.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Mailbox ▸ Export Mailbox…: messages as one mbox file (mboxrd), the
/// format Mac Mail writes and other mail apps import. Each message gets a
/// `From ` separator line, its line endings become LF, and body lines that
/// begin with `From ` (after any `>`s) gain one more `>`.
pub fn mbox(sources: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for source in sources {
        let text = String::from_utf8_lossy(source);
        let sender = text
            .lines()
            .take_while(|line| !line.is_empty())
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("From").then_some(value)
            })
            .and_then(|value| {
                let value = value.trim();
                let address = value
                    .rsplit_once('<')
                    .map_or(value, |(_, rest)| rest.trim_end_matches('>'));
                (!address.is_empty() && !address.contains(char::is_whitespace))
                    .then(|| address.to_owned())
            })
            .unwrap_or_else(|| "MAILER-DAEMON".to_owned());
        out.extend_from_slice(format!("From {sender} Thu Jan  1 00:00:00 1970\n").as_bytes());
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.trim_start_matches('>').starts_with("From ") {
                out.push(b'>');
            }
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MailState;

    fn state() -> MailState {
        MailState::fixture()
    }

    #[test]
    fn conditions_match_any_or_all_and_skip_bin_and_junk() {
        let state = state();
        let anna = state
            .messages
            .iter()
            .find(|message| message.id == "anna")
            .unwrap()
            .clone();
        let from_anna = SmartCondition::Contains {
            field: SmartField::From,
            text: "ANNA".into(),
        };
        assert!(from_anna.matches(&anna, 0));
        assert!(!SmartCondition::Contains {
            field: SmartField::Subject,
            text: "   ".into()
        }
        .matches(&anna, 0));
        assert!(SmartCondition::HasAttachment.matches(&anna, 0));
        assert!(
            SmartCondition::ReceivedWithinDays { days: 1 }.matches(&anna, anna.received_at + 3600)
        );
        assert!(!SmartCondition::ReceivedWithinDays { days: 1 }
            .matches(&anna, anna.received_at + 3 * 86_400));
        let mut smart = SmartMailbox {
            id: 1,
            name: "Anna".into(),
            match_all: true,
            conditions: vec![from_anna.clone(), SmartCondition::Flagged],
        };
        assert_eq!(smart.matches(&anna, 0), anna.flagged);
        smart.match_all = false;
        assert!(smart.matches(&anna, 0));
        let mut binned = anna.clone();
        binned.mailbox = state
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.special_use() == Some(SpecialUse::Trash))
            .unwrap()
            .clone();
        assert!(!smart.matches(&binned, 0));
        smart.conditions.clear();
        assert!(!smart.matches(&anna, 0));
    }

    #[test]
    fn smart_mailboxes_sit_after_favourites_filter_the_list_and_survive_refresh() {
        let mut state = state();
        let id = state.next_smart_id();
        assert_eq!(state.next_smart_name(), "Smart Mailbox 1");
        let smart = SmartMailbox {
            id,
            name: "Unread".into(),
            match_all: true,
            conditions: vec![SmartCondition::Unread],
        };
        state.save_smart_mailbox(smart.clone());
        assert_eq!(state.mailboxes[4], Mailbox::Smart(smart.clone()));
        assert_eq!(state.mailboxes[4].account(), "Smart Mailboxes");
        state.select_mailbox(Mailbox::Smart(smart.clone()));
        let unread_before = state
            .visible()
            .iter()
            .filter(|&&index| state.messages[index].unread)
            .count();
        assert!(unread_before > 0);
        // Editing the open smart mailbox updates what it shows.
        state.save_smart_mailbox(SmartMailbox {
            name: "Renamed".into(),
            ..smart.clone()
        });
        assert_eq!(state.mailbox.label(), "Renamed");
        let copy = state.duplicate_smart_mailbox(id).unwrap();
        assert_eq!(copy.name, "Renamed Copy");
        assert_eq!(state.smart_mailboxes().len(), 2);
        // A sync snapshot keeps them.
        let mailboxes: Vec<Mailbox> = state
            .mailboxes
            .iter()
            .filter(|mailbox| !matches!(mailbox, Mailbox::Smart(_)))
            .cloned()
            .collect();
        let messages = state.messages.clone();
        state.refresh_live(mailboxes, messages);
        assert_eq!(state.smart_mailboxes().len(), 2);
        state.remove_smart_mailbox(id);
        assert_eq!(state.mailbox, Mailbox::AllInboxes);
        assert_eq!(state.smart_mailboxes(), vec![copy]);
    }

    #[test]
    fn smart_mailboxes_round_trip_through_their_file() {
        let path = std::env::temp_dir().join(format!("smart-{}.json", Uuid::new_v4()));
        assert!(load_smart_mailboxes(&path).is_empty());
        let list = vec![SmartMailbox {
            id: 3,
            name: "Receipts".into(),
            match_all: false,
            conditions: vec![
                SmartCondition::Contains {
                    field: SmartField::EntireMessage,
                    text: "receipt".into(),
                },
                SmartCondition::ReceivedWithinDays { days: 30 },
            ],
        }];
        save_smart_mailboxes(&path, &list).unwrap();
        assert_eq!(load_smart_mailboxes(&path), list);
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_smart_mailboxes(&path).is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn real_mailboxes_are_added_renamed_with_children_and_removed_with_their_mail() {
        let mut state = state();
        let receipts = state
            .mailboxes
            .iter()
            .find_map(|mailbox| match mailbox {
                Mailbox::Real(real) if real.name == "Receipts" => Some(real.clone()),
                _ => None,
            })
            .unwrap();
        state.add_real_mailbox(RealMailbox {
            mailbox_id: 70,
            name: "Receipts/2026".into(),
            ..receipts.clone()
        });
        assert_eq!(state.mailbox.label(), "Receipts/2026");
        state.select_mailbox(Mailbox::Real(receipts.clone()));
        assert_eq!(
            state.editable_mailbox(),
            Some(&Mailbox::Real(receipts.clone()))
        );
        state.rename_real_mailbox(receipts.account, receipts.mailbox_id, "Bills", '/');
        let names: Vec<String> = state.mailboxes.iter().map(Mailbox::label).collect();
        assert!(names.contains(&"Bills".to_owned()));
        assert!(names.contains(&"Bills/2026".to_owned()));
        assert!(!names.contains(&"Receipts".to_owned()));
        assert_eq!(state.mailbox.label(), "Bills");
        let before = state.messages.len();
        let in_bills = state
            .messages
            .iter()
            .filter(|message| message.mailbox.label().starts_with("Bills"))
            .count();
        state.remove_real_mailbox(receipts.account, receipts.mailbox_id, '/');
        assert_eq!(state.messages.len(), before - in_bills);
        assert!(!state
            .mailboxes
            .iter()
            .any(|mailbox| mailbox.label().starts_with("Bills")));
        assert_eq!(state.mailbox, Mailbox::AllInboxes);
        // Inbox can be neither renamed nor deleted.
        state.select_mailbox(
            state
                .mailboxes
                .iter()
                .find(|mailbox| mailbox.special_use() == Some(SpecialUse::Inbox))
                .unwrap()
                .clone(),
        );
        assert_eq!(state.editable_mailbox(), None);
        assert_eq!(renamed_path("A/B", "C", '/'), "A/C");
        assert_eq!(renamed_path("A", "C", '/'), "C");
        assert_eq!(leaf_name("A/B", '/'), "B");
    }

    #[test]
    fn mbox_separates_messages_and_quotes_from_lines() {
        let out = mbox(&[
            b"From: Ada <ada@example.test>\r\nSubject: Hi\r\n\r\nFrom the start\r\n>From here\r\n"
                .to_vec(),
            b"Subject: No sender\n\nBody\n".to_vec(),
        ]);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("From ada@example.test "));
        assert!(text.contains("\n>From the start\n"));
        assert!(text.contains("\n>>From here\n"));
        assert!(text.contains("\nFrom MAILER-DAEMON "));
        assert!(!text.contains('\r'));
    }
}
