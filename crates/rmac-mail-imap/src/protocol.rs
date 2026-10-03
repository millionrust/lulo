use crate::Error;
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyUid {
    pub uid_validity: u64,
    pub source_uids: String,
    pub destination_uids: String,
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
        let Some((_, rest)) = read_token(tail) else {
            continue;
        };
        let encoded_name = if rest.trim_start().starts_with('{') {
            response
                .first_literal()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        } else {
            read_token(rest).map(|(name, _)| name)
        };
        let Some(name) = encoded_name.and_then(|name| decode_mailbox(&name)) else {
            continue;
        };
        let kind = if name.eq_ignore_ascii_case("INBOX") {
            MailboxKind::Inbox
        } else if has_flag(flags, "\\Drafts") {
            MailboxKind::Drafts
        } else if has_flag(flags, "\\Sent") {
            MailboxKind::Sent
        } else if has_flag(flags, "\\Trash") {
            MailboxKind::Trash
        } else if has_flag(flags, "\\Junk") {
            MailboxKind::Junk
        } else if has_flag(flags, "\\Archive") || has_flag(flags, "\\All") {
            MailboxKind::Archive
        } else {
            MailboxKind::Other
        };
        mailboxes.push(Mailbox {
            name,
            kind,
            selectable: !has_flag(flags, "\\Noselect"),
        });
    }
    mailboxes
}

fn has_flag(flags: &str, wanted: &str) -> bool {
    flags
        .split_ascii_whitespace()
        .any(|flag| flag.eq_ignore_ascii_case(wanted))
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

pub(crate) fn parse_search(responses: &[Response]) -> Vec<u32> {
    responses
        .iter()
        .filter_map(|response| {
            let line = std::str::from_utf8(response.first_line()).ok()?;
            line.strip_prefix("* SEARCH")
        })
        .flat_map(str::split_ascii_whitespace)
        .filter_map(|uid| uid.parse::<u32>().ok().filter(|uid| *uid != 0))
        .collect()
}

pub(crate) fn parse_copyuid(response: &Response) -> Option<CopyUid> {
    let line = String::from_utf8_lossy(response.first_line());
    let (_, rest) = line.split_once("[COPYUID ")?;
    let (values, _) = rest.split_once(']')?;
    let mut values = values.split_ascii_whitespace();
    let uid_validity = values.next()?.parse().ok()?;
    let source_uids = values.next()?.to_string();
    let destination_uids = values.next()?.to_string();
    if values.next().is_some()
        || validate_uid_set(&source_uids).is_err()
        || validate_uid_set(&destination_uids).is_err()
    {
        return None;
    }
    Some(CopyUid {
        uid_validity,
        source_uids,
        destination_uids,
    })
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

fn read_token(input: &str) -> Option<(String, &str)> {
    let input = input.trim_start();
    if let Some(rest) = input.strip_prefix('"') {
        let mut value = String::new();
        let mut escaped = false;
        for (index, ch) in rest.char_indices() {
            if escaped {
                value.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return Some((value, &rest[index + 1..]));
            } else {
                value.push(ch);
            }
        }
        None
    } else {
        let end = input.find(' ').unwrap_or(input.len());
        Some((input[..end].to_string(), &input[end..]))
    }
}

pub(crate) fn quote(value: &str) -> Result<String, Error> {
    let value = encode_mailbox(value)?;
    quote_string(&value)
}

pub(crate) fn quote_string(value: &str) -> Result<String, Error> {
    if value.chars().any(char::is_control) {
        return Err(Error::Protocol("Invalid IMAP quoted string"));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn encode_mailbox(value: &str) -> Result<String, Error> {
    if value.chars().any(char::is_control) {
        return Err(Error::Protocol("Invalid IMAP mailbox name"));
    }
    let mut output = String::new();
    let mut unicode = Vec::new();
    for ch in value.chars() {
        if ch.is_ascii() {
            flush_unicode(&mut output, &mut unicode);
            if ch == '&' {
                output.push_str("&-");
            } else {
                output.push(ch);
            }
        } else {
            let mut units = [0u16; 2];
            for unit in ch.encode_utf16(&mut units).iter() {
                unicode.extend_from_slice(&unit.to_be_bytes());
            }
        }
    }
    flush_unicode(&mut output, &mut unicode);
    Ok(output)
}

fn flush_unicode(output: &mut String, unicode: &mut Vec<u8>) {
    if !unicode.is_empty() {
        output.push('&');
        output.push_str(&STANDARD_NO_PAD.encode(&*unicode).replace('/', ","));
        output.push('-');
        unicode.clear();
    }
}

fn decode_mailbox(value: &str) -> Option<String> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        let encoded = &rest[start + 1..];
        let end = encoded.find('-')?;
        if end == 0 {
            output.push('&');
        } else {
            let bytes = STANDARD_NO_PAD
                .decode(encoded[..end].replace(',', "/"))
                .ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            output.push_str(&String::from_utf16(&units).ok()?);
        }
        rest = &encoded[end + 1..];
    }
    output.push_str(rest);
    Some(output)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_names_round_trip_modified_utf7() {
        let name = "旅行 & Work";
        let encoded = encode_mailbox(name).unwrap();
        assert_eq!(decode_mailbox(&encoded), Some(name.into()));
        assert!(encoded.contains("&-"));
        assert_eq!(quote(name).unwrap(), format!("\"{encoded}\""));
        assert_eq!(quote_string("A&B").unwrap(), "\"A&B\"");
    }

    #[test]
    fn list_parses_escaped_and_literal_mailbox_names() {
        let quoted = Response::new(
            b"* LIST (\\sent) \"/\" \"Sent \\\"Work\\\"\"\r\n".to_vec(),
            None,
        )
        .unwrap();
        let name = encode_mailbox("旅行").unwrap();
        let raw = format!(
            "* LIST (\\Noselect) \"/\" {{{}}}\r\n{}\r\n",
            name.len(),
            name
        )
        .into_bytes();
        let literal = Response::new(raw, Some(name.into_bytes())).unwrap();
        let mailboxes = parse_list(&[quoted, literal]);
        assert_eq!(mailboxes.len(), 2);
        assert_eq!(mailboxes[0].name, "Sent \"Work\"");
        assert_eq!(mailboxes[0].kind, MailboxKind::Sent);
        assert_eq!(mailboxes[1].name, "旅行");
        assert!(!mailboxes[1].selectable);
    }
}
