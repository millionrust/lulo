//! Builds `MailState` from each discovered account's live `rmac-mail-storage`
//! cache (MAIL-4), and persists the window's organise/read actions back into
//! it. Every function here does blocking disk I/O and must run off the GPUI
//! thread — `view.rs` calls these through `cx.background_executor()`,
//! mirroring `compose_window.rs`'s pattern for drafts and attachments.

use chrono::{Datelike, Local, TimeZone};
use rmac_mail::{
    Mailbox, Message, MessageAttachment, Persist, PersistChange, RealMailbox, SpecialUse,
};
use rmac_mail_storage::{Change, MailStorage, MessageSummary};
use uuid::Uuid;

use crate::delivery::{data_root, ComposeAccount};

/// Caps how many of a mailbox's newest messages load into memory at once.
/// The virtual list only ever renders a window of rows, so this bounds
/// Mail's resident memory independently of how large the real mailbox is
/// (MAIL-10's memory soak).
const MAILBOX_LOAD_LIMIT: usize = 20_000;

fn special_use_of(name: Option<&str>) -> Option<SpecialUse> {
    match name? {
        "\\Inbox" => Some(SpecialUse::Inbox),
        "\\Drafts" => Some(SpecialUse::Drafts),
        "\\Sent" => Some(SpecialUse::Sent),
        "\\Junk" => Some(SpecialUse::Junk),
        "\\Trash" => Some(SpecialUse::Trash),
        "\\Archive" => Some(SpecialUse::Archive),
        _ => None,
    }
}

/// `sender`/`sender_address`/`initials` from a `"Display Name <addr>"` or a
/// bare address, matching whichever form the IMAP sync stored.
fn split_sender(raw: &str) -> (String, String) {
    if let Some((name, rest)) = raw.rsplit_once('<') {
        let address = rest.trim_end_matches('>').trim();
        let name = name.trim();
        if !name.is_empty() {
            return (name.to_owned(), address.to_owned());
        }
        return (address.to_owned(), address.to_owned());
    }
    (raw.to_owned(), raw.to_owned())
}

fn initials_of(display_name: &str) -> String {
    let initials: String = display_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .map(|ch| ch.to_ascii_uppercase())
        .collect();
    if initials.is_empty() {
        "?".to_owned()
    } else {
        initials
    }
}

/// `"Today at 09:41"` / `"Yesterday"` / a weekday name / `"5 Oct 2025"`,
/// matching the Mac's message list date style.
fn format_date(received_at: i64) -> String {
    let Some(when) = Local.timestamp_opt(received_at, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = (now.date_naive() - when.date_naive()).num_days();
    if days == 0 {
        when.format("Today at %H:%M").to_string()
    } else if days == 1 {
        "Yesterday".to_owned()
    } else if (2..7).contains(&days) {
        when.format("%A").to_string()
    } else if when.year() == now.year() {
        when.format("%-d %b").to_string()
    } else {
        when.format("%-d %b %Y").to_string()
    }
}

fn summary_to_message(mailbox: Mailbox, account: Uuid, summary: MessageSummary) -> Message {
    let (sender, sender_address) = split_sender(&summary.sender);
    let initials = initials_of(&sender);
    let unread = summary.flags & rmac_mail_storage::FLAG_SEEN == 0;
    let flagged = summary.flags & rmac_mail_storage::FLAG_FLAGGED != 0;
    let body = rmac_mail_mime::RichText::from_plain(&summary.preview);
    let id = format!("{account}:{}:{}", summary.mailbox_id, summary.id);
    // Real conversation grouping (RFC 5322 References/In-Reply-To, via
    // `MailStorage::threads`) is deferred (`docs/parity.md` MAIL-4), so
    // each live message is its own one-row thread: a shared id here would
    // wrongly collapse every message in a mailbox into one when View ▸
    // Organize by Conversation (on by default) is active.
    let thread_id = id.clone();
    Message {
        id,
        mailbox,
        row_id: Some(summary.id),
        raw_flags: summary.flags,
        sender,
        sender_address,
        initials,
        date: format_date(summary.received_at),
        to: summary.recipients.clone(),
        cc: summary.cc.clone(),
        to_addresses: summary.recipients,
        cc_addresses: summary.cc,
        subject: summary.subject,
        preview: summary.preview,
        unread,
        flagged,
        attachment: None,
        // A real conversation groups by RFC 5322 References/In-Reply-To
        // (`rmac_mail_store::thread_messages`, driven from
        // `MailStorage::threads`); loading that forest eagerly for every
        // mailbox on startup is deferred, so each live message is its own
        // one-row thread for now (`docs/parity.md` MAIL-4).
        thread_id,
        body,
        body_loaded: false,
        junk_origin: None,
    }
}

/// Every discovered account's mailboxes and messages, ready for
/// `MailState::new`. Returns the four Favourites with no real mailboxes
/// when `accounts` is empty or storage cannot be opened — `main.rs` shows
/// the "no accounts" empty state in that case rather than ever drawing from
/// a fixture.
pub fn load(accounts: &[ComposeAccount]) -> (Vec<Mailbox>, Vec<Message>) {
    let mut mailboxes = vec![
        Mailbox::AllInboxes,
        Mailbox::Flagged,
        Mailbox::Drafts,
        Mailbox::Sent,
    ];
    let mut messages = Vec::new();
    let Some(root) = data_root() else {
        return (mailboxes, messages);
    };
    for account in accounts {
        let Ok(storage) = MailStorage::open(&root, account.id) else {
            continue;
        };
        let Ok(listing) = storage.all_mailboxes() else {
            continue;
        };
        for entry in listing {
            let mailbox = Mailbox::Real(RealMailbox {
                account: account.id,
                account_path: account.path.clone(),
                account_label: account.address.clone(),
                mailbox_id: entry.id,
                name: entry.name.clone(),
                special_use: special_use_of(entry.special_use.as_deref()),
            });
            mailboxes.push(mailbox.clone());
            let Ok(summaries) = storage.messages_in_mailbox(entry.id, MAILBOX_LOAD_LIMIT) else {
                continue;
            };
            messages.extend(
                summaries
                    .into_iter()
                    .map(|summary| summary_to_message(mailbox.clone(), account.id, summary)),
            );
        }
    }
    (mailboxes, messages)
}

/// Fetches and parses one message's full body, off the GPUI thread, for
/// `MailState::set_loaded_body` once a lightweight live row is selected. A
/// message synced header-only (no cached blob yet) falls back to its
/// preview text rather than blocking on a network fetch.
pub fn load_body(
    account: Uuid,
    mailbox_id: i64,
    row_id: i64,
) -> Option<(rmac_mail_mime::RichText, Option<MessageAttachment>)> {
    let root = data_root()?;
    let storage = MailStorage::open(&root, account).ok()?;
    let summary = storage.get_message(row_id).ok()??;
    if summary.mailbox_id != mailbox_id {
        return None;
    }
    let Some(hash) = &summary.body_hash else {
        return Some((rmac_mail_mime::RichText::from_plain(&summary.preview), None));
    };
    let Ok(bytes) = storage.read_blob(hash) else {
        return Some((rmac_mail_mime::RichText::from_plain(&summary.preview), None));
    };
    let Ok(parsed) = rmac_mail_mime::parse(&bytes) else {
        return Some((rmac_mail_mime::RichText::from_plain(&summary.preview), None));
    };
    let attachment = parsed
        .attachments
        .into_iter()
        .find(|attachment| attachment.content_id.is_none())
        .map(|attachment| MessageAttachment {
            filename: attachment.filename,
            size_label: human_size(attachment.bytes.len()),
            bytes: attachment.bytes,
        });
    Some((parsed.rich_text, attachment))
}

fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{value:.0} B")
    } else if value < KB * KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

/// Replays one organise/read action into its account's storage journal, for
/// `rmac_mail_runtime`'s worker to send to the server on its next wake.
/// Never called for fixture data (`Persist` only exists for a message with
/// a real `row_id`).
pub fn persist(change: Persist) {
    let Some(root) = data_root() else {
        return;
    };
    let Ok(mut storage) = MailStorage::open(&root, change.account) else {
        return;
    };
    let journal_change = match change.change {
        PersistChange::Flags(bits) => Change::SetFlags(bits),
        PersistChange::Move(target) => Change::Move(target),
    };
    let _ = storage.queue_change(change.row_id, journal_change);
}
