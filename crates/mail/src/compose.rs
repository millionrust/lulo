//! Compose state shared by the window and worker. No disk or network I/O here.

use rmac_mail_mime::{quote_forward, quote_reply, valid_address, Draft};

use crate::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: String,
    pub address: String,
}

impl Recipient {
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            self.address.clone()
        } else {
            format!("{} <{}>", self.name, self.address)
        }
    }
}

/// Accept comma/semicolon separated mailboxes, including `Name <address>`.
/// The MIME builder validates the envelope again before queueing.
pub fn addresses(input: &str) -> Result<Vec<String>, &'static str> {
    let mut result = Vec::new();
    for token in input.split([',', ';']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let address = if let Some((_, rest)) = token.rsplit_once('<') {
            rest.strip_suffix('>')
                .ok_or("Check the recipient address")?
                .trim()
        } else {
            token
        };
        if !valid_address(address) {
            return Err("Check the recipient address");
        }
        if !result
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(address))
        {
            result.push(address.to_owned());
        }
    }
    Ok(result)
}

pub fn complete(query: &str, candidates: &[Recipient]) -> Vec<Recipient> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    candidates
        .iter()
        .filter(|candidate| {
            candidate.name.to_lowercase().contains(&query)
                || candidate.address.to_lowercase().contains(&query)
        })
        .take(8)
        .cloned()
        .collect()
}

pub fn initial_draft(kind: ComposeKind, message: Option<&Message>, from: &str) -> Draft {
    let mut draft = Draft {
        from: from.to_owned(),
        ..Draft::default()
    };
    let Some(message) = message else {
        return draft;
    };
    let subject = message.subject.trim();
    match kind {
        ComposeKind::New => {}
        ComposeKind::Reply | ComposeKind::ReplyAll => {
            draft.subject = if subject.to_lowercase().starts_with("re:") {
                subject.to_owned()
            } else {
                format!("Re: {subject}")
            };
            // MAIL-5's fixture has display names, not RFC 5322 addresses.
            // Only an actual address may enter the envelope.
            if valid_address(message.sender_address) {
                draft.to.push(message.sender_address.to_owned());
            }
            if kind == ComposeKind::ReplyAll {
                for address in message
                    .to
                    .split([',', ';'])
                    .chain(message.cc.split([',', ';']))
                {
                    let address = address.trim();
                    if valid_address(address)
                        && !address.eq_ignore_ascii_case(from)
                        && !draft
                            .to
                            .iter()
                            .any(|item| item.eq_ignore_ascii_case(address))
                    {
                        draft.cc.push(address.to_owned());
                    }
                }
            }
            draft.text = format!(
                "\n\n{}",
                quote_reply(message.date, message.sender, &message.body.plain_text())
            );
        }
        ComposeKind::Forward => {
            draft.subject = if subject.to_lowercase().starts_with("fwd:") {
                subject.to_owned()
            } else {
                format!("Fwd: {subject}")
            };
            draft.text = format!(
                "\n\n{}",
                quote_forward(
                    message.date,
                    message.sender,
                    message.to,
                    subject,
                    &message.body.plain_text()
                )
            );
        }
    }
    draft
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MailState;

    #[test]
    fn addresses_validate_and_deduplicate() {
        assert_eq!(
            addresses("Anna <anna@example.test>; anna@example.test, bob@example.test").unwrap(),
            vec!["anna@example.test", "bob@example.test"]
        );
        assert!(addresses("a@example.test\r\nBcc: thief@example.test").is_err());
        assert!(addresses("Anna <missing@example.test").is_err());
    }

    #[test]
    fn completion_matches_name_or_address() {
        let candidates = vec![Recipient {
            name: "Anna Kim".into(),
            address: "anna@example.test".into(),
        }];
        assert_eq!(complete("kim", &candidates).len(), 1);
        assert_eq!(complete("example.test", &candidates).len(), 1);
        assert!(complete("sam", &candidates).is_empty());
    }

    #[test]
    fn reply_and_forward_quote_the_selected_message() {
        let state = MailState::fixture();
        let original = state.selected_message().unwrap();
        let reply = initial_draft(ComposeKind::Reply, Some(original), "jacob@example.test");
        assert_eq!(reply.subject, "Re: Lunch on Friday?");
        assert_eq!(reply.to, vec!["anna@example.test"]);
        let all = initial_draft(ComposeKind::ReplyAll, Some(original), "jacob@example.test");
        assert_eq!(all.cc, vec!["sam@example.test"]);
        assert!(reply.text.contains("Anna Kim wrote:"));
        assert!(reply.text.contains("> Hi Jacob,"));
        let forward = initial_draft(ComposeKind::Forward, Some(original), "jacob@example.test");
        assert_eq!(forward.subject, "Fwd: Lunch on Friday?");
        assert!(forward.text.contains("Begin forwarded message:"));
    }
}
