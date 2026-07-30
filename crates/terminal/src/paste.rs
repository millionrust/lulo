pub(crate) const MAX_BYTES: usize = 1024 * 1024;

const BRACKETED_START: &[u8] = b"\x1b[200~";
const BRACKETED_END: &[u8] = b"\x1b[201~";

pub(crate) fn logical_line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut lines = 1;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                lines += 1;
                index += 2;
            }
            b'\r' | b'\n' => {
                lines += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    lines
}

pub(crate) fn has_unsafe_unbracketed_control(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\r' | '\n'))
}

pub(crate) fn prepare(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = Vec::with_capacity(text.len().saturating_add(12));
        bytes.extend_from_slice(BRACKETED_START);
        bytes.extend(
            text.as_bytes()
                .iter()
                .copied()
                .filter(|byte| !matches!(byte, b'\x1b' | b'\x03')),
        );
        bytes.extend_from_slice(BRACKETED_END);
        bytes
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

pub(crate) struct PendingPaste {
    pub(crate) session_id: u64,
    pub(crate) text: String,
    pub(crate) line_count: usize,
    pub(crate) byte_count: usize,
}

impl PendingPaste {
    pub(crate) fn new(session_id: u64, text: String) -> Self {
        Self {
            session_id,
            line_count: logical_line_count(&text),
            byte_count: text.len(),
            text,
        }
    }
}

impl std::fmt::Debug for PendingPaste {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingPaste")
            .field("session_id", &self.session_id)
            .field("text", &"<private>")
            .field("line_count", &self.line_count)
            .field("byte_count", &self.byte_count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        has_unsafe_unbracketed_control, logical_line_count, prepare, PendingPaste, BRACKETED_END,
        BRACKETED_START, MAX_BYTES,
    };

    #[test]
    fn line_count_normalizes_platform_boundaries() {
        assert_eq!(MAX_BYTES, 1024 * 1024);
        assert_eq!(logical_line_count(""), 0);
        assert_eq!(logical_line_count("one"), 1);
        assert_eq!(logical_line_count("one\ntwo"), 2);
        assert_eq!(logical_line_count("one\r\ntwo"), 2);
        assert_eq!(logical_line_count("one\rtwo"), 2);
        assert_eq!(logical_line_count("one\r\ntwo\nthree\rfour"), 4);
    }

    #[test]
    fn bracketed_payload_cannot_embed_its_terminator() {
        let payload = prepare("one\x1b[201~two\x03\nthree", true);
        let mut expected = BRACKETED_START.to_vec();
        expected.extend_from_slice(b"one[201~two\nthree");
        expected.extend_from_slice(BRACKETED_END);

        assert_eq!(payload, expected);
        assert_eq!(
            payload
                .windows(BRACKETED_END.len())
                .filter(|window| *window == BRACKETED_END)
                .count(),
            1
        );
    }

    #[test]
    fn unbracketed_payload_uses_return_and_rejects_controls() {
        assert_eq!(
            prepare("one\r\ntwo\nthree\rfour", false),
            b"one\rtwo\rthree\rfour"
        );
        assert!(has_unsafe_unbracketed_control("one\x1btwo"));
        assert!(has_unsafe_unbracketed_control("one\x03two"));
        assert!(!has_unsafe_unbracketed_control("one\ttwo\nthree"));
    }

    #[test]
    fn pending_review_debug_redacts_clipboard_text() {
        let pending = PendingPaste::new(7, "private clipboard body".into());
        let debug = format!("{pending:?}");

        assert_eq!(pending.line_count, 1);
        assert_eq!(pending.byte_count, 22);
        assert!(!debug.contains("private clipboard body"));
        assert!(debug.contains("<private>"));
        assert!(debug.contains("line_count: 1"));
    }
}
