//! Mail ▸ Settings ▸ Rules and Message ▸ Apply Rules (⌥⌘L): local rules
//! that act on new mail as it arrives in an Inbox, and on the selection on
//! demand. The model and matcher are pure; `MailState::apply_rules` turns
//! a match into the same organise actions (and server journal entries) as
//! the menu commands.
//!
//! Persisted beside Mail's settings at `~/.config/lulo/mail-rules.json`,
//! directory 0700, file 0600, atomic writes.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{FlagColour, Message};

const VERSION: u32 = 1;
const FILE_NAME: &str = "mail-rules.json";
const MAX_FILE_BYTES: usize = 512 * 1024;

/// What a condition looks at.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Field {
    #[default]
    From,
    To,
    Cc,
    Subject,
    AnyRecipient,
    /// "Sender is in my contacts" (no text).
    SenderInContacts,
    /// "Sender is not in my contacts" (no text).
    SenderNotInContacts,
    /// "Sender is VIP" (no text).
    SenderIsVip,
}

impl Field {
    pub const ALL: [Self; 8] = [
        Self::From,
        Self::To,
        Self::Cc,
        Self::Subject,
        Self::AnyRecipient,
        Self::SenderInContacts,
        Self::SenderNotInContacts,
        Self::SenderIsVip,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::From => "From",
            Self::To => "To",
            Self::Cc => "Cc",
            Self::Subject => "Subject",
            Self::AnyRecipient => "Any Recipient",
            Self::SenderInContacts => "Sender is in my Contacts",
            Self::SenderNotInContacts => "Sender is not in my Contacts",
            Self::SenderIsVip => "Sender is VIP",
        }
    }

    /// Whether the row has an operator and a text field.
    pub const fn takes_text(self) -> bool {
        !matches!(
            self,
            Self::SenderInContacts | Self::SenderNotInContacts | Self::SenderIsVip
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Operator {
    #[default]
    Contains,
    DoesNotContain,
    BeginsWith,
    EndsWith,
    IsEqualTo,
}

impl Operator {
    pub const ALL: [Self; 5] = [
        Self::Contains,
        Self::DoesNotContain,
        Self::BeginsWith,
        Self::EndsWith,
        Self::IsEqualTo,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::DoesNotContain => "does not contain",
            Self::BeginsWith => "begins with",
            Self::EndsWith => "ends with",
            Self::IsEqualTo => "is equal to",
        }
    }

    fn test(self, haystack: &str, needle: &str) -> bool {
        let haystack = haystack.to_lowercase();
        let needle = needle.trim().to_lowercase();
        match self {
            Self::Contains => haystack.contains(&needle),
            Self::DoesNotContain => !haystack.contains(&needle),
            Self::BeginsWith => haystack.starts_with(&needle),
            Self::EndsWith => haystack.ends_with(&needle),
            Self::IsEqualTo => haystack == needle,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Condition {
    pub field: Field,
    pub operator: Operator,
    pub value: String,
}

/// A real mailbox a rule files into, by account and name, so it survives
/// a cache rebuild that renumbers mailbox rows.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct MailboxRef {
    pub account: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    MoveMessage {
        to: MailboxRef,
    },
    CopyMessage {
        to: MailboxRef,
    },
    /// "Mark as Flagged" in a flag colour (Mac Mail's colour pop-up).
    MarkAsFlagged {
        colour: String,
    },
    MarkAsRead,
    DeleteMessage,
    StopEvaluatingRules,
}

impl Action {
    /// The kinds a rule's action pop-up offers, with a fresh value each.
    pub fn kinds() -> [Action; 6] {
        [
            Action::MoveMessage {
                to: MailboxRef::default(),
            },
            Action::CopyMessage {
                to: MailboxRef::default(),
            },
            Action::MarkAsFlagged {
                colour: FlagColour::Red.label().to_owned(),
            },
            Action::MarkAsRead,
            Action::DeleteMessage,
            Action::StopEvaluatingRules,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::MoveMessage { .. } => "Move Message",
            Self::CopyMessage { .. } => "Copy Message",
            Self::MarkAsFlagged { .. } => "Mark as Flagged",
            Self::MarkAsRead => "Mark as Read",
            Self::DeleteMessage => "Delete Message",
            Self::StopEvaluatingRules => "Stop evaluating rules",
        }
    }

    pub fn same_kind(&self, other: &Action) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// The flag colour of a Mark as Flagged action.
    pub fn flag_colour(&self) -> Option<FlagColour> {
        match self {
            Self::MarkAsFlagged { colour } => Some(
                FlagColour::MENU
                    .into_iter()
                    .find(|candidate| candidate.label() == colour)
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Rule {
    pub description: String,
    pub enabled: bool,
    /// "If any (true) / all (false) of the following conditions are met".
    pub any: bool,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

impl Default for Rule {
    /// A new rule as Mac Mail's sheet opens it: From contains "", Move
    /// Message.
    fn default() -> Self {
        Self {
            description: String::new(),
            enabled: true,
            any: true,
            conditions: vec![Condition::default()],
            actions: vec![Action::MoveMessage {
                to: MailboxRef::default(),
            }],
        }
    }
}

/// What a condition about the sender's standing needs to know.
pub trait People {
    fn knows(&self, address: &str) -> bool;
    fn is_vip(&self, address: &str) -> bool;
}

impl People for rmac_contacts::Contacts {
    fn knows(&self, address: &str) -> bool {
        rmac_contacts::Contacts::knows(self, address)
    }
    fn is_vip(&self, address: &str) -> bool {
        rmac_contacts::Contacts::is_vip(self, address)
    }
}

impl Condition {
    pub fn matches(&self, message: &Message, people: &dyn People) -> bool {
        let text = |value: &str| self.operator.test(value, &self.value);
        match self.field {
            Field::From => {
                let full = format!("{} <{}>", message.sender, message.sender_address);
                let forms = [message.sender.as_str(), &message.sender_address, &full];
                if self.operator == Operator::DoesNotContain {
                    forms.iter().all(|form| text(form))
                } else {
                    forms.iter().any(|form| text(form))
                }
            }
            Field::To => any_address(&message.to, &message.to_addresses, &text, self.operator),
            Field::Cc => any_address(&message.cc, &message.cc_addresses, &text, self.operator),
            Field::AnyRecipient => {
                let to = any_address(&message.to, &message.to_addresses, &text, self.operator);
                let cc = any_address(&message.cc, &message.cc_addresses, &text, self.operator);
                if self.operator == Operator::DoesNotContain {
                    to && cc
                } else {
                    to || cc
                }
            }
            Field::Subject => text(&message.subject),
            Field::SenderInContacts => people.knows(&message.sender_address),
            Field::SenderNotInContacts => !people.knows(&message.sender_address),
            Field::SenderIsVip => people.is_vip(&message.sender_address),
        }
    }
}

/// A recipient field matches if any one name or address in it does; for
/// "does not contain", none may.
fn any_address(
    names: &str,
    addresses: &str,
    text: &dyn Fn(&str) -> bool,
    operator: Operator,
) -> bool {
    let parts: Vec<&str> = names
        .split([',', ';'])
        .chain(addresses.split([',', ';']))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return text("");
    }
    if operator == Operator::DoesNotContain {
        parts.iter().all(|part| text(part))
    } else {
        parts.iter().any(|part| text(part))
    }
}

impl Rule {
    pub fn matches(&self, message: &Message, people: &dyn People) -> bool {
        if !self.enabled || self.conditions.is_empty() {
            return false;
        }
        if self.any {
            self.conditions
                .iter()
                .any(|condition| condition.matches(message, people))
        } else {
            self.conditions
                .iter()
                .all(|condition| condition.matches(message, people))
        }
    }
}

/// The actions to take on `message`, in order: every matching rule's,
/// until one says Stop evaluating rules.
pub fn actions_for<'a>(
    rules: &'a [Rule],
    message: &Message,
    people: &dyn People,
) -> Vec<&'a Action> {
    let mut out = Vec::new();
    for rule in rules {
        if !rule.matches(message, people) {
            continue;
        }
        for action in &rule.actions {
            if *action == Action::StopEvaluatingRules {
                return out;
            }
            out.push(action);
        }
    }
    out
}

#[derive(Deserialize, Serialize)]
struct Stored {
    version: u32,
    rules: Vec<Rule>,
}

fn rules_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let config = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".config"),
    };
    config.join("lulo").join(FILE_NAME)
}

/// The saved rules (none until some are added). Blocking: off the UI thread.
pub fn load() -> io::Result<Vec<Rule>> {
    load_from(&rules_path())
}

/// Blocking: off the UI thread.
pub fn save(rules: &[Rule]) -> io::Result<()> {
    save_to(&rules_path(), rules)
}

pub fn load_from(path: &Path) -> io::Result<Vec<Rule>> {
    let bytes = match rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let stored: Stored = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if stored.version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "mail-rules.json version {} is not supported",
                stored.version
            ),
        ));
    }
    Ok(stored.rules)
}

pub fn save_to(path: &Path, rules: &[Rule]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let stored = Stored {
        version: VERSION,
        rules: rules.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MailState;

    struct Fake;
    impl People for Fake {
        fn knows(&self, address: &str) -> bool {
            address == "anna@example.test"
        }
        fn is_vip(&self, address: &str) -> bool {
            address == "anna@example.test"
        }
    }

    fn message(id: &str) -> Message {
        MailState::fixture()
            .messages
            .into_iter()
            .find(|message| message.id == id)
            .unwrap()
    }

    fn rule(field: Field, operator: Operator, value: &str, actions: Vec<Action>) -> Rule {
        Rule {
            description: "test".to_owned(),
            conditions: vec![Condition {
                field,
                operator,
                value: value.to_owned(),
            }],
            actions,
            ..Rule::default()
        }
    }

    #[test]
    fn text_operators_ignore_case_and_cover_names_and_addresses() {
        let anna = message("anna");
        for (operator, value, expected) in [
            (Operator::Contains, "ANNA", true),
            (Operator::Contains, "example.test", true),
            (Operator::DoesNotContain, "anna", false),
            (Operator::BeginsWith, "anna", true),
            (Operator::EndsWith, ".test", true),
            (Operator::IsEqualTo, "anna@example.test", true),
            (Operator::IsEqualTo, "anna", false),
        ] {
            let condition = Condition {
                field: Field::From,
                operator,
                value: value.to_owned(),
            };
            assert_eq!(
                condition.matches(&anna, &Fake),
                expected,
                "{operator:?} {value}"
            );
        }
        let recipients = Condition {
            field: Field::AnyRecipient,
            operator: Operator::Contains,
            value: "sam@".to_owned(),
        };
        assert!(recipients.matches(&anna, &Fake), "Cc: sam@example.test");
        let subject = Condition {
            field: Field::Subject,
            operator: Operator::Contains,
            value: "lunch".to_owned(),
        };
        assert_eq!(
            subject.matches(&anna, &Fake),
            anna.subject.to_lowercase().contains("lunch")
        );
    }

    #[test]
    fn contacts_and_vip_conditions_ask_the_people_store() {
        let anna = message("anna");
        let sam = message("sam");
        let vip = Condition {
            field: Field::SenderIsVip,
            ..Condition::default()
        };
        assert!(vip.matches(&anna, &Fake));
        assert!(!vip.matches(&sam, &Fake));
        let stranger = Condition {
            field: Field::SenderNotInContacts,
            ..Condition::default()
        };
        assert!(stranger.matches(&sam, &Fake));
    }

    #[test]
    fn any_or_all_and_stop_evaluating() {
        let anna = message("anna");
        let mut both = rule(
            Field::From,
            Operator::Contains,
            "anna",
            vec![Action::MarkAsRead],
        );
        both.conditions.push(Condition {
            field: Field::Subject,
            operator: Operator::Contains,
            value: "no such subject".to_owned(),
        });
        assert!(both.matches(&anna, &Fake));
        both.any = false;
        assert!(!both.matches(&anna, &Fake));
        let rules = vec![
            rule(
                Field::From,
                Operator::Contains,
                "anna",
                vec![Action::MarkAsRead, Action::StopEvaluatingRules],
            ),
            rule(
                Field::From,
                Operator::Contains,
                "anna",
                vec![Action::DeleteMessage],
            ),
        ];
        assert_eq!(actions_for(&rules, &anna, &Fake), vec![&Action::MarkAsRead]);
        let mut disabled = rules.clone();
        disabled[0].enabled = false;
        assert_eq!(
            actions_for(&disabled, &anna, &Fake),
            vec![&Action::DeleteMessage]
        );
    }

    #[test]
    fn rules_round_trip_through_their_file() {
        let dir = std::env::temp_dir().join(format!("rmac-mail-rules-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let rules = vec![rule(
            Field::To,
            Operator::EndsWith,
            "@example.test",
            vec![
                Action::MarkAsFlagged {
                    colour: "Blue".to_owned(),
                },
                Action::MoveMessage {
                    to: MailboxRef {
                        account: "jacob@example.com".to_owned(),
                        name: "Receipts".to_owned(),
                    },
                },
            ],
        )];
        save_to(&path, &rules).unwrap();
        assert_eq!(load_from(&path).unwrap(), rules);
        assert_eq!(rules[0].actions[0].flag_colour(), Some(FlagColour::Blue));
        assert!(load_from(&dir.join("none.json")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
