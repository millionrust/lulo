//! `mailto:` URI parsing (RFC 6068) into a `rmac_mail_mime::Draft` (MAIL-8).
//! The desktop and MIME registration live in `packaging/rmac-apps`; the
//! session hands the URI to `rmac-mail` as its one command-line argument
//! (`Exec=/usr/bin/rmac-mail %u`), and `main.rs` calls [`parse`] before
//! opening a compose window prefilled with the result.

use rmac_mail_mime::Draft;

/// Parses a `mailto:` URI into a compose draft. Returns `None` for anything
/// that is not a `mailto:` URI, or one with neither a recipient, a subject
/// nor a body (nothing worth opening a compose window for).
pub fn parse(uri: &str) -> Option<Draft> {
    let rest = uri.strip_prefix("mailto:")?;
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let mut draft = Draft {
        to: split_addresses(path),
        ..Draft::default()
    };
    if let Some(query) = query {
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = percent_decode(value);
            match key.to_ascii_lowercase().as_str() {
                "to" => draft.to.extend(split_addresses_decoded(&value)),
                "cc" => draft.cc.extend(split_addresses_decoded(&value)),
                "bcc" => draft.bcc.extend(split_addresses_decoded(&value)),
                "subject" => draft.subject = value,
                "body" => draft.text = value,
                _ => {}
            }
        }
    }
    draft.to.retain(|address| !address.is_empty());
    (!draft.to.is_empty() || !draft.subject.is_empty() || !draft.text.is_empty()).then_some(draft)
}

fn split_addresses(raw: &str) -> Vec<String> {
    split_addresses_decoded(&percent_decode(raw))
}

fn split_addresses_decoded(decoded: &str) -> Vec<String> {
    decoded
        .split(',')
        .map(|address| address.trim().to_owned())
        .filter(|address| !address.is_empty())
        .collect()
}

/// RFC 3986 percent-decoding into UTF-8, byte-accurate so a multi-byte
/// sequence like `%C3%A9` decodes to `é` rather than two mangled chars. `+`
/// stays literal: RFC 6068 does not give it `application/x-www-form`'s
/// space meaning.
fn percent_decode(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let mut chars = value.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = chars.next().and_then(hex_value);
            let low = chars.next().and_then(hex_value);
            if let (Some(high), Some(low)) = (high, low) {
                bytes.push(high * 16 + low);
                continue;
            }
            bytes.push(byte);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_address_becomes_a_single_recipient() {
        let draft = parse("mailto:anna@example.com").unwrap();
        assert_eq!(draft.to, vec!["anna@example.com".to_owned()]);
        assert!(draft.subject.is_empty());
        assert!(draft.text.is_empty());
    }

    #[test]
    fn query_fields_fill_cc_subject_and_body() {
        let draft = parse(
            "mailto:anna@example.com?cc=sam@example.com&subject=Lunch&body=See%20you%20at%2012",
        )
        .unwrap();
        assert_eq!(draft.to, vec!["anna@example.com".to_owned()]);
        assert_eq!(draft.cc, vec!["sam@example.com".to_owned()]);
        assert_eq!(draft.subject, "Lunch");
        assert_eq!(draft.text, "See you at 12");
    }

    #[test]
    fn to_only_in_the_query_still_recipients() {
        let draft = parse("mailto:?to=anna@example.com,sam@example.com&subject=Hi").unwrap();
        assert_eq!(
            draft.to,
            vec!["anna@example.com".to_owned(), "sam@example.com".to_owned()]
        );
    }

    #[test]
    fn percent_encoded_utf8_decodes_as_one_character() {
        let draft = parse("mailto:?subject=Caf%C3%A9&to=a@example.com").unwrap();
        assert_eq!(draft.subject, "Café");
    }

    #[test]
    fn newline_escape_in_body_becomes_a_real_newline() {
        let draft = parse("mailto:?to=a@example.com&body=Hello%0AWorld").unwrap();
        assert_eq!(draft.text, "Hello\nWorld");
    }

    #[test]
    fn non_mailto_uris_and_empty_ones_are_rejected() {
        assert!(parse("https://example.com").is_none());
        assert!(parse("mailto:").is_none());
    }

    #[test]
    fn cc_only_with_no_recipient_subject_or_body_is_rejected() {
        assert!(parse("mailto:?cc=x@example.com").is_none());
    }
}
