//! MIME conversion for Mail's worker threads. HTML is reduced to inert data;
//! the GPUI viewer renders `RichText` and never receives executable markup.

mod rich_text;
pub use rich_text::{sanitize_html, Block, BlockKind, Image, RichText, Span};

use mail_builder::MessageBuilder;
use mail_parser::{MessageParser, MimeHeaders};

#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Bcc is only used for the SMTP envelope; never put it in the MIME source.
    pub bcc: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    /// Present for a CID image referenced by the HTML body.
    pub content_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltMessage {
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub enum Error {
    InvalidAddress,
    InvalidMessageId,
    InvalidContentType,
    EmptyRecipients,
    Build(std::io::Error),
    Parse,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAddress => write!(f, "invalid email address"),
            Self::InvalidMessageId => write!(f, "invalid message ID"),
            Self::InvalidContentType => write!(f, "invalid attachment content type"),
            Self::EmptyRecipients => write!(f, "message needs a recipient"),
            Self::Build(error) => write!(f, "MIME build failed: {error}"),
            Self::Parse => write!(f, "message could not be parsed"),
        }
    }
}
impl std::error::Error for Error {}

/// Validate envelope addresses before either MIME creation or SMTP submission.
/// The app's address completion handles display names separately.
pub fn valid_address(address: &str) -> bool {
    !address.is_empty()
        && address.is_ascii()
        && address.matches('@').count() == 1
        && address.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !address
            .bytes()
            .any(|b| b <= 32 || b == 127 || b == b'<' || b == b'>' || b == b',' || b == b';')
}

fn normalise_id(id: &str) -> Option<&str> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>');
    (id.is_ascii()
        && id.contains('@')
        && !id
            .bytes()
            .any(|byte| byte <= 32 || byte == 127 || matches!(byte, b'<' | b'>')))
    .then_some(id)
}

fn normalise_content_id(id: &str) -> Option<&str> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>');
    (!id.is_empty()
        && id.is_ascii()
        && !id
            .bytes()
            .any(|byte| byte <= 32 || byte == 127 || matches!(byte, b'<' | b'>')))
    .then_some(id)
}

fn valid_content_type(value: &str) -> bool {
    let Some((type_, subtype)) = value.split_once('/') else {
        return false;
    };
    !type_.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                )
        })
}

pub fn build(draft: &Draft) -> Result<BuiltMessage, Error> {
    let recipients: Vec<String> = draft
        .to
        .iter()
        .chain(&draft.cc)
        .chain(&draft.bcc)
        .cloned()
        .collect();
    if recipients.is_empty() {
        return Err(Error::EmptyRecipients);
    }
    if !valid_address(&draft.from) || !recipients.iter().all(|address| valid_address(address)) {
        return Err(Error::InvalidAddress);
    }
    let mut builder = MessageBuilder::new()
        .from(draft.from.as_str())
        .subject(draft.subject.as_str())
        .text_body(draft.text.as_str());
    if !draft.to.is_empty() {
        builder = builder.to(draft.to.iter().map(String::as_str).collect::<Vec<_>>());
    }
    if !draft.cc.is_empty() {
        builder = builder.cc(draft.cc.iter().map(String::as_str).collect::<Vec<_>>());
    }
    if let Some(html) = &draft.html {
        builder = builder.html_body(html.as_str());
    }
    if let Some(id) = &draft.in_reply_to {
        builder = builder.in_reply_to(normalise_id(id).ok_or(Error::InvalidMessageId)?.to_owned());
    }
    if !draft.references.is_empty() {
        let ids = draft
            .references
            .iter()
            .map(|id| {
                normalise_id(id)
                    .map(str::to_owned)
                    .ok_or(Error::InvalidMessageId)
            })
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.references(ids);
    }
    for attachment in &draft.attachments {
        if !valid_content_type(&attachment.content_type) {
            return Err(Error::InvalidContentType);
        }
        builder = if let Some(id) = &attachment.content_id {
            builder.inline(
                attachment.content_type.as_str(),
                normalise_content_id(id).ok_or(Error::InvalidMessageId)?,
                attachment.bytes.as_slice(),
            )
        } else {
            builder.attachment(
                attachment.content_type.as_str(),
                attachment.filename.as_str(),
                attachment.bytes.as_slice(),
            )
        };
    }
    let bytes = builder.write_to_vec().map_err(Error::Build)?;
    Ok(BuiltMessage {
        envelope_from: draft.from.clone(),
        envelope_to: recipients,
        bytes,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMessage {
    pub subject: String,
    pub message_id: Option<String>,
    pub plain_text: String,
    pub rich_text: RichText,
    pub attachments: Vec<Attachment>,
}

pub fn parse(bytes: &[u8]) -> Result<ParsedMessage, Error> {
    let message = MessageParser::default().parse(bytes).ok_or(Error::Parse)?;
    let plain_text = message
        .body_text(0)
        .map(|body| body.into_owned())
        .unwrap_or_default();
    let rich_text = match message.body_html(0) {
        Some(html) => sanitize_html(&html),
        None => RichText::from_plain(&plain_text),
    };
    let mut attachments = Vec::new();
    for index in 0..message.attachment_count() {
        let Some(part) = message.attachment(index as u32) else {
            continue;
        };
        attachments.push(Attachment {
            filename: part.attachment_name().unwrap_or("attachment").to_owned(),
            content_type: part
                .content_type()
                .map(|value| {
                    format!(
                        "{}/{}",
                        value.c_type,
                        value.c_subtype.as_deref().unwrap_or("octet-stream")
                    )
                })
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            bytes: part.contents().to_vec(),
            content_id: part
                .content_id()
                .map(|id| id.trim_start_matches('<').trim_end_matches('>').to_owned()),
        });
    }
    Ok(ParsedMessage {
        subject: message.subject().unwrap_or_default().to_owned(),
        message_id: message.message_id().map(str::to_owned),
        plain_text,
        rich_text,
        attachments,
    })
}

/// Textual reply/forward output for the compose editor. The date and sender are
/// caller supplied so locale and Mac wording can be adjusted without parsing a
/// display name from an address or trusting a message's Date header.
pub fn quote_reply(date: &str, sender: &str, body: &str) -> String {
    let mut quoted = format!("On {date}, {sender} wrote:\n");
    for line in body.lines() {
        quoted.push_str("> ");
        quoted.push_str(line);
        quoted.push('\n');
    }
    quoted
}

pub fn quote_forward(date: &str, sender: &str, to: &str, subject: &str, body: &str) -> String {
    format!("Begin forwarded message:\n\nFrom: {sender}\nDate: {date}\nTo: {to}\nSubject: {subject}\n\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_parse_multipart_and_bcc_envelope() {
        let draft = Draft {
            from: "a@example.test".into(),
            to: vec!["b@example.test".into()],
            bcc: vec!["hidden@example.test".into()],
            subject: "Café".into(),
            text: "Plain café".into(),
            html: Some("<p>HTML café</p>".into()),
            attachments: vec![Attachment {
                filename: "file.txt".into(),
                content_type: "text/plain".into(),
                bytes: b"attached".to_vec(),
                content_id: None,
            }],
            ..Draft::default()
        };
        let built = build(&draft).unwrap();
        assert_eq!(built.envelope_to.len(), 2);
        assert!(!String::from_utf8_lossy(&built.bytes).contains("hidden@example.test"));
        let parsed = parse(&built.bytes).unwrap();
        assert_eq!(parsed.subject, "Café");
        assert!(parsed.plain_text.contains("Plain café"));
        assert_eq!(parsed.attachments[0].bytes, b"attached");
    }

    #[test]
    fn rejects_header_injection_and_empty_recipients() {
        let mut draft = Draft {
            from: "a@example.test".into(),
            to: vec!["b@example.test".into()],
            ..Draft::default()
        };
        draft.to[0] = "x@example.test\r\nBcc: y@example.test".into();
        assert!(matches!(build(&draft), Err(Error::InvalidAddress)));
        draft.to.clear();
        assert!(matches!(build(&draft), Err(Error::EmptyRecipients)));
        draft.to.push("b@example.test".into());
        draft.attachments.push(Attachment {
            filename: "x".into(),
            content_type: "text/plain\r\nBcc: thief@example.test".into(),
            bytes: vec![],
            content_id: None,
        });
        assert!(matches!(build(&draft), Err(Error::InvalidContentType)));
    }

    #[test]
    fn references_are_normalised_and_cannot_inject_headers() {
        let mut draft = Draft {
            from: "a@example.test".into(),
            to: vec!["b@example.test".into()],
            in_reply_to: Some("<original@example.test>".into()),
            references: vec![
                "<older@example.test>".into(),
                "<original@example.test>".into(),
            ],
            ..Draft::default()
        };
        let built = build(&draft).unwrap();
        let source = String::from_utf8_lossy(&built.bytes);
        assert!(source.contains("In-Reply-To: <original@example.test>"));
        assert!(source.contains("References: <older@example.test> <original@example.test>"));
        draft
            .references
            .push("bad@example.test\r\nBcc: thief@example.test".into());
        assert!(matches!(build(&draft), Err(Error::InvalidMessageId)));
    }

    #[test]
    fn cid_images_keep_an_attachment_mapping() {
        let draft = Draft {
            from: "a@example.test".into(),
            to: vec!["b@example.test".into()],
            html: Some("<p><img src='cid:photo@example.test' alt='Photo'></p>".into()),
            attachments: vec![Attachment {
                filename: "photo.png".into(),
                content_type: "image/png".into(),
                bytes: vec![137, 80, 78, 71],
                content_id: Some("photo@example.test".into()),
            }],
            ..Draft::default()
        };
        let parsed = parse(&build(&draft).unwrap().bytes).unwrap();
        assert_eq!(
            parsed.rich_text.inline_images[0].content_id,
            "photo@example.test"
        );
        assert!(parsed
            .attachments
            .iter()
            .any(
                |part| part.content_id.as_deref() == Some("photo@example.test")
                    && part.bytes == vec![137, 80, 78, 71]
            ));
    }

    #[test]
    fn quoting_goldens() {
        assert_eq!(
            quote_reply("3 Oct 2026, at 09:41", "Anna Kim", "Hi\n\nBye"),
            "On 3 Oct 2026, at 09:41, Anna Kim wrote:\n> Hi\n> \n> Bye\n"
        );
        assert_eq!(quote_forward("3 Oct 2026", "Anna Kim", "Jacob", "Lunch", "See you"), "Begin forwarded message:\n\nFrom: Anna Kim\nDate: 3 Oct 2026\nTo: Jacob\nSubject: Lunch\n\nSee you");
    }
}
