//! Mail's interaction model. `MailState` holds whatever the window is
//! showing right now: either the live per-account mailboxes built by
//! `crate::live` from `rmac-mail-storage` (MAIL-4), or — only when
//! `RMAC_MAIL_FIXTURE=1` is set, for development and the Mac-parity
//! screenshots — the fixed sample data in `fixture()`. A real user with no
//! accounts yet never sees fixture data; `main.rs` shows the empty state
//! instead (see `docs/design/calendar-mail.md` §3).

pub mod ics;
pub mod mailto;
pub mod settings;

use rmac_mail_mime::{sanitize_html, RichText};
use rmac_mail_storage::SearchQuery;
use uuid::Uuid;

pub mod compose;

/// `RMAC_MAIL_FIXTURE=1` is the only path to `MailState::fixture()`
/// (`main.rs`); these two fixed ids let it hand out matching
/// `ComposeAccount`s so Compose's "From" picker and Settings ▸ Signatures
/// behave the same way against fixture data as against a live account.
pub const FIXTURE_GOOGLE_ACCOUNT: Uuid = Uuid::from_u128(0x6000_0000_0000_0000_0000_0000_0000_0001);
pub const FIXTURE_ICLOUD_ACCOUNT: Uuid = Uuid::from_u128(0x6000_0000_0000_0000_0000_0000_0000_0002);

/// The handful of roles IMAP's SPECIAL-USE extension (or a well-known
/// mailbox name) assigns a real mailbox. `None` is an ordinary user mailbox
/// or label (for example Gmail's "Receipts").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpecialUse {
    Inbox,
    Drafts,
    Sent,
    Junk,
    Trash,
    Archive,
}

/// One account's real, server-backed mailbox. `account`/`mailbox_id` name a
/// row in that account's own `rmac-mail-storage` cache; `account_path` is
/// GOA's object path, kept here so an organise action can wake exactly that
/// account's `rmac_mail_runtime::Runtime` worker to replay it promptly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealMailbox {
    pub account: Uuid,
    pub account_path: String,
    pub account_label: String,
    pub mailbox_id: i64,
    pub name: String,
    pub special_use: Option<SpecialUse>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mailbox {
    AllInboxes,
    Flagged,
    Drafts,
    Sent,
    Real(RealMailbox),
}

impl Mailbox {
    pub fn label(&self) -> String {
        match self {
            Self::AllInboxes => "All Inboxes".to_owned(),
            Self::Flagged => "Flagged".to_owned(),
            Self::Drafts => "Drafts".to_owned(),
            Self::Sent => "Sent".to_owned(),
            Self::Real(real) => match real.special_use {
                Some(SpecialUse::Inbox) => "Inbox".to_owned(),
                Some(SpecialUse::Drafts) => "Drafts".to_owned(),
                Some(SpecialUse::Sent) => "Sent".to_owned(),
                Some(SpecialUse::Junk) => "Junk".to_owned(),
                Some(SpecialUse::Trash) => "Bin".to_owned(),
                Some(SpecialUse::Archive) => "Archive".to_owned(),
                None => real.name.clone(),
            },
        }
    }

    /// The sidebar group header: "Favourites", or the real account's address.
    pub fn account(&self) -> &str {
        match self {
            Self::AllInboxes | Self::Flagged | Self::Drafts | Self::Sent => "Favourites",
            Self::Real(real) => &real.account_label,
        }
    }

    pub const fn is_real(&self) -> bool {
        matches!(self, Self::Real(_))
    }

    pub const fn special_use(&self) -> Option<SpecialUse> {
        match self {
            Self::Real(real) => real.special_use,
            _ => None,
        }
    }
}

/// A cheap, `Copy` key that identifies one concrete mailbox without cloning
/// any of its strings — used to dedupe conversations per mailbox while
/// filtering thousands of rows (MAIL-10).
fn mailbox_key(mailbox: &Mailbox) -> (Option<Uuid>, i64) {
    match mailbox {
        Mailbox::AllInboxes => (None, -1),
        Mailbox::Flagged => (None, -2),
        Mailbox::Drafts => (None, -3),
        Mailbox::Sent => (None, -4),
        Mailbox::Real(real) => (Some(real.account), real.mailbox_id),
    }
}

/// An attachment on a `Message`. `bytes` is real content so
/// `ics::stage_for_handoff` has something true to write when the viewer
/// hands a calendar invite to Calendar (MAIL-8).
#[derive(Clone, Debug)]
pub struct MessageAttachment {
    pub filename: String,
    pub size_label: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub mailbox: Mailbox,
    /// This account's local `rmac-mail-storage` row id, and its full IMAP
    /// flags bitmask, for a message backed by live storage. `None`/`0` for
    /// fixture data, which nothing ever persists.
    pub row_id: Option<i64>,
    pub raw_flags: i64,
    pub sender: String,
    pub sender_address: String,
    pub initials: String,
    pub date: String,
    pub to: String,
    pub cc: String,
    pub to_addresses: String,
    pub cc_addresses: String,
    pub subject: String,
    pub preview: String,
    pub unread: bool,
    pub flagged: bool,
    pub attachment: Option<MessageAttachment>,
    pub thread_id: String,
    pub body: RichText,
    /// Whether `body`/`attachment` reflect the message's real content yet.
    /// A live message loads as a lightweight row first; `crate::live`
    /// fetches and parses the full body in the background only once it is
    /// actually selected, so a 10 000-message mailbox never parses MIME for
    /// rows nobody has opened.
    pub body_loaded: bool,
    pub junk_origin: Option<Mailbox>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    AllMailboxes,
    CurrentMailbox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrganizeAction {
    Archive,
    Delete,
    Junk,
    Flag,
    MarkRead,
    MarkUnread,
    Move(Mailbox),
    Copy(Mailbox),
}

/// What a mutation that just happened to a live message needs to replay
/// into its account's `rmac-mail-storage` journal. `None` when nothing
/// changed, when the message is fixture data, or (`OrganizeAction::Copy`)
/// when the action has no journal entry yet — see `docs/parity.md` MAIL-4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Persist {
    pub account: Uuid,
    pub account_path: String,
    pub row_id: i64,
    pub change: PersistChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistChange {
    Flags(i64),
    Move(i64),
}

#[derive(Clone)]
struct Undo {
    original: Message,
    added_id: Option<String>,
    selection: Option<String>,
    /// The inverse of whatever `Persist` the action produced, replayed if
    /// the person undoes it.
    undo_persist: Option<Persist>,
}

/// A small, valid iCalendar invite (RFC 5545) for the fixture's "Calendar
/// sync spec" thread, so Mail's attachment chip has a real `.ics` to hand to
/// Calendar instead of only a display label.
const SAMPLE_ICS: &[u8] = b"BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Lulo OS//Mail//EN\r\n\
BEGIN:VEVENT\r\n\
UID:standup-2026-10-05@lulo.local\r\n\
DTSTAMP:20261004T090000Z\r\n\
DTSTART:20261005T090000Z\r\n\
DTEND:20261005T091500Z\r\n\
SUMMARY:Calendar sync standup\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

pub struct MailState {
    pub messages: Vec<Message>,
    /// Every mailbox the sidebar shows: the four Favourites, then each live
    /// account's real mailboxes. Built by `crate::live` for a real user;
    /// `fixture()` builds an equivalent fixed set for development.
    pub mailboxes: Vec<Mailbox>,
    pub mailbox: Mailbox,
    pub selected: Option<String>,
    pub threads: bool,
    pub unread_only: bool,
    pub search: SearchQuery,
    pub search_scope: SearchScope,
    undo: Option<Undo>,
    next_copy: u64,
    last_persist: Option<Persist>,
}

impl MailState {
    /// Builds an empty-but-valid state (the four Favourites, no messages).
    /// `main.rs` only reaches for this through `crate::live::load`, which
    /// returns the same shape once real accounts have synced.
    pub fn new(mailboxes: Vec<Mailbox>, messages: Vec<Message>) -> Self {
        let mut state = Self {
            messages,
            mailboxes,
            mailbox: Mailbox::AllInboxes,
            selected: None,
            threads: true,
            unread_only: false,
            search: SearchQuery::default(),
            search_scope: SearchScope::AllMailboxes,
            undo: None,
            next_copy: 0,
            last_persist: None,
        };
        // Highlights the first row without marking it read: opening Mail
        // must never silently consume someone's unread count.
        state.selected = state
            .visible()
            .first()
            .map(|&index| state.messages[index].id.clone());
        state
    }

    pub fn fixture() -> Self {
        let google = FIXTURE_GOOGLE_ACCOUNT;
        let icloud = FIXTURE_ICLOUD_ACCOUNT;
        let real =
            |account: Uuid, label: &str, id: i64, name: &str, special: Option<SpecialUse>| {
                Mailbox::Real(RealMailbox {
                    account,
                    account_path: format!("/fixture/{label}"),
                    account_label: label.to_owned(),
                    mailbox_id: id,
                    name: name.to_owned(),
                    special_use: special,
                })
            };
        let google_inbox = real(google, "Google", 1, "INBOX", Some(SpecialUse::Inbox));
        let google_drafts = real(google, "Google", 2, "Drafts", Some(SpecialUse::Drafts));
        let google_sent = real(google, "Google", 3, "Sent", Some(SpecialUse::Sent));
        let google_junk = real(google, "Google", 4, "Junk", Some(SpecialUse::Junk));
        let google_trash = real(google, "Google", 5, "Trash", Some(SpecialUse::Trash));
        let google_archive = real(google, "Google", 6, "Archive", Some(SpecialUse::Archive));
        let google_receipts = real(google, "Google", 7, "Receipts", None);
        let icloud_inbox = real(icloud, "iCloud", 1, "INBOX", Some(SpecialUse::Inbox));
        let icloud_sent = real(icloud, "iCloud", 2, "Sent", Some(SpecialUse::Sent));
        let icloud_junk = real(icloud, "iCloud", 3, "Junk", Some(SpecialUse::Junk));
        let icloud_trash = real(icloud, "iCloud", 4, "Trash", Some(SpecialUse::Trash));
        let mailboxes = vec![
            Mailbox::AllInboxes,
            Mailbox::Flagged,
            Mailbox::Drafts,
            Mailbox::Sent,
            google_inbox.clone(),
            google_drafts,
            google_sent,
            google_junk,
            google_trash,
            google_archive,
            google_receipts,
            icloud_inbox.clone(),
            icloud_sent,
            icloud_junk,
            icloud_trash,
        ];
        let data = [
            (
                "anna",
                "Anna Kim",
                "AK",
                "Today at 09:41",
                "Lunch on Friday?",
                "Café Lulo at 12:30 works for me. I booked the table by the window.",
                true,
                false,
                "lunch",
            ),
            (
                "beta",
                "Lulo OS Beta",
                "LB",
                "Today at 08:12",
                "Your build 0.9.0-beta.2 is ready",
                "Release notes, known limitations and how to report a bug.",
                true,
                false,
                "beta",
            ),
            (
                "sam",
                "Sam Ortiz",
                "SO",
                "Yesterday",
                "Re: Calendar sync spec",
                "I left comments on the reminder agent section.",
                false,
                true,
                "calendar",
            ),
            (
                "grandma",
                "Grandma",
                "G",
                "Thursday",
                "Photos from Sunday",
                "Here are the pictures from lunch. The cake one is my favourite!",
                true,
                false,
                "photos",
            ),
            (
                "bank",
                "Bank of Example",
                "BE",
                "Wednesday",
                "Your statement is available",
                "Your October statement is now available to view online.",
                false,
                false,
                "bank",
            ),
            (
                "climbing",
                "Climbing Wall",
                "CW",
                "Tuesday",
                "Membership renewal",
                "Your membership renews on 1 November.",
                false,
                false,
                "climbing",
            ),
            (
                "sam-old",
                "Sam Ortiz",
                "SO",
                "Monday",
                "Calendar sync spec",
                "The draft covers event sync and reminders.",
                false,
                false,
                "calendar",
            ),
            (
                "sam-reply",
                "Jacob Samas",
                "JS",
                "Tuesday",
                "Re: Calendar sync spec",
                "Thanks, I will review the reminder section.",
                false,
                false,
                "calendar",
            ),
        ];
        let icloud_data = [
            (
                "northwind",
                "Northwind Air",
                "NA",
                "Yesterday",
                "Your trip to Lisbon",
                "Booking reference QX7L2P. Flight NW 482 departs 19 October.",
                false,
                true,
                "trip",
            ),
            (
                "ana",
                "Ana Ruiz",
                "AR",
                "Monday",
                "Weekend plans",
                "Are we still on for the farmers market on Saturday?",
                false,
                false,
                "weekend",
            ),
        ];
        let mut messages: Vec<Message> = data
            .into_iter()
            .map(
                |(id, sender, initials, date, subject, preview, unread, flagged, thread_id)| {
                    fixture_message(
                        id,
                        google_inbox.clone(),
                        sender,
                        initials,
                        date,
                        subject,
                        preview,
                        unread,
                        flagged,
                        thread_id,
                    )
                },
            )
            .collect();
        messages.extend(icloud_data.into_iter().map(
            |(id, sender, initials, date, subject, preview, unread, flagged, thread_id)| {
                fixture_message(
                    id,
                    icloud_inbox.clone(),
                    sender,
                    initials,
                    date,
                    subject,
                    preview,
                    unread,
                    flagged,
                    thread_id,
                )
            },
        ));
        // Built directly rather than through `new()`/`select_mailbox()`,
        // which would mark the first message read as a side effect before
        // any test (or a person) ever opened it.
        Self {
            messages,
            mailboxes,
            mailbox: Mailbox::AllInboxes,
            selected: Some("anna".to_owned()),
            threads: true,
            unread_only: false,
            search: SearchQuery::default(),
            search_scope: SearchScope::AllMailboxes,
            undo: None,
            next_copy: 0,
            last_persist: None,
        }
    }

    /// Swaps in a freshly loaded mailbox/message list after a sync snapshot
    /// or new-mail event, keeping the person's current selection, mailbox
    /// filter and search exactly as they were — a background refresh must
    /// never yank focus away from what someone is reading or searching.
    pub fn refresh_live(&mut self, mailboxes: Vec<Mailbox>, mut messages: Vec<Message>) {
        if let Some(selected_id) = self.selected.clone() {
            let already_loaded = self
                .messages
                .iter()
                .find(|message| message.id == selected_id)
                .filter(|message| message.body_loaded)
                .cloned();
            if let Some(loaded) = already_loaded {
                if let Some(refreshed) = messages
                    .iter_mut()
                    .find(|message| message.id == selected_id)
                {
                    refreshed.body = loaded.body;
                    refreshed.attachment = loaded.attachment;
                    refreshed.body_loaded = true;
                }
            }
        }
        self.mailboxes = mailboxes;
        self.messages = messages;
        if let Some(selected_id) = &self.selected {
            if !self
                .messages
                .iter()
                .any(|message| &message.id == selected_id)
            {
                self.selected = None;
            }
        }
    }

    /// Replaces a lightweight live row's placeholder body/attachment with
    /// its real, parsed content once `crate::live::load_body` has fetched
    /// it in the background. A no-op if `id` has since scrolled out or
    /// changed mailbox.
    pub fn set_loaded_body(
        &mut self,
        id: &str,
        body: RichText,
        attachment: Option<MessageAttachment>,
    ) {
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            message.body = body;
            message.attachment = attachment;
            message.body_loaded = true;
        }
    }

    fn in_mailbox(&self, message: &Message, mailbox: &Mailbox) -> bool {
        match mailbox {
            Mailbox::AllInboxes => message.mailbox.special_use() == Some(SpecialUse::Inbox),
            Mailbox::Flagged => message.flagged,
            Mailbox::Drafts => message.mailbox.special_use() == Some(SpecialUse::Drafts),
            Mailbox::Sent => message.mailbox.special_use() == Some(SpecialUse::Sent),
            real => &message.mailbox == real,
        }
    }

    /// The real mailbox playing one special-use role for `account`, if its
    /// cache has one — for example whether it has an Archive mailbox at
    /// all, which decides whether the toolbar's Archive button is enabled.
    pub fn find_special(&self, account: Uuid, special: SpecialUse) -> Option<Mailbox> {
        self.mailboxes
            .iter()
            .find(|mailbox| {
                matches!(mailbox, Mailbox::Real(real) if real.account == account && real.special_use == Some(special))
            })
            .cloned()
    }

    pub fn visible(&self) -> Vec<usize> {
        let mut seen = std::collections::HashSet::new();
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                let in_mailbox =
                    if !self.search.is_empty() && self.search_scope == SearchScope::AllMailboxes {
                        true
                    } else {
                        self.in_mailbox(message, &self.mailbox)
                    };
                // `matches_fields` on an empty query is vacuously true, so
                // skip building its (body-formatting, allocating) argument
                // for the common case of no active search — otherwise
                // every `visible()` call would format every message's full
                // body, however large the mailbox (MAIL-10).
                let search_matches = self.search.is_empty()
                    || self.search.matches_fields(
                        &message.sender,
                        &message.to,
                        &message.subject,
                        &format!("{} {}", message.preview, message.body.plain_text()),
                    );
                if !in_mailbox
                    || self.unread_only && !message.unread
                    || !search_matches
                    || self.threads
                        && !seen.insert((mailbox_key(&message.mailbox), message.thread_id.as_str()))
                {
                    None
                } else {
                    Some(index)
                }
            })
            .collect()
    }

    /// Selects `id`, marking it read. `take_persist` returns whatever
    /// needs replaying into storage as a result.
    pub fn select(&mut self, id: &str) {
        self.selected = Some(id.to_owned());
        self.last_persist = None;
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            if message.unread {
                message.unread = false;
                message.raw_flags |= rmac_mail_storage::FLAG_SEEN;
                if let (Mailbox::Real(real), Some(row_id)) = (&message.mailbox, message.row_id) {
                    self.last_persist = Some(Persist {
                        account: real.account,
                        account_path: real.account_path.clone(),
                        row_id,
                        change: PersistChange::Flags(message.raw_flags),
                    });
                }
            }
        }
    }

    /// Takes whatever `Persist` the last `select`/`apply`/`undo` call
    /// produced. The caller (`MailView`) replays it on a background thread
    /// and wakes that account's sync worker.
    pub fn take_persist(&mut self) -> Option<Persist> {
        self.last_persist.take()
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .find(|message| self.selected.as_deref() == Some(message.id.as_str()))
    }

    pub fn select_next(&mut self, direction: i32) {
        let visible = self.visible();
        if visible.is_empty() {
            self.selected = None;
            return;
        }
        let current = visible
            .iter()
            .position(|&index| self.selected.as_deref() == Some(self.messages[index].id.as_str()))
            .unwrap_or(0);
        let next = (current as i32 + direction).clamp(0, visible.len() as i32 - 1) as usize;
        let id = self.messages[visible[next]].id.clone();
        self.select(&id);
    }

    pub fn select_mailbox(&mut self, mailbox: Mailbox) {
        self.mailbox = mailbox;
        if let Some(id) = self
            .visible()
            .first()
            .map(|&index| self.messages[index].id.clone())
        {
            self.select(&id);
        } else {
            self.selected = None;
            self.last_persist = None;
        }
    }

    pub fn set_search(&mut self, text: &str) {
        self.search = SearchQuery::parse(text);
        if self.search.is_empty() {
            return;
        }
        if !self
            .visible()
            .iter()
            .any(|&index| self.selected.as_deref() == Some(self.messages[index].id.as_str()))
        {
            self.selected = self
                .visible()
                .first()
                .map(|&index| self.messages[index].id.clone());
        }
    }

    pub fn apply(&mut self, action: OrganizeAction) -> bool {
        let Some(index) = self
            .messages
            .iter()
            .position(|message| self.selected.as_deref() == Some(message.id.as_str()))
        else {
            return false;
        };
        let original = self.messages[index].clone();
        let Mailbox::Real(original_real) = &original.mailbox else {
            // A message is always filed under a real mailbox; Favourites
            // are views, never a message's own location.
            return false;
        };
        let original_real = original_real.clone();
        let destination = match &action {
            OrganizeAction::Archive => {
                self.find_special(original_real.account, SpecialUse::Archive)
            }
            OrganizeAction::Delete => self.find_special(original_real.account, SpecialUse::Trash),
            OrganizeAction::Junk => {
                if original_real.special_use == Some(SpecialUse::Junk) {
                    original
                        .junk_origin
                        .clone()
                        .or_else(|| self.find_special(original_real.account, SpecialUse::Inbox))
                } else {
                    self.find_special(original_real.account, SpecialUse::Junk)
                }
            }
            OrganizeAction::Move(target) | OrganizeAction::Copy(target) => {
                (target.account() == original.mailbox.account() && target.is_real())
                    .then(|| target.clone())
            }
            _ => None,
        };
        if matches!(
            action,
            OrganizeAction::Archive
                | OrganizeAction::Delete
                | OrganizeAction::Junk
                | OrganizeAction::Move(_)
                | OrganizeAction::Copy(_)
        ) && destination.is_none()
        {
            return false;
        }
        if destination == Some(original.mailbox.clone()) {
            return false;
        }
        let selection = self.selected.clone();
        let mut added_id = None;
        let mut persist = None;
        match &action {
            OrganizeAction::Flag => {
                self.messages[index].flagged = !original.flagged;
                self.messages[index].raw_flags = toggle_flag(
                    original.raw_flags,
                    rmac_mail_storage::FLAG_FLAGGED,
                    self.messages[index].flagged,
                );
                persist = live_persist(
                    &original_real,
                    original.row_id,
                    PersistChange::Flags(self.messages[index].raw_flags),
                );
            }
            OrganizeAction::MarkRead => {
                self.messages[index].unread = false;
                self.messages[index].raw_flags = original.raw_flags | rmac_mail_storage::FLAG_SEEN;
                persist = live_persist(
                    &original_real,
                    original.row_id,
                    PersistChange::Flags(self.messages[index].raw_flags),
                );
            }
            OrganizeAction::MarkUnread => {
                self.messages[index].unread = true;
                self.messages[index].raw_flags = original.raw_flags & !rmac_mail_storage::FLAG_SEEN;
                persist = live_persist(
                    &original_real,
                    original.row_id,
                    PersistChange::Flags(self.messages[index].raw_flags),
                );
            }
            OrganizeAction::Copy(_) => {
                self.next_copy += 1;
                let mut copy = original.clone();
                copy.id = format!("{}-copy-{}", original.id, self.next_copy);
                copy.mailbox = destination.clone().expect("copy destination checked");
                // `rmac_mail_storage::Change` has no copy kind yet (MAIL-4
                // follow-up, `docs/parity.md`); the copy only exists locally
                // until that lands.
                copy.row_id = None;
                added_id = Some(copy.id.clone());
                self.messages.push(copy);
            }
            OrganizeAction::Junk => {
                let target = destination.clone().expect("junk destination checked");
                self.messages[index].mailbox = target.clone();
                self.messages[index].junk_origin =
                    if original_real.special_use == Some(SpecialUse::Junk) {
                        None
                    } else {
                        Some(original.mailbox.clone())
                    };
                if let Mailbox::Real(target_real) = &target {
                    persist = live_persist(
                        &original_real,
                        original.row_id,
                        PersistChange::Move(target_real.mailbox_id),
                    );
                }
            }
            OrganizeAction::Archive | OrganizeAction::Delete | OrganizeAction::Move(_) => {
                let target = destination.clone().expect("move destination checked");
                self.messages[index].mailbox = target.clone();
                if let Mailbox::Real(target_real) = &target {
                    persist = live_persist(
                        &original_real,
                        original.row_id,
                        PersistChange::Move(target_real.mailbox_id),
                    );
                }
            }
        }
        if self.messages[index].mailbox != original.mailbox && !self.visible().contains(&index) {
            self.selected = self
                .visible()
                .first()
                .map(|&visible| self.messages[visible].id.clone());
        }
        let undo_persist = persist.as_ref().map(|forward| Persist {
            change: match forward.change {
                PersistChange::Flags(_) => PersistChange::Flags(original.raw_flags),
                PersistChange::Move(_) => PersistChange::Move(original_real.mailbox_id),
            },
            ..forward.clone()
        });
        self.undo = Some(Undo {
            original,
            added_id,
            selection,
            undo_persist,
        });
        self.last_persist = persist;
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(undo) = self.undo.take() else {
            return false;
        };
        if let Some(id) = undo.added_id {
            self.messages.retain(|message| message.id != id);
        }
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == undo.original.id)
        {
            *message = undo.original;
        }
        self.selected = undo.selection;
        self.last_persist = undo.undo_persist;
        true
    }

    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    pub fn thread_count(&self, thread_id: &str) -> usize {
        self.messages
            .iter()
            .filter(|message| message.thread_id == thread_id)
            .count()
    }

    pub fn unread_count(&self, mailbox: &Mailbox) -> usize {
        self.messages
            .iter()
            .filter(|message| message.unread && self.in_mailbox(message, mailbox))
            .count()
    }
}

fn toggle_flag(bits: i64, flag: i64, set: bool) -> i64 {
    if set {
        bits | flag
    } else {
        bits & !flag
    }
}

fn live_persist(real: &RealMailbox, row_id: Option<i64>, change: PersistChange) -> Option<Persist> {
    let row_id = row_id?;
    Some(Persist {
        account: real.account,
        account_path: real.account_path.clone(),
        row_id,
        change,
    })
}

#[allow(clippy::too_many_arguments)]
fn fixture_message(
    id: &str,
    mailbox: Mailbox,
    sender: &str,
    initials: &str,
    date: &str,
    subject: &str,
    preview: &str,
    unread: bool,
    flagged: bool,
    thread_id: &str,
) -> Message {
    let body = if id == "anna" {
        sanitize_html("<p>Hi Jacob,</p><p>Café Lulo at 12:30 works for me. I booked the table by the window — tell me if Sam is coming too so I can change it.</p><p>I attached the menu in case you want to pre-order.</p><p>Anna</p><blockquote>On 2 Oct 2026, at 18:05, Jacob Samas wrote:<br>Shall we do lunch on Friday?</blockquote><img src='https://example.invalid/tracker'>")
    } else {
        RichText::from_plain(preview)
    };
    let attachment = match id {
        "anna" => Some(MessageAttachment {
            filename: "Menu.pdf".to_owned(),
            size_label: "212 KB".to_owned(),
            bytes: Vec::new(),
        }),
        "grandma" => Some(MessageAttachment {
            filename: "Photos.zip".to_owned(),
            size_label: "2.4 MB".to_owned(),
            bytes: Vec::new(),
        }),
        "sam" => Some(MessageAttachment {
            filename: "standup.ics".to_owned(),
            size_label: "1 KB".to_owned(),
            bytes: SAMPLE_ICS.to_vec(),
        }),
        _ => None,
    };
    let sender_address = match id {
        "anna" => "anna@example.test",
        "sam" | "sam-old" | "sam-reply" => "sam@example.test",
        _ => "sender@example.test",
    }
    .to_owned();
    let (to, cc) = if id == "anna" {
        ("Jacob Samas".to_owned(), "Sam Ortiz".to_owned())
    } else {
        ("Jacob Samas".to_owned(), String::new())
    };
    let (to_addresses, cc_addresses) = if id == "anna" {
        (
            "jacob@example.test".to_owned(),
            "sam@example.test".to_owned(),
        )
    } else {
        ("jacob@example.test".to_owned(), String::new())
    };
    Message {
        id: id.to_owned(),
        mailbox,
        row_id: None,
        raw_flags: 0,
        sender: sender.to_owned(),
        sender_address,
        initials: initials.to_owned(),
        date: date.to_owned(),
        to,
        cc,
        to_addresses,
        cc_addresses,
        subject: subject.to_owned(),
        preview: preview.to_owned(),
        unread,
        flagged,
        attachment,
        thread_id: thread_id.to_owned(),
        body,
        body_loaded: true,
        junk_origin: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(state: &MailState, account: &str, label: &str) -> Mailbox {
        state
            .mailboxes
            .iter()
            .find(|mailbox| mailbox.account() == account && mailbox.label() == label)
            .cloned()
            .unwrap_or_else(|| panic!("no {account} mailbox named {label}"))
    }

    #[test]
    fn open_message_marks_read_and_updates_unread_counts() {
        let mut state = MailState::fixture();
        assert_eq!(state.unread_count(&Mailbox::AllInboxes), 3);
        state.select("anna");
        assert!(!state.selected_message().unwrap().unread);
        assert_eq!(state.unread_count(&Mailbox::AllInboxes), 2);
    }

    #[test]
    fn conversations_collapse_and_expand_without_losing_message() {
        let mut state = MailState::fixture();
        assert_eq!(state.visible().len(), 8);
        assert_eq!(state.thread_count("calendar"), 3);
        state.threads = false;
        assert_eq!(state.visible().len(), 10);
        state.select("sam-old");
        assert_eq!(state.selected_message().unwrap().id, "sam-old");
    }

    #[test]
    fn folder_and_unread_filters_intersect() {
        let mut state = MailState::fixture();
        let icloud_inbox = find(&state, "iCloud", "Inbox");
        let google_inbox = find(&state, "Google", "Inbox");
        state.select_mailbox(icloud_inbox);
        assert_eq!(state.visible().len(), 2);
        state.unread_only = true;
        assert!(state.visible().is_empty());
        state.select_mailbox(google_inbox);
        assert_eq!(state.visible().len(), 2);
        assert!(!state.selected_message().unwrap().unread);
    }

    #[test]
    fn viewer_receives_sanitized_rich_text() {
        let state = MailState::fixture();
        let message = state.selected_message().unwrap();
        assert_eq!(message.body.blocked_remote_images.len(), 1);
        assert!(message.body.plain_text().contains("Café Lulo"));
        assert!(!message.body.plain_text().contains("tracker"));
    }

    #[test]
    fn organise_routes_within_account_and_undo_restores_message() {
        let mut state = MailState::fixture();
        let google_junk = find(&state, "Google", "Junk");
        let google_trash = find(&state, "Google", "Bin");
        let google_inbox = find(&state, "Google", "Inbox");
        let icloud_inbox = find(&state, "iCloud", "Inbox");
        let google_receipts = find(&state, "Google", "Receipts");
        state.apply(OrganizeAction::Flag);
        assert!(state.selected_message().unwrap().flagged);
        assert!(state.undo());
        assert!(!state.selected_message().unwrap().flagged);
        assert!(state.apply(OrganizeAction::Junk));
        assert_eq!(state.messages[0].mailbox, google_junk);
        // Junking a message that was visible moves selection to the next
        // visible one (it leaves the current mailbox view), so re-select it
        // before toggling it back, the same way a real click would.
        state.select("anna");
        assert!(state.apply(OrganizeAction::Junk));
        assert_eq!(state.messages[0].mailbox, google_inbox);
        assert!(state.apply(OrganizeAction::Delete));
        assert_eq!(state.messages[0].mailbox, google_trash);
        assert!(state.undo());
        assert_eq!(state.messages[0].mailbox, google_inbox);
        assert!(!state.apply(OrganizeAction::Move(icloud_inbox)));
        assert!(state.apply(OrganizeAction::Move(google_receipts.clone())));
        assert_eq!(state.messages[0].mailbox, google_receipts);
        assert!(state.undo());
        assert_eq!(state.messages[0].mailbox, google_inbox);
    }

    #[test]
    fn copy_and_search_tokens_respect_mailbox_scope() {
        let mut state = MailState::fixture();
        let google_receipts = find(&state, "Google", "Receipts");
        assert!(state.apply(OrganizeAction::Copy(google_receipts)));
        assert_eq!(state.messages.len(), 11);
        state.set_search("from:Anna subject:\"Lunch on Friday\"");
        assert_eq!(state.visible().len(), 2);
        state.search_scope = SearchScope::CurrentMailbox;
        assert_eq!(state.visible().len(), 1);
        assert!(state.undo());
        assert_eq!(state.messages.len(), 10);
        assert_eq!(state.visible().len(), 1);
    }

    #[test]
    fn marking_read_on_a_live_message_queues_a_flags_persist() {
        let account = Uuid::new_v4();
        let inbox = Mailbox::Real(RealMailbox {
            account,
            account_path: "/org/gnome/OnlineAccounts/Accounts/1".to_owned(),
            account_label: "me@example.test".to_owned(),
            mailbox_id: 1,
            name: "INBOX".to_owned(),
            special_use: Some(SpecialUse::Inbox),
        });
        let mut message = fixture_message(
            "live-1",
            inbox.clone(),
            "Ada",
            "A",
            "Today",
            "Hi",
            "Hi",
            true,
            false,
            "t1",
        );
        message.row_id = Some(42);
        let mut state = MailState::new(vec![Mailbox::AllInboxes, inbox], vec![message]);
        state.select("live-1");
        let persist = state.take_persist().expect("marking read should persist");
        assert_eq!(persist.account, account);
        assert_eq!(persist.row_id, 42);
        assert_eq!(
            persist.change,
            PersistChange::Flags(rmac_mail_storage::FLAG_SEEN)
        );
    }

    /// MAIL-10: a 10 000-message mailbox must stay smooth. `visible()` and
    /// `select()`/`select_next()` run on every scroll-driven render and
    /// every arrow-key press, so each must stay linear in the message
    /// count rather than the O(n²) a per-row `thread_count()` call would
    /// give `crate::view::list` (fixed alongside this test — see its
    /// `thread_counts` map). A generous 200 ms budget absorbs a slow CI
    /// runner while still catching an accidental quadratic regression,
    /// which would take tens of seconds at this size.
    #[test]
    fn ten_thousand_messages_stay_linear() {
        let account = Uuid::new_v4();
        let inbox = Mailbox::Real(RealMailbox {
            account,
            account_path: "/fixture/perf".to_owned(),
            account_label: "perf@example.test".to_owned(),
            mailbox_id: 1,
            name: "INBOX".to_owned(),
            special_use: Some(SpecialUse::Inbox),
        });
        let messages: Vec<Message> = (0..10_000)
            .map(|index| {
                let mut message = fixture_message(
                    &format!("perf-{index}"),
                    inbox.clone(),
                    "Sender",
                    "SE",
                    "Today",
                    "Subject",
                    "Preview",
                    index % 5 == 0,
                    index % 11 == 0,
                    &format!("perf-{index}"),
                );
                message.row_id = Some(index);
                message
            })
            .collect();
        let mut state = MailState::new(vec![Mailbox::AllInboxes, inbox], messages);
        let start = std::time::Instant::now();
        for _ in 0..20 {
            let visible = state.visible();
            assert_eq!(visible.len(), 10_000);
            state.select_next(1);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "20 passes over 10 000 messages took {elapsed:?}, expected well under 200ms"
        );
    }
}
