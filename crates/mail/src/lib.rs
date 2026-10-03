//! Read-only Mail snapshot and selection model. MAIL-4 will supply snapshots.

use rmac_mail_mime::{sanitize_html, RichText};

pub mod compose;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mailbox {
    AllInboxes,
    Flagged,
    Drafts,
    Sent,
    GoogleInbox,
    GoogleDrafts,
    GoogleSent,
    GoogleJunk,
    GoogleTrash,
    GoogleArchive,
    GoogleReceipts,
    IcloudInbox,
    IcloudSent,
    IcloudJunk,
    IcloudTrash,
}

impl Mailbox {
    pub const ALL: [Self; 15] = [
        Self::AllInboxes,
        Self::Flagged,
        Self::Drafts,
        Self::Sent,
        Self::GoogleInbox,
        Self::GoogleDrafts,
        Self::GoogleSent,
        Self::GoogleJunk,
        Self::GoogleTrash,
        Self::GoogleArchive,
        Self::GoogleReceipts,
        Self::IcloudInbox,
        Self::IcloudSent,
        Self::IcloudJunk,
        Self::IcloudTrash,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::AllInboxes => "All Inboxes",
            Self::Flagged => "Flagged",
            Self::Drafts | Self::GoogleDrafts => "Drafts",
            Self::Sent | Self::GoogleSent | Self::IcloudSent => "Sent",
            Self::GoogleInbox | Self::IcloudInbox => "Inbox",
            Self::GoogleJunk | Self::IcloudJunk => "Junk",
            Self::GoogleTrash | Self::IcloudTrash => "Bin",
            Self::GoogleArchive => "Archive",
            Self::GoogleReceipts => "Receipts",
        }
    }

    pub const fn account(self) -> &'static str {
        match self {
            Self::AllInboxes | Self::Flagged | Self::Drafts | Self::Sent => "Favourites",
            Self::IcloudInbox | Self::IcloudSent | Self::IcloudJunk | Self::IcloudTrash => "iCloud",
            _ => "Google",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: &'static str,
    pub mailbox: Mailbox,
    pub sender: &'static str,
    pub sender_address: &'static str,
    pub initials: &'static str,
    pub date: &'static str,
    pub to: &'static str,
    pub cc: &'static str,
    pub to_addresses: &'static str,
    pub cc_addresses: &'static str,
    pub subject: &'static str,
    pub preview: &'static str,
    pub unread: bool,
    pub flagged: bool,
    pub attachment: Option<(&'static str, &'static str)>,
    pub thread_id: &'static str,
    pub body: RichText,
}

pub struct MailState {
    pub messages: Vec<Message>,
    pub mailbox: Mailbox,
    pub selected: Option<&'static str>,
    pub threads: bool,
    pub unread_only: bool,
}

impl MailState {
    pub fn fixture() -> Self {
        let data = [
            (
                "anna",
                Mailbox::GoogleInbox,
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
                Mailbox::GoogleInbox,
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
                Mailbox::GoogleInbox,
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
                "northwind",
                Mailbox::IcloudInbox,
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
                "grandma",
                Mailbox::GoogleInbox,
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
                Mailbox::GoogleInbox,
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
                Mailbox::GoogleInbox,
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
                "ana",
                Mailbox::IcloudInbox,
                "Ana Ruiz",
                "AR",
                "Monday",
                "Weekend plans",
                "Are we still on for the farmers market on Saturday?",
                false,
                false,
                "weekend",
            ),
            (
                "sam-old",
                Mailbox::GoogleInbox,
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
                Mailbox::GoogleInbox,
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
        let messages = data.into_iter().map(|(id, mailbox, sender, initials, date, subject, preview, unread, flagged, thread_id)| {
            let body = if id == "anna" {
                sanitize_html("<p>Hi Jacob,</p><p>Café Lulo at 12:30 works for me. I booked the table by the window — tell me if Sam is coming too so I can change it.</p><p>I attached the menu in case you want to pre-order.</p><p>Anna</p><blockquote>On 2 Oct 2026, at 18:05, Jacob Samas wrote:<br>Shall we do lunch on Friday?</blockquote><img src='https://example.invalid/tracker'>")
            } else {
                RichText::from_plain(preview)
            };
            Message { id, mailbox, sender, sender_address: if id == "anna" { "anna@example.test" } else if id == "sam" { "sam@example.test" } else { "sender@example.test" }, initials, date, to: "Jacob Samas", cc: if id == "anna" { "Sam Ortiz" } else { "" }, to_addresses: "jacob@example.test", cc_addresses: if id == "anna" { "sam@example.test" } else { "" }, subject, preview, unread, flagged, attachment: if id == "anna" { Some(("Menu.pdf", "212 KB")) } else if id == "grandma" { Some(("Photos.zip", "2.4 MB")) } else { None }, thread_id, body }
        }).collect();
        Self {
            messages,
            mailbox: Mailbox::AllInboxes,
            selected: Some("anna"),
            threads: true,
            unread_only: false,
        }
    }

    pub fn visible(&self) -> Vec<usize> {
        let mut seen = std::collections::HashSet::new();
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                let in_mailbox = match self.mailbox {
                    Mailbox::AllInboxes => {
                        matches!(message.mailbox, Mailbox::GoogleInbox | Mailbox::IcloudInbox)
                    }
                    Mailbox::Flagged => message.flagged,
                    Mailbox::Drafts | Mailbox::Sent => false,
                    folder => message.mailbox == folder,
                };
                if !in_mailbox
                    || self.unread_only && !message.unread
                    || self.threads && !seen.insert(message.thread_id)
                {
                    None
                } else {
                    Some(index)
                }
            })
            .collect()
    }

    pub fn select(&mut self, id: &'static str) {
        self.selected = Some(id);
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            message.unread = false;
        }
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .find(|message| Some(message.id) == self.selected)
    }

    pub fn select_next(&mut self, direction: i32) {
        let visible = self.visible();
        if visible.is_empty() {
            self.selected = None;
            return;
        }
        let current = visible
            .iter()
            .position(|&index| Some(self.messages[index].id) == self.selected)
            .unwrap_or(0);
        let next = (current as i32 + direction).clamp(0, visible.len() as i32 - 1) as usize;
        self.select(self.messages[visible[next]].id);
    }

    pub fn select_mailbox(&mut self, mailbox: Mailbox) {
        self.mailbox = mailbox;
        if let Some(id) = self.visible().first().map(|&index| self.messages[index].id) {
            self.select(id);
        } else {
            self.selected = None;
        }
    }

    pub fn thread_count(&self, thread_id: &str) -> usize {
        self.messages
            .iter()
            .filter(|message| message.thread_id == thread_id)
            .count()
    }

    pub fn unread_count(&self, mailbox: Mailbox) -> usize {
        self.messages
            .iter()
            .filter(|message| {
                message.unread
                    && match mailbox {
                        Mailbox::AllInboxes => {
                            matches!(message.mailbox, Mailbox::GoogleInbox | Mailbox::IcloudInbox)
                        }
                        Mailbox::Flagged => message.flagged,
                        folder => message.mailbox == folder,
                    }
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_message_marks_read_and_updates_unread_counts() {
        let mut state = MailState::fixture();
        assert_eq!(state.unread_count(Mailbox::AllInboxes), 3);
        state.select("anna");
        assert!(!state.selected_message().unwrap().unread);
        assert_eq!(state.unread_count(Mailbox::AllInboxes), 2);
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
        state.select_mailbox(Mailbox::IcloudInbox);
        assert_eq!(state.visible().len(), 2);
        state.unread_only = true;
        assert!(state.visible().is_empty());
        state.select_mailbox(Mailbox::GoogleInbox);
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
}
