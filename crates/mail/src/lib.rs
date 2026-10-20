//! Mail's interaction model. `MailState` holds whatever the window is
//! showing right now: either the live per-account mailboxes built by
//! `crate::live` from `rmac-mail-storage` (MAIL-4), or — only when
//! `RMAC_MAIL_FIXTURE=1` is set, for development and the Mac-parity
//! screenshots — the fixed sample data in `fixture()`. A real user with no
//! accounts yet never sees fixture data; `main.rs` shows the empty state
//! instead (see `docs/design/calendar-mail.md` §3).

pub mod ics;
pub mod mailto;
pub mod rules;
pub mod settings;
pub mod smart;

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
    /// Every message from a VIP (Message ▸ Add Sender to VIPs). Listed only
    /// while there is at least one VIP, as on the Mac.
    Vips,
    Flagged,
    Drafts,
    Sent,
    Real(RealMailbox),
    /// A saved search (Mailbox ▸ New Smart Mailbox…, `crate::smart`).
    Smart(smart::SmartMailbox),
}

impl Mailbox {
    pub fn label(&self) -> String {
        match self {
            Self::AllInboxes => "All Inboxes".to_owned(),
            Self::Vips => "VIPs".to_owned(),
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
            Self::Smart(smart) => smart.name.clone(),
        }
    }

    /// The sidebar group header: "Favourites", or the real account's address.
    pub fn account(&self) -> &str {
        match self {
            Self::AllInboxes | Self::Vips | Self::Flagged | Self::Drafts | Self::Sent => {
                "Favourites"
            }
            Self::Real(real) => &real.account_label,
            Self::Smart(_) => "Smart Mailboxes",
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
        Mailbox::Vips => (None, -5),
        Mailbox::Drafts => (None, -3),
        Mailbox::Sent => (None, -4),
        Mailbox::Real(real) => (Some(real.account), real.mailbox_id),
        Mailbox::Smart(smart) => (None, -100 - i64::try_from(smart.id).unwrap_or(0)),
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

/// How far along `Message::body`/`attachment` are towards reflecting the
/// message's real content. A live message loads as a lightweight row
/// first; `crate::live` fetches the full body only once it is actually
/// selected, so a 10 000-message mailbox never parses MIME for rows nobody
/// has opened, and a header-only message (any mail from before an
/// account's first full sync) downloads it on demand instead of showing
/// just the one-line preview forever.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyState {
    /// Nothing requested yet.
    NotLoaded,
    /// Checking whether a copy is already cached locally.
    Loading,
    /// No local copy; fetching it over the network.
    Downloading,
    /// `body`/`attachment` reflect the real content.
    Loaded,
    /// The network fetch failed (offline, server error, etc). The message
    /// to show the person, with a Retry action.
    Failed(String),
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
    /// When the message arrived, as Unix seconds: the key View ▸ Sort By ▸
    /// Date orders by. `0` sorts as the oldest.
    pub received_at: i64,
    /// The full date and time, shown in place of `date` while View ▸ Show
    /// Date and Time is on.
    pub date_time: String,
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
    pub body_state: BodyState,
    pub junk_origin: Option<Mailbox>,
    /// The message's size in bytes, for View ▸ Sort By ▸ Size and Show
    /// Message Size. 0 while the server has not said.
    pub size: i64,
}

impl Message {
    /// Message ▸ Flag's colour, from the flag-colour bits Mail shares with
    /// Mac Mail (`$MailFlagBit0..2`). Red when no bit is set.
    pub fn flag_colour(&self) -> FlagColour {
        FlagColour::from_flags(self.raw_flags)
    }
}

/// Message ▸ Flag ▸ colour. The value is Mac Mail's: the three
/// `$MailFlagBit` keywords read as a number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FlagColour {
    #[default]
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Grey,
}

impl FlagColour {
    /// In Message ▸ Flag's order.
    pub const MENU: [FlagColour; 7] = [
        FlagColour::Orange,
        FlagColour::Red,
        FlagColour::Purple,
        FlagColour::Blue,
        FlagColour::Yellow,
        FlagColour::Green,
        FlagColour::Grey,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Grey => "Grey",
        }
    }

    const fn value(self) -> i64 {
        match self {
            Self::Red => 0,
            Self::Orange => 1,
            Self::Yellow => 2,
            Self::Green => 3,
            Self::Blue => 4,
            Self::Purple => 5,
            Self::Grey => 6,
        }
    }

    pub fn from_flags(flags: i64) -> Self {
        let mut value = 0;
        for (index, bit) in [
            rmac_mail_storage::FLAG_COLOUR_BIT0,
            rmac_mail_storage::FLAG_COLOUR_BIT1,
            rmac_mail_storage::FLAG_COLOUR_BIT2,
        ]
        .into_iter()
        .enumerate()
        {
            if flags & bit != 0 {
                value |= 1 << index;
            }
        }
        Self::MENU
            .into_iter()
            .find(|colour| colour.value() == value)
            .unwrap_or_default()
    }

    /// `flags` flagged with this colour.
    pub fn apply(self, flags: i64) -> i64 {
        let mut bits =
            (flags & !rmac_mail_storage::FLAG_COLOUR_MASK) | rmac_mail_storage::FLAG_FLAGGED;
        for (index, bit) in [
            rmac_mail_storage::FLAG_COLOUR_BIT0,
            rmac_mail_storage::FLAG_COLOUR_BIT1,
            rmac_mail_storage::FLAG_COLOUR_BIT2,
        ]
        .into_iter()
        .enumerate()
        {
            if self.value() & (1 << index) != 0 {
                bits |= bit;
            }
        }
        bits
    }
}

/// `212 KB` / `2.4 MB`, as Mac Mail shows sizes.
pub fn size_label(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes.max(0) as f64;
    if value < KB {
        format!("{value:.0} bytes")
    } else if value < KB * KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

/// One Edit ▸ Find match in a message body: which block and span it sits
/// in, and its byte range in that span's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMatch {
    pub block: usize,
    pub span: usize,
    pub range: std::ops::Range<usize>,
}

/// Every case-insensitive occurrence of `query` in `body`, in reading
/// order, for Edit ▸ Find in the message viewer.
pub fn find_in_body(body: &RichText, query: &str) -> Vec<FindMatch> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (block_index, block) in body.blocks.iter().enumerate() {
        for (span_index, span) in block.spans.iter().enumerate() {
            let text = span.text.as_str();
            let mut skip_until = 0;
            for (start, _) in text.char_indices() {
                if start < skip_until {
                    continue;
                }
                let mut wanted = needle.iter();
                let mut end = start;
                let mut matched = true;
                'chars: for (offset, ch) in text[start..].char_indices() {
                    for lower in ch.to_lowercase() {
                        match wanted.next() {
                            Some(&want) if want == lower => {}
                            Some(_) => {
                                matched = false;
                                break 'chars;
                            }
                            None => break 'chars,
                        }
                    }
                    end = start + offset + ch.len_utf8();
                    if wanted.as_slice().is_empty() {
                        break;
                    }
                }
                if matched && wanted.as_slice().is_empty() && end > start {
                    matches.push(FindMatch {
                        block: block_index,
                        span: span_index,
                        range: start..end,
                    });
                    skip_until = end;
                }
            }
        }
    }
    matches
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    AllMailboxes,
    CurrentMailbox,
}

/// View ▸ Sort By. Mail keeps no sent date, reminder or send-later time,
/// so those keys are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    Attachments,
    #[default]
    Date,
    Flags,
    From,
    Size,
    Subject,
    To,
    Unread,
}

impl SortKey {
    pub const ALL: [SortKey; 8] = [
        SortKey::Attachments,
        SortKey::Date,
        SortKey::Flags,
        SortKey::From,
        SortKey::Size,
        SortKey::Subject,
        SortKey::To,
        SortKey::Unread,
    ];

    /// The order Mail starts each key in, as the Mac does: newest, flagged,
    /// unread and with-attachment messages on top, names from A.
    const fn descending_by_default(self) -> bool {
        matches!(
            self,
            SortKey::Date | SortKey::Flags | SortKey::Unread | SortKey::Attachments | SortKey::Size
        )
    }
}

/// View ▸ Filter: the message filter, and which messages it keeps while it
/// is on. Within "Include:" and "Addressed:" a message needs any one ticked
/// row; the groups and "Only Mail with Attachments" must all hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageFilter {
    pub enabled: bool,
    pub unread: bool,
    pub flagged: bool,
    pub to_me: bool,
    pub cc_me: bool,
    pub attachments: bool,
    /// "Only from VIP".
    pub vip: bool,
}

impl Default for MessageFilter {
    fn default() -> Self {
        Self {
            enabled: false,
            unread: true,
            flagged: false,
            to_me: false,
            cc_me: false,
            attachments: false,
            vip: false,
        }
    }
}

/// Mailbox ▸ Erase Deleted Items / Erase Junk Mail: which special mailbox
/// to empty, and in which account (`None` for every account).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Erase {
    pub special: SpecialUse,
    pub account: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrganizeAction {
    Archive,
    Delete,
    Junk,
    Flag,
    /// Message ▸ Flag ▸ colour: flags the message in that colour.
    FlagColour(FlagColour),
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
    /// Erase: remove the message from the server for good.
    Delete,
}

#[derive(Clone)]
struct Undo {
    action: OrganizeAction,
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
    pub search: SearchQuery,
    pub search_scope: SearchScope,
    pub sort: SortKey,
    /// Whether the sort runs from the highest value (newest, Z, flagged).
    pub descending: bool,
    pub filter: MessageFilter,
    /// The addresses of this person's accounts, for Filter ▸ To: Me / Cc: Me.
    pub my_addresses: Vec<String>,
    /// Conversations shown message by message while Organise by
    /// Conversation is on (View ▸ Expand All Conversations).
    pub expanded: std::collections::HashSet<String>,
    undo: Option<Undo>,
    /// The action Undo just reversed, for Edit ▸ Redo.
    redo: Option<(OrganizeAction, String)>,
    /// Where Message ▸ Move to last moved a message, for Move Again.
    pub last_move: Option<Mailbox>,
    next_copy: u64,
    last_persist: Vec<Persist>,
    /// Bumped whenever a message changes here (read marks, organise
    /// actions, undo, erase), so a second viewer window can follow.
    pub revision: u64,
    /// The local contacts store (`rmac-contacts`), for VIPs, rules and
    /// Add Sender to Contacts.
    pub people: rmac_contacts::Contacts,
    /// `people`'s VIP addresses, lower-cased, so the VIPs mailbox costs a
    /// hash lookup per message.
    vip_addresses: std::collections::HashSet<String>,
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
            search: SearchQuery::default(),
            search_scope: SearchScope::AllMailboxes,
            sort: SortKey::Date,
            descending: true,
            filter: MessageFilter::default(),
            my_addresses: Vec::new(),
            expanded: std::collections::HashSet::new(),
            undo: None,
            redo: None,
            last_move: None,
            next_copy: 0,
            last_persist: Vec::new(),
            revision: 0,
            people: rmac_contacts::Contacts::default(),
            vip_addresses: std::collections::HashSet::new(),
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
        // Arrival times matching the date labels above (Monday 28 September
        // to Today, 5 October 2026), so Sort By ▸ Date orders them the way
        // the labels read.
        const TODAY_0941: i64 = 1_791_193_260;
        const HOUR: i64 = 3_600;
        const DAY: i64 = 24 * HOUR;
        for message in &mut messages {
            let ago = match message.id.as_str() {
                "anna" => 0,
                "beta" => HOUR + 29 * 60,
                "sam" => DAY + 2 * HOUR,
                "northwind" => DAY + 5 * HOUR,
                "grandma" => 4 * DAY,
                "bank" => 5 * DAY,
                "climbing" => 6 * DAY,
                "sam-reply" => 6 * DAY + 3 * HOUR,
                "sam-old" => 7 * DAY,
                _ => 7 * DAY + 4 * HOUR,
            };
            message.received_at = TODAY_0941 - ago;
            message.date_time = format_date_time(message.received_at);
        }
        // Built directly rather than through `new()`/`select_mailbox()`,
        // which would mark the first message read as a side effect before
        // any test (or a person) ever opened it.
        Self {
            messages,
            mailboxes,
            mailbox: Mailbox::AllInboxes,
            selected: Some("anna".to_owned()),
            threads: true,
            search: SearchQuery::default(),
            search_scope: SearchScope::AllMailboxes,
            sort: SortKey::Date,
            descending: true,
            filter: MessageFilter::default(),
            my_addresses: Vec::new(),
            expanded: std::collections::HashSet::new(),
            undo: None,
            redo: None,
            last_move: None,
            next_copy: 0,
            last_persist: Vec::new(),
            revision: 0,
            people: rmac_contacts::Contacts::default(),
            vip_addresses: std::collections::HashSet::new(),
        }
    }

    /// The fixture plus `extra` generated inbox messages, for the speed
    /// sweep's long-list scrolling (`RMAC_MAIL_FIXTURE_MESSAGES`).
    pub fn fixture_with_extra(extra: usize) -> Self {
        let mut state = Self::fixture();
        let inbox = state
            .mailboxes
            .iter()
            .find(|mailbox| matches!(mailbox, Mailbox::Real(real) if real.special_use == Some(SpecialUse::Inbox)))
            .cloned()
            .unwrap_or(Mailbox::AllInboxes);
        state.messages.extend((0..extra).map(|index| {
            let id = format!("bulk-{index}");
            let mut message = fixture_message(
                &id,
                inbox.clone(),
                "Speed Sweep",
                "SS",
                "Last week",
                &format!("Generated message {index}"),
                "A generated message for scrolling a long list.",
                index % 3 == 0,
                index % 7 == 0,
                &id,
            );
            // Older than every fixture message, one minute apart.
            message.received_at = 1_790_000_000 - index as i64 * 60;
            message
        }));
        state
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
                .filter(|message| message.body_state == BodyState::Loaded)
                .cloned();
            if let Some(loaded) = already_loaded {
                if let Some(refreshed) = messages
                    .iter_mut()
                    .find(|message| message.id == selected_id)
                {
                    refreshed.body = loaded.body;
                    refreshed.attachment = loaded.attachment;
                    refreshed.body_state = BodyState::Loaded;
                }
            }
        }
        // Smart mailboxes live on this computer, not in the sync snapshot.
        let smart = self.smart_mailboxes();
        self.mailboxes = mailboxes;
        self.messages = messages;
        self.sync_vip_mailbox();
        if !smart.is_empty() {
            self.set_smart_mailboxes(smart);
        }
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
    /// its real, parsed content once `crate::live` has loaded or fetched
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
            message.body_state = BodyState::Loaded;
        }
    }

    /// Marks a message's body loading/downloading/failed, for the
    /// "Downloading…"/"Retry" states while `crate::live::fetch_and_store_body`
    /// runs in the background. A no-op if `id` has since scrolled out.
    pub fn set_body_state(&mut self, id: &str, state: BodyState) {
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            message.body_state = state;
        }
    }

    fn in_mailbox(&self, message: &Message, mailbox: &Mailbox) -> bool {
        match mailbox {
            Mailbox::AllInboxes => message.mailbox.special_use() == Some(SpecialUse::Inbox),
            Mailbox::Flagged => message.flagged,
            Mailbox::Vips => self.is_vip(&message.sender_address),
            Mailbox::Drafts => message.mailbox.special_use() == Some(SpecialUse::Drafts),
            Mailbox::Sent => message.mailbox.special_use() == Some(SpecialUse::Sent),
            Mailbox::Smart(smart) => smart.matches(message, smart::now()),
            real => &message.mailbox == real,
        }
    }

    /// Whether `address` (bare or "Name <address>") is a VIP's.
    pub fn is_vip(&self, address: &str) -> bool {
        !self.vip_addresses.is_empty()
            && self
                .vip_addresses
                .contains(&rmac_contacts::bare_address(address).to_ascii_lowercase())
    }

    /// Replaces the contacts store (loaded off the UI thread, or changed by
    /// Add Sender to Contacts / VIPs) and shows or hides the VIPs mailbox.
    pub fn set_people(&mut self, people: rmac_contacts::Contacts) {
        self.vip_addresses = people.vip_addresses();
        self.people = people;
        self.sync_vip_mailbox();
        self.revision += 1;
    }

    /// The VIPs row sits under All Inboxes while there is any VIP.
    fn sync_vip_mailbox(&mut self) {
        let listed = self
            .mailboxes
            .iter()
            .position(|mailbox| *mailbox == Mailbox::Vips);
        match (self.vip_addresses.is_empty(), listed) {
            (false, None) => {
                let at = self
                    .mailboxes
                    .iter()
                    .position(|mailbox| *mailbox == Mailbox::AllInboxes)
                    .map_or(0, |index| index + 1);
                self.mailboxes.insert(at, Mailbox::Vips);
            }
            (true, Some(index)) => {
                self.mailboxes.remove(index);
                if self.mailbox == Mailbox::Vips {
                    self.mailbox = Mailbox::AllInboxes;
                }
            }
            _ => {}
        }
    }

    /// Mail ▸ Settings ▸ Rules on `ids` (new mail as it arrives, or the
    /// selection for Message ▸ Apply Rules): every matching rule's actions,
    /// through the same organise actions and journal entries as the menu
    /// commands. Not undoable, as on the Mac. Returns how many messages a
    /// rule acted on.
    pub fn apply_rules(&mut self, rules: &[rules::Rule], ids: &[String]) -> usize {
        if rules.is_empty() || ids.is_empty() {
            return 0;
        }
        let selection = self.selected.clone();
        let mut persists = Vec::new();
        let mut acted = 0;
        for id in ids {
            let Some(message) = self.messages.iter().find(|message| &message.id == id) else {
                continue;
            };
            let actions: Vec<rules::Action> = rules::actions_for(rules, message, &self.people)
                .into_iter()
                .cloned()
                .collect();
            let unread = message.unread;
            let mut changed = false;
            for action in actions {
                let organize = match &action {
                    rules::Action::MoveMessage { to } => {
                        self.rule_target(to).map(OrganizeAction::Move)
                    }
                    rules::Action::CopyMessage { to } => {
                        self.rule_target(to).map(OrganizeAction::Copy)
                    }
                    rules::Action::MarkAsFlagged { .. } => {
                        action.flag_colour().map(OrganizeAction::FlagColour)
                    }
                    rules::Action::MarkAsRead => unread.then_some(OrganizeAction::MarkRead),
                    rules::Action::DeleteMessage => Some(OrganizeAction::Delete),
                    rules::Action::StopEvaluatingRules => None,
                };
                let Some(organize) = organize else {
                    continue;
                };
                self.selected = Some(id.clone());
                if self.apply(organize) {
                    changed = true;
                    persists.append(&mut self.last_persist);
                }
            }
            acted += usize::from(changed);
        }
        self.undo = None;
        self.redo = None;
        let visible = self.visible();
        self.selected = selection
            .filter(|id| visible.iter().any(|&index| &self.messages[index].id == id))
            .or_else(|| {
                visible
                    .first()
                    .map(|&index| self.messages[index].id.clone())
            });
        self.last_persist = persists;
        acted
    }

    /// The real mailbox a rule names, by account and mailbox name.
    pub fn rule_target(&self, target: &rules::MailboxRef) -> Option<Mailbox> {
        self.mailboxes
            .iter()
            .find(|mailbox| {
                matches!(mailbox, Mailbox::Real(real)
                    if real.account_label == target.account && real.name == target.name)
            })
            .cloned()
    }

    /// The id of the message `(account, row_id)` names, for a new-mail
    /// event.
    pub fn id_for_row(&self, account: Uuid, row_id: i64) -> Option<String> {
        self.messages
            .iter()
            .find(|message| {
                message.row_id == Some(row_id)
                    && matches!(&message.mailbox, Mailbox::Real(real) if real.account == account)
            })
            .map(|message| message.id.clone())
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
        let mut shown: Vec<usize> = self
            .messages
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
                (in_mailbox && search_matches && self.passes_filter(message)).then_some(index)
            })
            .collect();
        self.sort_indices(&mut shown);
        if self.threads {
            // Each conversation shows as its first message in sort order,
            // unless it is expanded.
            let mut seen = std::collections::HashSet::new();
            shown.retain(|&index| {
                let message = &self.messages[index];
                self.expanded.contains(&message.thread_id)
                    || seen.insert((mailbox_key(&message.mailbox), message.thread_id.as_str()))
            });
        }
        shown
    }

    /// Orders `indices` by View ▸ Sort By. A stable sort, so equal keys
    /// keep the order the mailbox listed them in.
    fn sort_indices(&self, indices: &mut [usize]) {
        let messages = &self.messages;
        let compare = |a: &usize, b: &usize| {
            let (a, b) = (&messages[*a], &messages[*b]);
            let order = match self.sort {
                SortKey::Date => a.received_at.cmp(&b.received_at),
                SortKey::Attachments => a.attachment.is_some().cmp(&b.attachment.is_some()),
                SortKey::Flags => a.flagged.cmp(&b.flagged),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Unread => a.unread.cmp(&b.unread),
                SortKey::From => compare_text(&a.sender, &b.sender),
                SortKey::Subject => compare_text(
                    strip_reply_prefix(&a.subject),
                    strip_reply_prefix(&b.subject),
                ),
                SortKey::To => compare_text(&a.to, &b.to),
            };
            // Within one value the newest message comes first, as on the Mac.
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| b.received_at.cmp(&a.received_at))
        };
        indices.sort_by(compare);
    }

    /// View ▸ Sort By ▸ `key`, starting in that key's usual direction.
    pub fn set_sort(&mut self, key: SortKey) {
        if self.sort != key {
            self.sort = key;
            self.descending = key.descending_by_default();
        }
    }

    /// View ▸ Message Filter: whether `message` passes the filter.
    fn passes_filter(&self, message: &Message) -> bool {
        let filter = self.filter;
        if !filter.enabled {
            return true;
        }
        let include = !(filter.unread || filter.flagged)
            || filter.unread && message.unread
            || filter.flagged && message.flagged;
        let addressed = !(filter.to_me || filter.cc_me)
            || filter.to_me && self.addressed_to_me(&message.to_addresses)
            || filter.cc_me && self.addressed_to_me(&message.cc_addresses);
        include
            && addressed
            && (!filter.attachments || message.attachment.is_some())
            && (!filter.vip || self.is_vip(&message.sender_address))
    }

    pub fn addressed_to_me(&self, addresses: &str) -> bool {
        addresses.split([',', ';']).any(|address| {
            let address = address
                .rsplit_once('<')
                .map_or(address, |(_, rest)| rest.trim_end_matches('>'))
                .trim();
            self.my_addresses
                .iter()
                .any(|mine| mine.eq_ignore_ascii_case(address))
        })
    }

    /// View ▸ Expand All Conversations.
    pub fn expand_all_conversations(&mut self) {
        let mut counts = std::collections::HashMap::<&str, usize>::new();
        for message in &self.messages {
            *counts.entry(message.thread_id.as_str()).or_insert(0) += 1;
        }
        self.expanded = counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(thread, _)| thread.to_owned())
            .collect();
    }

    /// View ▸ Collapse All Conversations.
    pub fn collapse_all_conversations(&mut self) {
        self.expanded.clear();
    }

    /// Selects `id`, marking it read. `take_persist` returns whatever
    /// needs replaying into storage as a result.
    pub fn select(&mut self, id: &str) {
        self.selected = Some(id.to_owned());
        self.last_persist.clear();
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            if message.unread {
                message.unread = false;
                message.raw_flags |= rmac_mail_storage::FLAG_SEEN;
                self.revision += 1;
                if let (Mailbox::Real(real), Some(row_id)) = (&message.mailbox, message.row_id) {
                    self.last_persist.push(Persist {
                        account: real.account,
                        account_path: real.account_path.clone(),
                        row_id,
                        change: PersistChange::Flags(message.raw_flags),
                    });
                }
            }
        }
    }

    /// Takes whatever `Persist`s the last `select`/`apply`/`undo`/erase
    /// call produced. The caller (`MailView`) replays them on a background
    /// thread and wakes each account's sync worker.
    pub fn take_persists(&mut self) -> Vec<Persist> {
        std::mem::take(&mut self.last_persist)
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
            self.last_persist.clear();
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
            OrganizeAction::FlagColour(colour) => {
                let flags = colour.apply(original.raw_flags);
                if original.flagged && flags == original.raw_flags {
                    return false;
                }
                self.messages[index].flagged = true;
                self.messages[index].raw_flags = flags;
                persist =
                    live_persist(&original_real, original.row_id, PersistChange::Flags(flags));
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
                PersistChange::Move(_) | PersistChange::Delete => {
                    PersistChange::Move(original_real.mailbox_id)
                }
            },
            ..forward.clone()
        });
        if let OrganizeAction::Move(target) = &action {
            self.last_move = Some(target.clone());
        }
        self.redo = None;
        self.undo = Some(Undo {
            action,
            original,
            added_id,
            selection,
            undo_persist,
        });
        self.last_persist = persist.into_iter().collect();
        self.revision += 1;
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(undo) = self.undo.take() else {
            return false;
        };
        if let Some(id) = undo.added_id {
            self.messages.retain(|message| message.id != id);
        }
        self.redo = Some((undo.action, undo.original.id.clone()));
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == undo.original.id)
        {
            *message = undo.original;
        }
        self.selected = undo.selection;
        self.last_persist = undo.undo_persist.into_iter().collect();
        self.revision += 1;
        true
    }

    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// Edit ▸ Redo: performs again the action Undo just reversed, on the
    /// same message.
    pub fn redo(&mut self) -> bool {
        let Some((action, id)) = self.redo.take() else {
            return false;
        };
        if !self.messages.iter().any(|message| message.id == id) {
            return false;
        }
        self.selected = Some(id);
        self.apply(action)
    }

    pub fn can_redo(&self) -> bool {
        self.redo.is_some()
    }

    /// Message ▸ Move Again: the last Move to target, when the selected
    /// message can go there (same account, not already in it).
    pub fn move_again_target(&self) -> Option<Mailbox> {
        let target = self.last_move.clone()?;
        let message = self.selected_message()?;
        (target.account() == message.mailbox.account() && target != message.mailbox)
            .then_some(target)
    }

    /// Mailbox ▸ Mark All Messages as Read, for every message the list
    /// shows now.
    pub fn mark_all_read(&mut self) -> usize {
        let mut marked = 0;
        self.last_persist.clear();
        for index in self.visible() {
            let message = &mut self.messages[index];
            if !message.unread {
                continue;
            }
            message.unread = false;
            message.raw_flags |= rmac_mail_storage::FLAG_SEEN;
            marked += 1;
            if let (Mailbox::Real(real), Some(row_id)) = (&message.mailbox, message.row_id) {
                self.last_persist.push(Persist {
                    account: real.account,
                    account_path: real.account_path.clone(),
                    row_id,
                    change: PersistChange::Flags(message.raw_flags),
                });
            }
        }
        if marked > 0 {
            self.revision += 1;
            // A read mark cannot be undone message by message here; the
            // single-step undo would restore the wrong one.
            self.undo = None;
            self.redo = None;
        }
        marked
    }

    pub fn has_unread_visible(&self) -> bool {
        self.visible()
            .into_iter()
            .any(|index| self.messages[index].unread)
    }

    /// The messages Mailbox ▸ Erase Deleted Items / Erase Junk Mail would
    /// remove for `erase`.
    pub fn erasable_count(&self, erase: Erase) -> usize {
        self.messages
            .iter()
            .filter(|message| erases(message, erase))
            .count()
    }

    /// Permanently removes every message in the chosen Bin or Junk
    /// mailboxes, as Mail's Erase commands do. Not undoable, as on the Mac.
    pub fn erase(&mut self, erase: Erase) -> usize {
        self.last_persist.clear();
        let before = self.messages.len();
        let mut persists = Vec::new();
        self.messages.retain(|message| {
            if !erases(message, erase) {
                return true;
            }
            if let (Mailbox::Real(real), Some(row_id)) = (&message.mailbox, message.row_id) {
                persists.push(Persist {
                    account: real.account,
                    account_path: real.account_path.clone(),
                    row_id,
                    change: PersistChange::Delete,
                });
            }
            false
        });
        let erased = before - self.messages.len();
        if erased > 0 {
            self.revision += 1;
            self.last_persist = persists;
            self.undo = None;
            self.redo = None;
            if self.selected_message().is_none() {
                self.selected = self
                    .visible()
                    .first()
                    .map(|&index| self.messages[index].id.clone());
            }
        }
        erased
    }

    /// The accounts with at least one real mailbox, in sidebar order: Get
    /// New Mail ▸ and Erase Deleted Items ▸ list one row each.
    pub fn accounts(&self) -> Vec<(Uuid, String, String)> {
        let mut accounts: Vec<(Uuid, String, String)> = Vec::new();
        for mailbox in &self.mailboxes {
            if let Mailbox::Real(real) = mailbox {
                if !accounts.iter().any(|(id, _, _)| *id == real.account) {
                    accounts.push((
                        real.account,
                        real.account_label.clone(),
                        real.account_path.clone(),
                    ));
                }
            }
        }
        accounts
    }

    /// The favourite mailboxes Mailbox ▸ Go to Favourite Mailbox lists, in
    /// sidebar order.
    pub fn favourites(&self) -> Vec<Mailbox> {
        self.mailboxes
            .iter()
            .filter(|mailbox| !mailbox.is_real())
            .cloned()
            .collect()
    }

    /// Mailbox ▸ Go to Previous/Next Mailbox: the sidebar row `step` away
    /// from the current one, or with `unread_only` the nearest one in that
    /// direction holding an unread message.
    pub fn adjacent_mailbox(&self, step: i32, unread_only: bool) -> Option<Mailbox> {
        let current = self
            .mailboxes
            .iter()
            .position(|mailbox| *mailbox == self.mailbox)?;
        let mut index = current as i64;
        loop {
            index += i64::from(step);
            let mailbox = self.mailboxes.get(usize::try_from(index).ok()?)?;
            if !unread_only || self.unread_count(mailbox) > 0 {
                return Some(mailbox.clone());
            }
        }
    }

    /// Mailbox ▸ Move to Favourite Mailbox: where the selected message
    /// would land for a favourite — its own account's Inbox, Drafts or Sent.
    /// Flagged is a view, not a place, so it has no target.
    pub fn favourite_target(&self, favourite: &Mailbox) -> Option<Mailbox> {
        let Mailbox::Real(real) = &self.selected_message()?.mailbox else {
            return None;
        };
        let special = match favourite {
            Mailbox::AllInboxes => SpecialUse::Inbox,
            Mailbox::Drafts => SpecialUse::Drafts,
            Mailbox::Sent => SpecialUse::Sent,
            Mailbox::Flagged | Mailbox::Vips | Mailbox::Real(_) | Mailbox::Smart(_) => return None,
        };
        self.find_special(real.account, special)
            .filter(|target| target.special_use() != real.special_use)
    }

    /// The real mailboxes Message ▸ Move to ▸ / Copy to ▸ offer for the
    /// selected message: the others in its own account.
    pub fn destinations(&self) -> Vec<Mailbox> {
        let Some(message) = self.selected_message() else {
            return Vec::new();
        };
        self.mailboxes
            .iter()
            .filter(|mailbox| {
                mailbox.is_real()
                    && mailbox.account() == message.mailbox.account()
                    && **mailbox != message.mailbox
            })
            .cloned()
            .collect()
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

fn erases(message: &Message, erase: Erase) -> bool {
    matches!(&message.mailbox, Mailbox::Real(real)
        if real.special_use == Some(erase.special)
            && erase.account.is_none_or(|account| account == real.account))
}

/// Case-insensitive, as Mail sorts names and subjects.
fn compare_text(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// "Re: Lunch" sorts with "Lunch", as Mail's subject sort ignores reply and
/// forward prefixes.
fn strip_reply_prefix(subject: &str) -> &str {
    let mut subject = subject.trim();
    loop {
        let lower = subject.get(..4).map(str::to_ascii_lowercase);
        match lower.as_deref() {
            Some("re: ") => subject = subject[4..].trim_start(),
            Some("fwd:") => subject = subject[4..].trim_start(),
            _ => return subject,
        }
    }
}

/// `"14 October 2026 at 09:41"`: View ▸ Show Date and Time's long form.
pub fn format_date_time(received_at: i64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_opt(received_at, 0)
        .single()
        .map(|when| when.format("%-d %B %Y at %H:%M").to_string())
        .unwrap_or_default()
}

/// An RFC 5322 rendering of what Mail shows for `message`, for a message
/// whose original source is not cached (fixture data, or synced header
/// only): File ▸ Save As…, Message ▸ Forward as Attachment and View ▸
/// Message ▸ Raw Source use the real source whenever it is.
pub fn message_source(message: &Message) -> String {
    use chrono::TimeZone as _;
    let mut source = String::new();
    let from = if message.sender_address.is_empty() || message.sender_address == message.sender {
        message.sender.clone()
    } else {
        format!("{} <{}>", message.sender, message.sender_address)
    };
    source.push_str(&format!("From: {from}\r\n"));
    if !message.to_addresses.is_empty() {
        source.push_str(&format!("To: {}\r\n", message.to_addresses));
    }
    if !message.cc_addresses.is_empty() {
        source.push_str(&format!("Cc: {}\r\n", message.cc_addresses));
    }
    source.push_str(&format!("Subject: {}\r\n", message.subject));
    if let Some(when) = chrono::Local.timestamp_opt(message.received_at, 0).single() {
        source.push_str(&format!("Date: {}\r\n", when.to_rfc2822()));
    }
    source.push_str("MIME-Version: 1.0\r\n");
    source.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    source.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
    for line in message.body.plain_text().lines() {
        source.push_str(line);
        source.push_str("\r\n");
    }
    source
}

/// View ▸ Message ▸ All Headers: every header of a message source, up to
/// the blank line before its body, with folded lines kept as they came.
pub fn header_block(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    let end = source.find("\n\n").unwrap_or(source.len());
    source[..end].trim_end().to_owned()
}

/// The title and text File ▸ Print… and Export as PDF… lay out for
/// `message`: its headers as Mail shows them, then the body.
pub fn printable_message(message: &Message) -> (String, String) {
    let mut text = format!("From: {}\n", message.sender);
    if !message.to.is_empty() {
        text.push_str(&format!("To: {}\n", message.to));
    }
    if !message.cc.is_empty() {
        text.push_str(&format!("Cc: {}\n", message.cc));
    }
    text.push_str(&format!("Subject: {}\n", message.subject));
    text.push_str(&format!("Date: {}\n\n", message.date_time));
    text.push_str(&message.body.plain_text());
    let title = if message.subject.trim().is_empty() {
        "Untitled".to_owned()
    } else {
        message.subject.trim().chars().take(120).collect()
    };
    (title, text)
}

/// A file name for saving `message` or one of its parts: the subject with
/// path separators and control characters replaced.
pub fn safe_file_stem(subject: &str) -> String {
    let stem: String = subject
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':') {
                '-'
            } else {
                ch
            }
        })
        .take(80)
        .collect();
    let stem = stem.trim_matches(['.', ' ']).to_owned();
    if stem.is_empty() {
        "Untitled".to_owned()
    } else {
        stem
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
    // The fixture accounts' own addresses (`main.rs`), so View ▸ Filter's
    // To: Me and Cc: Me match the messages addressed to them.
    let me = if mailbox.account() == "iCloud" {
        "jacob@icloud.example"
    } else {
        "jacob@example.com"
    };
    let (to, cc, to_addresses, cc_addresses) = match id {
        "anna" => (
            "Jacob Samas".to_owned(),
            "Sam Ortiz".to_owned(),
            me.to_owned(),
            "sam@example.test".to_owned(),
        ),
        // A list mail, sent to the testers with a copy to Jacob.
        "beta" => (
            "Beta Testers".to_owned(),
            "Jacob Samas".to_owned(),
            "testers@lulo.example".to_owned(),
            me.to_owned(),
        ),
        _ => (
            "Jacob Samas".to_owned(),
            String::new(),
            me.to_owned(),
            String::new(),
        ),
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
        received_at: 0,
        date_time: date.to_owned(),
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
        body_state: BodyState::Loaded,
        junk_origin: None,
        size: fixture_size(id, preview),
    }
}

/// A plausible size for each fixture message: its attachment plus a few
/// kilobytes of headers and text.
fn fixture_size(id: &str, preview: &str) -> i64 {
    let attachment = match id {
        "anna" => 217_088,
        "grandma" => 2_516_582,
        "sam" => 1_024,
        _ => 0,
    };
    attachment + 2_048 + 24 * preview.len() as i64
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
        state.filter.enabled = true;
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
        let persist = state
            .take_persists()
            .pop()
            .expect("marking read should persist");
        assert_eq!(persist.account, account);
        assert_eq!(persist.row_id, 42);
        assert_eq!(
            persist.change,
            PersistChange::Flags(rmac_mail_storage::FLAG_SEEN)
        );
    }

    fn visible_ids(state: &MailState) -> Vec<&str> {
        state
            .visible()
            .into_iter()
            .map(|index| state.messages[index].id.as_str())
            .collect()
    }

    #[test]
    fn sort_by_orders_the_list_and_flips_direction() {
        let mut state = MailState::fixture();
        assert_eq!(
            visible_ids(&state),
            [
                "anna",
                "beta",
                "sam",
                "northwind",
                "grandma",
                "bank",
                "climbing",
                "ana"
            ]
        );
        state.set_sort(SortKey::From);
        assert!(!state.descending);
        assert_eq!(visible_ids(&state)[0], "ana");
        assert_eq!(visible_ids(&state)[1], "anna");
        state.descending = true;
        assert_eq!(visible_ids(&state)[0], "sam");
        state.set_sort(SortKey::Date);
        assert!(state.descending);
        state.descending = false;
        assert_eq!(visible_ids(&state)[0], "ana");
        // The conversation shows as its oldest message now.
        assert!(visible_ids(&state).contains(&"sam-old"));
        state.set_sort(SortKey::Unread);
        let ids = visible_ids(&state);
        assert!(ids[..3]
            .iter()
            .all(|id| ["anna", "beta", "grandma"].contains(id)));
        state.set_sort(SortKey::Subject);
        // "Re: Calendar sync spec" sorts under C.
        assert_eq!(visible_ids(&state)[0], "sam");
    }

    #[test]
    fn message_filter_combines_its_groups() {
        let mut state = MailState::fixture();
        state.my_addresses = vec!["jacob@example.com".into(), "jacob@icloud.example".into()];
        state.filter.enabled = true;
        assert_eq!(visible_ids(&state), ["anna", "beta", "grandma"]);
        state.filter.flagged = true;
        assert_eq!(visible_ids(&state).len(), 5);
        state.filter.unread = false;
        state.filter.flagged = false;
        state.filter.cc_me = true;
        assert_eq!(visible_ids(&state), ["beta"]);
        state.filter.cc_me = false;
        state.filter.to_me = true;
        assert!(!visible_ids(&state).contains(&"beta"));
        state.filter.to_me = false;
        state.filter.attachments = true;
        assert_eq!(visible_ids(&state), ["anna", "sam", "grandma"]);
        state.filter.enabled = false;
        assert_eq!(visible_ids(&state).len(), 8);
    }

    #[test]
    fn conversations_expand_and_collapse_all() {
        let mut state = MailState::fixture();
        assert_eq!(state.visible().len(), 8);
        state.expand_all_conversations();
        assert_eq!(state.visible().len(), 10);
        state.collapse_all_conversations();
        assert_eq!(state.visible().len(), 8);
    }

    #[test]
    fn mark_all_read_clears_the_visible_unread_count() {
        let mut state = MailState::fixture();
        assert!(state.has_unread_visible());
        assert_eq!(state.mark_all_read(), 3);
        assert_eq!(state.unread_count(&Mailbox::AllInboxes), 0);
        assert!(!state.has_unread_visible());
    }

    #[test]
    fn erase_removes_bin_or_junk_messages_for_good() {
        let mut state = MailState::fixture();
        assert!(state.apply(OrganizeAction::Delete));
        state.select("beta");
        assert!(state.apply(OrganizeAction::Junk));
        let bin = Erase {
            special: SpecialUse::Trash,
            account: None,
        };
        let junk = Erase {
            special: SpecialUse::Junk,
            account: Some(FIXTURE_ICLOUD_ACCOUNT),
        };
        assert_eq!(state.erasable_count(bin), 1);
        assert_eq!(state.erasable_count(junk), 0);
        assert_eq!(state.erase(bin), 1);
        assert!(!state.messages.iter().any(|message| message.id == "anna"));
        assert!(!state.can_undo());
        assert_eq!(
            state.erase(Erase {
                special: SpecialUse::Junk,
                account: Some(FIXTURE_GOOGLE_ACCOUNT),
            }),
            1
        );
        assert_eq!(state.messages.len(), 8);
    }

    #[test]
    fn saved_and_printed_messages_carry_their_headers() {
        let state = MailState::fixture();
        let anna = state.selected_message().unwrap();
        let source = message_source(anna);
        assert!(source.starts_with("From: Anna Kim <anna@example.test>\r\n"));
        assert!(source.contains("Subject: Lunch on Friday?\r\n"));
        assert!(source.contains("\r\n\r\nHi Jacob,"));
        let (title, text) = printable_message(anna);
        assert_eq!(title, "Lunch on Friday?");
        assert!(text.contains("Cc: Sam Ortiz"));
        assert_eq!(safe_file_stem("Re: a/b"), "Re- a-b");
        assert_eq!(safe_file_stem("  "), "Untitled");
    }

    #[test]
    fn redo_repeats_what_undo_reversed() {
        let mut state = MailState::fixture();
        assert!(state.apply(OrganizeAction::Flag));
        assert!(state.undo());
        assert!(state.can_redo());
        assert!(state.redo());
        assert!(state.messages[0].flagged);
        assert!(!state.can_redo());
    }

    #[test]
    fn mailbox_navigation_and_move_targets() {
        let mut state = MailState::fixture();
        assert_eq!(state.favourites().len(), 4);
        assert_eq!(state.adjacent_mailbox(1, false), Some(Mailbox::Flagged));
        assert_eq!(state.adjacent_mailbox(-1, false), None);
        let next_unread = state.adjacent_mailbox(1, true).unwrap();
        assert_eq!(next_unread.label(), "Inbox");
        assert_eq!(next_unread.account(), "Google");
        assert_eq!(state.favourite_target(&Mailbox::AllInboxes), None);
        let sent = state.favourite_target(&Mailbox::Sent).unwrap();
        assert_eq!(sent.label(), "Sent");
        assert_eq!(state.favourite_target(&Mailbox::Flagged), None);
        let destinations = state.destinations();
        assert_eq!(destinations.len(), 6);
        let receipts = find(&state, "Google", "Receipts");
        assert!(state.apply(OrganizeAction::Move(receipts.clone())));
        state.select("beta");
        assert_eq!(state.move_again_target(), Some(receipts));
        state.select("northwind");
        assert_eq!(state.move_again_target(), None);
        assert_eq!(state.accounts().len(), 2);
    }

    /// MAIL-10: a 10 000-message mailbox must stay smooth. `visible()` and
    /// `select()`/`select_next()` run on every scroll-driven render and
    /// every arrow-key press, so each must stay linear in the message
    /// count rather than the O(n²) a per-row `thread_count()` call would
    /// give `crate::view::list` (fixed alongside this test — see its
    /// `thread_counts` map). The budget is generous (an O(n²) regression
    /// at this size takes tens of seconds, not low single-digit seconds)
    /// to absorb a debug build on a loaded, shared CI runner.
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
            elapsed < std::time::Duration::from_secs(5),
            "20 passes over 10 000 messages took {elapsed:?}, expected a few seconds at most"
        );
    }

    #[test]
    fn flag_colours_round_trip_through_mac_mail_bits() {
        for colour in FlagColour::MENU {
            let flags = colour.apply(rmac_mail_storage::FLAG_SEEN);
            assert_eq!(FlagColour::from_flags(flags), colour);
            assert_ne!(flags & rmac_mail_storage::FLAG_FLAGGED, 0);
            assert_ne!(flags & rmac_mail_storage::FLAG_SEEN, 0);
        }
        // Mac Mail's numbering: purple is $MailFlagBit0 + $MailFlagBit2.
        assert_eq!(
            FlagColour::Purple.apply(0) & rmac_mail_storage::FLAG_COLOUR_MASK,
            rmac_mail_storage::FLAG_COLOUR_BIT0 | rmac_mail_storage::FLAG_COLOUR_BIT2
        );
        assert_eq!(
            FlagColour::Red.apply(0) & rmac_mail_storage::FLAG_COLOUR_MASK,
            0
        );
    }

    #[test]
    fn flag_colour_action_flags_undoes_and_persists() {
        let mut state = MailState::fixture();
        state.select("beta");
        assert!(!state.selected_message().unwrap().flagged);
        assert!(state.apply(OrganizeAction::FlagColour(FlagColour::Blue)));
        let message = state.selected_message().unwrap();
        assert!(message.flagged);
        assert_eq!(message.flag_colour(), FlagColour::Blue);
        // The same colour again changes nothing.
        assert!(!state.apply(OrganizeAction::FlagColour(FlagColour::Blue)));
        assert!(state.apply(OrganizeAction::FlagColour(FlagColour::Green)));
        assert_eq!(
            state.selected_message().unwrap().flag_colour(),
            FlagColour::Green
        );
        assert!(state.undo());
        assert_eq!(
            state.selected_message().unwrap().flag_colour(),
            FlagColour::Blue
        );
    }

    #[test]
    fn sort_by_size_puts_the_largest_first() {
        let mut state = MailState::fixture();
        state.threads = false;
        state.set_sort(SortKey::Size);
        assert!(state.descending);
        let visible = state.visible();
        let sizes: Vec<i64> = visible
            .iter()
            .map(|&index| state.messages[index].size)
            .collect();
        assert!(sizes.windows(2).all(|pair| pair[0] >= pair[1]), "{sizes:?}");
        assert_eq!(state.messages[visible[0]].id, "grandma");
        assert_eq!(size_label(2_516_582), "2.4 MB");
        assert_eq!(size_label(4_096), "4 KB");
        assert_eq!(size_label(12), "12 bytes");
    }

    #[test]
    fn find_in_body_matches_case_insensitively_in_reading_order() {
        let body = rmac_mail_mime::RichText::from_plain("Café Lulo at noon\nSee you at the CAFÉ");
        let matches = find_in_body(&body, "café");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].block, 0);
        assert_eq!(
            &body.blocks[0].spans[0].text[matches[0].range.clone()],
            "Café"
        );
        assert_eq!(matches[1].block, 1);
        assert_eq!(
            &body.blocks[1].spans[0].text[matches[1].range.clone()],
            "CAFÉ"
        );
        assert_eq!(find_in_body(&body, "at").len(), 2);
        assert!(find_in_body(&body, "").is_empty());
        assert!(find_in_body(&body, "tea").is_empty());
        // Overlapping matches count once, as a find bar steps through them.
        let repeated = rmac_mail_mime::RichText::from_plain("aaaa");
        assert_eq!(find_in_body(&repeated, "aa").len(), 2);
    }

    #[test]
    fn header_block_stops_at_the_body() {
        let source =
            "From: Ada <a@example.test>\r\nSubject: Hi\r\n there\r\nX-Priority: 1\r\n\r\nBody\r\n";
        assert_eq!(
            header_block(source),
            "From: Ada <a@example.test>\nSubject: Hi\n there\nX-Priority: 1"
        );
        assert_eq!(header_block("Subject: only"), "Subject: only");
    }

    #[test]
    fn vips_mailbox_appears_with_the_first_vip_and_filters_by_sender() {
        let mut state = MailState::fixture();
        assert!(!state.mailboxes.contains(&Mailbox::Vips));
        let mut people = rmac_contacts::Contacts::default();
        people.set_vip("Anna Kim", "anna@example.test", true);
        state.set_people(people);
        assert_eq!(state.mailboxes[1], Mailbox::Vips);
        state.select_mailbox(Mailbox::Vips);
        let shown: Vec<&str> = state
            .visible()
            .into_iter()
            .map(|index| state.messages[index].id.as_str())
            .collect();
        assert_eq!(shown, vec!["anna"]);
        state.select_mailbox(Mailbox::AllInboxes);
        state.filter.enabled = true;
        state.filter.unread = false;
        state.filter.vip = true;
        assert_eq!(state.visible().len(), 1);
        // A refresh from storage keeps the row; removing the last VIP drops it.
        let (mailboxes, messages) = (
            MailState::fixture().mailboxes,
            MailState::fixture().messages,
        );
        state.refresh_live(mailboxes, messages);
        assert!(state.mailboxes.contains(&Mailbox::Vips));
        state.mailbox = Mailbox::Vips;
        let mut people = state.people.clone();
        people.set_vip("", "anna@example.test", false);
        state.set_people(people);
        assert!(!state.mailboxes.contains(&Mailbox::Vips));
        assert_eq!(state.mailbox, Mailbox::AllInboxes);
    }

    #[test]
    fn rules_move_and_flag_matching_mail_and_skip_the_rest() {
        let mut state = MailState::fixture();
        let rules = vec![rules::Rule {
            description: "Statements".to_owned(),
            conditions: vec![rules::Condition {
                field: rules::Field::Subject,
                operator: rules::Operator::Contains,
                value: "statement".to_owned(),
            }],
            actions: vec![
                rules::Action::MarkAsFlagged {
                    colour: "Blue".to_owned(),
                },
                rules::Action::MoveMessage {
                    to: rules::MailboxRef {
                        account: "Google".to_owned(),
                        name: "Receipts".to_owned(),
                    },
                },
            ],
            ..rules::Rule::default()
        }];
        let ids = vec!["bank".to_owned(), "anna".to_owned()];
        assert_eq!(state.apply_rules(&rules, &ids), 1);
        let bank = state.messages.iter().find(|m| m.id == "bank").unwrap();
        assert_eq!(bank.mailbox.label(), "Receipts");
        assert!(bank.flagged);
        assert_eq!(bank.flag_colour(), FlagColour::Blue);
        let anna = state.messages.iter().find(|m| m.id == "anna").unwrap();
        assert_eq!(anna.mailbox.label(), "Inbox");
        assert!(!state.can_undo(), "rules are not undoable");
        assert_eq!(state.selected.as_deref(), Some("anna"));
    }
}
