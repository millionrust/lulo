use crate::Error;
use imap_codec::{decode::Decoder, ResponseCodec};

const MAX_RESPONSE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Capabilities(Vec<String>);

impl Capabilities {
    pub fn has(&self, capability: &str) -> bool {
        self.0
            .iter()
            .any(|item| item.eq_ignore_ascii_case(capability))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxKind {
    Inbox,
    Drafts,
    Sent,
    Trash,
    Junk,
    Archive,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub name: String,
    pub kind: MailboxKind,
    pub selectable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncCursor {
    pub uid_validity: u64,
    pub highest_modseq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectState {
    pub exists: u32,
    pub uid_validity: Option<u64>,
    pub uid_next: Option<u32>,
    pub highest_modseq: Option<u64>,
    pub vanished: Vec<String>,
    pub changed: Vec<MessageChange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageChange {
    pub uid: u32,
    pub flags: Vec<String>,
    pub modseq: Option<u64>,
}

pub(crate) struct Response {
    raw: Vec<u8>,
    literal: Option<Vec<u8>>,
}

impl Response {
    pub(crate) fn new(raw: Vec<u8>, literal: Option<Vec<u8>>) -> Result<Self, Error> {
        if raw.len() > MAX_RESPONSE {
            return Err(Error::Protocol("IMAP response is too large"));
        }
        // Validate ordinary tagged completions with the reviewed codec.
        // Response codes and unsolicited extension data may be unknown to
        // the stable codec; those are bounded and parsed by the client.
        let first = raw.split(|byte| *byte == b'\r').next().unwrap_or_default();
        if first.first() == Some(&b'L')
            && !first.contains(&b'[')
            && ResponseCodec::new().decode(&raw).is_err()
        {
            return Err(Error::Protocol("Invalid IMAP command completion"));
        }
        Ok(Self { raw, literal })
    }

    pub(crate) fn first_line(&self) -> &[u8] {
        self.raw
            .split(|byte| *byte == b'\r')
            .next()
            .unwrap_or_default()
    }

    pub(crate) fn first_literal(&self) -> Option<Vec<u8>> {
        self.literal.clone()
    }

    pub(crate) fn safe_event(&self) -> String {
        let line = String::from_utf8_lossy(self.first_line());
        if line.contains(" EXISTS") {
            "exists".into()
        } else if line.contains(" EXPUNGE") {
            "expunge".into()
        } else if line.contains(" FETCH") {
            "fetch".into()
        } else if line.starts_with("* VANISHED") {
            "vanished".into()
        } else {
            "update".into()
        }
    }
}

pub(crate) fn parse_capabilities(responses: &[Response]) -> Capabilities {
    let mut found = Vec::new();
    for response in responses {
        let line = String::from_utf8_lossy(response.first_line());
        if let Some(rest) = line.strip_prefix("* CAPABILITY ") {
            found.extend(rest.split_ascii_whitespace().map(str::to_ascii_uppercase));
        }
    }
    Capabilities(found)
}

pub(crate) fn parse_list(responses: &[Response]) -> Vec<Mailbox> {
    let mut mailboxes = Vec::new();
    for response in responses {
        let line = String::from_utf8_lossy(response.first_line());
        let Some(rest) = line.strip_prefix("* LIST (") else {
            continue;
        };
        let Some((flags, tail)) = rest.split_once(") ") else {
            continue;
        };
        let Some((_, name)) = read_token(tail).and_then(|(_, rest)| read_token(rest)) else {
            continue;
        };
        let name = name.to_string();
        let kind = if name.eq_ignore_ascii_case("INBOX") {
            MailboxKind::Inbox
        } else if flags.contains("\\Drafts") {
            MailboxKind::Drafts
        } else if flags.contains("\\Sent") {
            MailboxKind::Sent
        } else if flags.contains("\\Trash") {
            MailboxKind::Trash
        } else if flags.contains("\\Junk") {
            MailboxKind::Junk
        } else if flags.contains("\\Archive") {
            MailboxKind::Archive
        } else {
            MailboxKind::Other
        };
        mailboxes.push(Mailbox {
            name,
            kind,
            selectable: !flags.contains("\\Noselect"),
        });
    }
    mailboxes
}

pub(crate) fn parse_select(responses: &[Response]) -> Result<SelectState, Error> {
    let mut state = SelectState::default();
    for response in responses {
        let line = String::from_utf8_lossy(response.first_line());
        if let Some(count) = line
            .strip_prefix("* ")
            .and_then(|s| s.strip_suffix(" EXISTS"))
        {
            state.exists = count
                .parse()
                .map_err(|_| Error::Protocol("Invalid EXISTS count"))?;
        }
        state.uid_validity = state
            .uid_validity
            .or_else(|| bracket_number(&line, "UIDVALIDITY"));
        state.uid_next = state
            .uid_next
            .or_else(|| bracket_number(&line, "UIDNEXT").and_then(|n| n.try_into().ok()));
        state.highest_modseq = state
            .highest_modseq
            .or_else(|| bracket_number(&line, "HIGHESTMODSEQ"));
        if let Some(uids) = line.strip_prefix("* VANISHED ") {
            state
                .vanished
                .push(uids.trim_start_matches("(EARLIER) ").to_string());
        }
        if let Some(change) = parse_fetch(response) {
            state.changed.push(change);
        }
    }
    Ok(state)
}

pub(crate) fn parse_uid_fetch(responses: &[Response]) -> Vec<MessageChange> {
    responses.iter().filter_map(parse_fetch).collect()
}

fn parse_fetch(response: &Response) -> Option<MessageChange> {
    let line = String::from_utf8_lossy(response.first_line());
    if !line.starts_with("* ") || !line.contains(" FETCH (") {
        return None;
    }
    let uid = field_number(&line, "UID ")?.try_into().ok()?;
    let flags = line
        .split_once("FLAGS (")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(flags, _)| flags.split_ascii_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    let modseq = line
        .split_once("MODSEQ (")
        .and_then(|(_, rest)| rest.split_once(')'))
        .and_then(|(value, _)| value.parse().ok());
    Some(MessageChange { uid, flags, modseq })
}

fn bracket_number(line: &str, name: &str) -> Option<u64> {
    line.split_once(&format!("[{name} "))
        .and_then(|(_, rest)| rest.split_once(']'))
        .and_then(|(value, _)| value.parse().ok())
}

fn field_number(line: &str, name: &str) -> Option<u64> {
    line.split_once(name)
        .and_then(|(_, rest)| rest.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|value| value.parse().ok())
}

fn read_token(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if let Some(rest) = input.strip_prefix('"') {
        let end = rest.find('"')?;
        Some((&rest[..end], &rest[end + 1..]))
    } else {
        let end = input.find(' ').unwrap_or(input.len());
        Some((&input[..end], &input[end..]))
    }
}

pub(crate) fn quote(value: &str) -> Result<String, Error> {
    if value
        .chars()
        .any(|ch| ch == '\r' || ch == '\n' || ch == '\0')
    {
        return Err(Error::Protocol("Invalid IMAP mailbox name"));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

pub(crate) fn validate_uid_set(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b',' | b':' | b'*'))
    {
        return Err(Error::Protocol("Invalid IMAP UID set"));
    }
    Ok(())
}
