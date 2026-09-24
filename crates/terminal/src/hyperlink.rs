//! Bounded, non-spoofing admission policy for terminal hyperlink activation.

use std::fmt;
use url::{Host, Url};

pub(crate) const MAX_URI_BYTES: usize = 768;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkTarget {
    uri: String,
    preview: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkRejection {
    TooLong,
    ControlCharacter,
    Invalid,
    UnsupportedScheme,
    MissingHost,
    Credentials,
}

impl LinkTarget {
    pub(crate) fn parse(raw: &str) -> Result<Self, LinkRejection> {
        if raw.len() > MAX_URI_BYTES {
            return Err(LinkRejection::TooLong);
        }
        if raw.chars().any(is_control_or_directional) {
            return Err(LinkRejection::ControlCharacter);
        }

        let url = Url::parse(raw).map_err(|_| LinkRejection::Invalid)?;
        let preview = match url.scheme() {
            "http" | "https" => {
                if !url.username().is_empty() || url.password().is_some() {
                    return Err(LinkRejection::Credentials);
                }
                let host = url.host().ok_or(LinkRejection::MissingHost)?;
                http_preview(url.scheme(), host, url.port())
            }
            "mailto" => "email message".into(),
            _ => return Err(LinkRejection::UnsupportedScheme),
        };

        Ok(Self {
            uri: raw.to_owned(),
            preview,
        })
    }

    pub(crate) fn uri(&self) -> &str {
        &self.uri
    }

    pub(crate) fn preview(&self) -> &str {
        &self.preview
    }
}

impl fmt::Display for LinkRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Terminal refused an unsupported or unsafe link address.")
    }
}

fn http_preview(scheme: &str, host: Host<&str>, port: Option<u16>) -> String {
    let host = match host {
        Host::Ipv6(address) => format!("[{address}]"),
        Host::Domain(domain) => domain.to_owned(),
        Host::Ipv4(address) => address.to_string(),
    };
    match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

const URL_WRAPPERS: [char; 6] = ['(', '[', '{', '<', '"', '\''];
const URL_TRAILERS: [char; 12] = ['.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '>'];

/// The `http`/`https` URL under `column` in a plain line of text, if any —
/// for programs that print a link as bare text instead of an OSC 8
/// hyperlink. A run of non-whitespace characters starting with `http://` or
/// `https://` is a candidate; punctuation a sentence would wrap it in
/// (`(…)`, `"…"`, a trailing `.` or `,`, …) is trimmed from both ends.
pub(crate) fn find_url(line: &str, column: usize) -> Option<&str> {
    let mut start = 0;
    for word in line.split(' ') {
        let word_start = start;
        start += word.chars().count() + 1;
        let leading = word
            .chars()
            .take_while(|c| URL_WRAPPERS.contains(c))
            .count();
        let candidate = &word[leading..];
        if !(candidate.starts_with("http://") || candidate.starts_with("https://")) {
            continue;
        }
        let trimmed = candidate.trim_end_matches(URL_TRAILERS.as_slice());
        if trimmed.is_empty() {
            continue;
        }
        let trimmed_start = word_start + leading;
        let trimmed_end = trimmed_start + trimmed.chars().count();
        if column >= trimmed_start && column < trimmed_end {
            return Some(trimmed);
        }
    }
    None
}

fn is_control_or_directional(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_web_and_email_links_with_private_previews() {
        let web =
            LinkTarget::parse("https://example.test/private/account?token=secret#recovery-code")
                .unwrap();
        assert_eq!(web.preview(), "https://example.test");
        assert!(!web.preview().contains("private"));
        assert!(!web.preview().contains("secret"));
        assert_eq!(
            web.uri(),
            "https://example.test/private/account?token=secret#recovery-code"
        );

        let email = LinkTarget::parse("mailto:private@example.test?subject=Secret").unwrap();
        assert_eq!(email.preview(), "email message");
    }

    #[test]
    fn rejects_credentialed_local_and_executable_schemes() {
        assert_eq!(
            LinkTarget::parse("https://user:password@example.test"),
            Err(LinkRejection::Credentials)
        );
        assert_eq!(
            LinkTarget::parse("file:///home/user/private.txt"),
            Err(LinkRejection::UnsupportedScheme)
        );
        assert_eq!(
            LinkTarget::parse("javascript:alert(1)"),
            Err(LinkRejection::UnsupportedScheme)
        );
    }

    #[test]
    fn rejects_missing_hosts_controls_directional_spoofing_and_oversize() {
        assert_eq!(LinkTarget::parse("https://"), Err(LinkRejection::Invalid));
        assert_eq!(
            LinkTarget::parse("https://example.test/\nnext"),
            Err(LinkRejection::ControlCharacter)
        );
        assert_eq!(
            LinkTarget::parse("https://example.test/\u{202e}txt.exe"),
            Err(LinkRejection::ControlCharacter)
        );
        assert_eq!(
            LinkTarget::parse(&format!(
                "https://example.test/{}",
                "a".repeat(MAX_URI_BYTES)
            )),
            Err(LinkRejection::TooLong)
        );
    }

    #[test]
    fn finds_plain_urls_and_trims_sentence_punctuation() {
        let line = "see https://example.test/docs, then continue.";
        assert_eq!(find_url(line, 4), Some("https://example.test/docs"));
        assert_eq!(find_url(line, 29), Some("https://example.test/docs"));
        assert_eq!(find_url(line, 0), None);
        assert_eq!(find_url(line, 40), None);
        assert_eq!(
            find_url("(http://a.test)", 1),
            Some("http://a.test"),
            "wrapping parentheses trim from both ends"
        );
        assert_eq!(find_url("no links here", 3), None);
    }

    #[test]
    fn formats_ports_and_ipv6_without_path_or_query_data() {
        assert_eq!(
            LinkTarget::parse("http://example.test:8080/a?b=c")
                .unwrap()
                .preview(),
            "http://example.test:8080"
        );
        assert_eq!(
            LinkTarget::parse("https://[::1]/private")
                .unwrap()
                .preview(),
            "https://[::1]"
        );
    }
}
