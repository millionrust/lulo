//! Bounded, redacted text prepared for lock-screen presentation.

use std::fmt;

use zeroize::Zeroize as _;

use crate::pam_broker::PromptId;
use crate::pam_conversation::RequestKind;

pub(crate) const MAX_PROMPT_LABEL_BYTES: usize = 256;

#[derive(Clone, Copy)]
pub(crate) struct PromptText<'a> {
    id: PromptId,
    kind: RequestKind,
    bytes: Option<&'a [u8]>,
}

impl<'a> PromptText<'a> {
    pub(crate) fn new(id: PromptId, kind: RequestKind, bytes: Option<&'a [u8]>) -> Self {
        Self { id, kind, bytes }
    }

    pub(crate) fn key(self) -> PromptKey {
        PromptKey::Pam(self.id)
    }
}

impl fmt::Debug for PromptText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PromptText(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKey {
    Pam(PromptId),
    AuthenticationFailure,
}

pub(crate) struct PromptLabel {
    key: PromptKey,
    value: String,
}

impl PromptLabel {
    pub(crate) fn from_prompt(prompt: PromptText<'_>) -> Self {
        let fallback = fallback_label(prompt.kind);
        let value = prompt
            .bytes
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(normalize)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback.to_owned());
        Self {
            key: PromptKey::Pam(prompt.id),
            value,
        }
    }

    pub(crate) fn authentication_failure() -> Self {
        Self {
            key: PromptKey::AuthenticationFailure,
            value: "Authentication failed. Try again.".to_owned(),
        }
    }

    pub(crate) fn key(&self) -> PromptKey {
        self.key
    }

    pub(crate) fn expose<R>(&self, use_text: impl FnOnce(&str) -> R) -> R {
        use_text(&self.value)
    }
}

impl fmt::Debug for PromptLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PromptLabel")
            .field("key", &"<redacted>")
            .field("value", &"<redacted>")
            .finish()
    }
}

impl Drop for PromptLabel {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len().min(MAX_PROMPT_LABEL_BYTES));
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() || character.is_control() {
            pending_space |= !normalized.is_empty();
            continue;
        }
        if is_bidi_control(character) {
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(character);
    }
    if normalized.len() > MAX_PROMPT_LABEL_BYTES {
        let mut end = MAX_PROMPT_LABEL_BYTES - '…'.len_utf8();
        while !normalized.is_char_boundary(end) {
            end -= 1;
        }
        normalized.truncate(end);
        normalized.push('…');
    }
    normalized
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn fallback_label(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::EchoOff => "Password",
        RequestKind::EchoOn => "Account information",
        RequestKind::Info => "Authentication message",
        RequestKind::Error => "Authentication error",
        RequestKind::Radio => "Choose an authentication option",
        RequestKind::Binary => "Additional authentication required",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pam_broker::conversation_channel;
    use crate::pam_conversation::{Conversation as _, Request};
    use std::thread;
    use std::time::Duration;

    fn pending_prompt(request: Request<'static>) -> crate::pam_broker::PendingPrompt {
        let (mut conversation, ui) = conversation_channel();
        thread::spawn(move || conversation.respond(request));
        ui.prompt_timeout(Duration::from_secs(2)).unwrap().unwrap()
    }

    #[test]
    fn preserves_unicode_but_normalizes_controls_and_bidi_overrides() {
        let pending = pending_prompt(Request::EchoOff(c"  Пароль:\n\u{202e} رمز  "));
        let prompt_value = pending.prompt();
        prompt_value.text(|text| {
            let presentation = PromptText::new(prompt_value.id(), prompt_value.kind(), text);
            assert_eq!(presentation.key(), PromptKey::Pam(prompt_value.id()));
            let label = PromptLabel::from_prompt(presentation);
            label.expose(|value| assert_eq!(value, "Пароль: رمز"));
            let debug = format!("{label:?}");
            assert!(!debug.contains("Пароль"));
            assert!(!debug.contains("رمز"));
        });
    }

    #[test]
    fn invalid_empty_and_oversized_prompts_get_safe_bounded_labels() {
        let pending = pending_prompt(Request::EchoOn(c"Account:"));
        let prompt_value = pending.prompt();
        let label = PromptLabel::from_prompt(PromptText::new(
            prompt_value.id(),
            prompt_value.kind(),
            Some(c"\xff".to_bytes()),
        ));
        label.expose(|value| assert_eq!(value, "Account information"));

        let pending = pending_prompt(Request::Info(c"   "));
        let prompt_value = pending.prompt();
        prompt_value.text(|text| {
            let label = PromptLabel::from_prompt(PromptText::new(
                prompt_value.id(),
                prompt_value.kind(),
                text,
            ));
            label.expose(|value| assert_eq!(value, "Authentication message"));
        });

        let value = normalize(&"é".repeat(MAX_PROMPT_LABEL_BYTES));
        assert!(value.len() <= MAX_PROMPT_LABEL_BYTES);
        assert!(value.ends_with('…'));

        let failure = PromptLabel::authentication_failure();
        assert_eq!(failure.key(), PromptKey::AuthenticationFailure);
        failure.expose(|value| assert!(value.starts_with("Authentication failed")));
    }
}
