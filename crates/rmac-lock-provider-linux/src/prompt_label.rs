//! Bounded, redacted text prepared for lock-screen presentation.

use std::fmt;

use zeroize::Zeroize as _;

#[cfg(any(target_os = "linux", test))]
use crate::pam_broker::PromptId;
use crate::pam_conversation::RequestKind;

pub(crate) const MAX_PROMPT_LABEL_BYTES: usize = 256;
pub(crate) const MAX_ACCOUNT_LABEL_BYTES: usize = 128;

pub(crate) struct AccountLabel {
    value: String,
}

impl AccountLabel {
    pub(crate) fn new(value: &str) -> Result<Self, AccountLabelError> {
        if value.is_empty() {
            return Err(AccountLabelError::Empty);
        }
        if value.len() > MAX_ACCOUNT_LABEL_BYTES {
            return Err(AccountLabelError::TooLong);
        }
        let normalized = normalize(value, MAX_ACCOUNT_LABEL_BYTES);
        if normalized.is_empty() {
            return Err(AccountLabelError::Empty);
        }
        if normalized != value {
            return Err(AccountLabelError::Unsafe);
        }
        Ok(Self { value: normalized })
    }

    pub(crate) fn expose<R>(&self, use_text: impl FnOnce(&str) -> R) -> R {
        use_text(&self.value)
    }
}

impl fmt::Debug for AccountLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccountLabel(<redacted>)")
    }
}

impl Drop for AccountLabel {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountLabelError {
    Empty,
    TooLong,
    Unsafe,
}

#[derive(Clone, Copy)]
pub(crate) struct PromptText<'a> {
    #[cfg(any(target_os = "linux", test))]
    id: PromptId,
    kind: RequestKind,
    bytes: Option<&'a [u8]>,
}

impl<'a> PromptText<'a> {
    pub(crate) fn new(
        id: crate::pam_broker::PromptId,
        kind: RequestKind,
        bytes: Option<&'a [u8]>,
    ) -> Self {
        #[cfg(not(any(target_os = "linux", test)))]
        let _ = id;
        Self {
            #[cfg(any(target_os = "linux", test))]
            id,
            kind,
            bytes,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn key(self) -> PromptKey {
        PromptKey::Pam(self.id)
    }

    pub(crate) fn kind(self) -> RequestKind {
        self.kind
    }
}

impl fmt::Debug for PromptText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PromptText(<redacted>)")
    }
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKey {
    Pam(PromptId),
    Authenticating,
    AuthenticationFailure,
}

pub(crate) struct PromptLabel {
    #[cfg(any(target_os = "linux", test))]
    key: PromptKey,
    value: String,
}

impl PromptLabel {
    pub(crate) fn from_prompt(prompt: PromptText<'_>) -> Self {
        let fallback = fallback_label(prompt.kind);
        let value = prompt
            .bytes
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(|value| normalize(value, MAX_PROMPT_LABEL_BYTES))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback.to_owned());
        Self {
            #[cfg(any(target_os = "linux", test))]
            key: PromptKey::Pam(prompt.id),
            value,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn authentication_failure() -> Self {
        Self {
            key: PromptKey::AuthenticationFailure,
            value: "Authentication failed. Try again.".to_owned(),
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn authenticating() -> Self {
        Self {
            key: PromptKey::Authenticating,
            value: "Authenticating…".to_owned(),
        }
    }

    #[cfg(any(target_os = "linux", test))]
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

fn normalize(value: &str, maximum_bytes: usize) -> String {
    let mut normalized = String::with_capacity(value.len().min(maximum_bytes));
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
    if normalized.len() > maximum_bytes {
        let mut end = maximum_bytes - '…'.len_utf8();
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

        let value = normalize(&"é".repeat(MAX_PROMPT_LABEL_BYTES), MAX_PROMPT_LABEL_BYTES);
        assert!(value.len() <= MAX_PROMPT_LABEL_BYTES);
        assert!(value.ends_with('…'));

        let failure = PromptLabel::authentication_failure();
        assert_eq!(failure.key(), PromptKey::AuthenticationFailure);
        failure.expose(|value| assert!(value.starts_with("Authentication failed")));

        let authenticating = PromptLabel::authenticating();
        assert_eq!(authenticating.key(), PromptKey::Authenticating);
        authenticating.expose(|value| assert_eq!(value, "Authenticating…"));
    }

    #[test]
    fn account_identity_is_bounded_sanitized_and_redacted() {
        let account = AccountLabel::new("jacob").unwrap();
        account.expose(|value| assert_eq!(value, "jacob"));
        assert_eq!(format!("{account:?}"), "AccountLabel(<redacted>)");

        let unicode = "界".repeat(MAX_ACCOUNT_LABEL_BYTES / '界'.len_utf8());
        let account = AccountLabel::new(&unicode).unwrap();
        account.expose(|value| assert_eq!(value, unicode));
        assert!(matches!(
            AccountLabel::new(" \n\u{202e} "),
            Err(AccountLabelError::Empty)
        ));
        assert!(matches!(
            AccountLabel::new("jacob\nadmin"),
            Err(AccountLabelError::Unsafe)
        ));
        assert!(matches!(
            AccountLabel::new(&"x".repeat(MAX_ACCOUNT_LABEL_BYTES + 1)),
            Err(AccountLabelError::TooLong)
        ));
    }
}
