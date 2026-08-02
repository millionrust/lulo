//! Bounded semantic keyboard events emitted by the Linux XKB adapter.
//!
//! Raw keycodes and keymaps never reach credential editing. This contract
//! carries only lock-screen actions and a small, drop-zeroized UTF-8 fragment.

use std::fmt;

use zeroize::Zeroize as _;

use crate::pam_broker::{PendingPrompt, Prompt, PromptResponseError};
use crate::pam_conversation::{Reply, RequestKind, TextResponse};
use crate::{SecretInput, MAX_SECRET_BYTES};

/// A single composed key press cannot inject an unbounded string into the lock
/// UI. This comfortably covers Unicode grapheme and compose sequences while
/// keeping each event small.
pub const MAX_DECODED_TEXT_BYTES: usize = 64;

pub enum DecodedKey {
    Text(DecodedText),
    Backspace,
    Submit,
    Cancel,
    SelectPrevious,
    SelectNext,
    ToggleSelection,
}

impl fmt::Debug for DecodedKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Text(_) => "DecodedKey::Text(<redacted>)",
            Self::Backspace => "DecodedKey::Backspace",
            Self::Submit => "DecodedKey::Submit",
            Self::Cancel => "DecodedKey::Cancel",
            Self::SelectPrevious => "DecodedKey::SelectPrevious",
            Self::SelectNext => "DecodedKey::SelectNext",
            Self::ToggleSelection => "DecodedKey::ToggleSelection",
        })
    }
}

pub struct DecodedText {
    value: String,
}

impl DecodedText {
    pub fn new(mut value: String) -> Result<Self, TextError> {
        if value.is_empty() {
            return Err(TextError::Empty);
        }
        if value.len() > MAX_DECODED_TEXT_BYTES {
            value.zeroize();
            return Err(TextError::TooLong);
        }
        if value.chars().any(char::is_control) {
            value.zeroize();
            return Err(TextError::ControlCharacter);
        }
        Ok(Self { value })
    }

    /// Scope decoded plaintext to immediate credential editing. Callers must
    /// not log or retain another copy.
    pub fn expose<R>(&self, use_text: impl FnOnce(&str) -> R) -> R {
        use_text(&self.value)
    }
}

impl fmt::Debug for DecodedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DecodedText(<redacted>)")
    }
}

impl Drop for DecodedText {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextError {
    Empty,
    TooLong,
    ControlCharacter,
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("decoded keyboard text is invalid")
    }
}

impl std::error::Error for TextError {}

/// Credential editor for one live PAM prompt.
///
/// It owns the prompt's single-use response capability and never clones secret
/// input. Binary prompts remain module-specific and cannot be answered by this
/// generic text/radio editor.
pub struct PromptEditor {
    kind: RequestKind,
    pending: Option<PendingPrompt>,
    state: EditorState,
}

impl PromptEditor {
    pub fn new(pending: PendingPrompt) -> Self {
        let kind = pending.prompt().kind();
        let state = match kind {
            RequestKind::EchoOff => EditorState::Secret(SecretInput::new()),
            RequestKind::EchoOn => EditorState::Text(TextInput::new()),
            RequestKind::Info | RequestKind::Error => EditorState::Notice,
            RequestKind::Radio => EditorState::Radio(false),
            RequestKind::Binary => EditorState::Binary,
        };
        Self {
            kind,
            pending: Some(pending),
            state,
        }
    }

    pub fn kind(&self) -> RequestKind {
        self.kind
    }

    pub(crate) fn prompt(&self) -> Option<&Prompt> {
        self.pending.as_ref().map(|pending| pending.prompt())
    }

    pub fn handle(&mut self, key: DecodedKey) -> Result<EditOutcome, EditError> {
        if self.pending.is_none() {
            return Err(EditError::Finished);
        }
        match key {
            DecodedKey::Text(text) => self.insert(text),
            DecodedKey::Backspace => Ok(if self.backspace() {
                EditOutcome::Changed
            } else {
                EditOutcome::Ignored
            }),
            DecodedKey::Submit => self.submit(),
            DecodedKey::Cancel => self.cancel(),
            DecodedKey::SelectPrevious => Ok(self.select(false)),
            DecodedKey::SelectNext => Ok(self.select(true)),
            DecodedKey::ToggleSelection => Ok(self.toggle_selection()),
        }
    }

    pub fn character_count(&self) -> Option<usize> {
        match &self.state {
            EditorState::Secret(input) => Some(input.character_count()),
            EditorState::Text(input) => std::str::from_utf8(&input.bytes)
                .ok()
                .map(|value| value.chars().count()),
            EditorState::Notice
            | EditorState::Radio(_)
            | EditorState::Binary
            | EditorState::Finished => None,
        }
    }

    pub fn radio_selection(&self) -> Option<bool> {
        match &self.state {
            EditorState::Radio(selected) => Some(*selected),
            _ => None,
        }
    }

    fn insert(&mut self, text: DecodedText) -> Result<EditOutcome, EditError> {
        let result = text.expose(|value| match &mut self.state {
            EditorState::Secret(input) => input.push_text(value).map_err(|_| EditError::Full),
            EditorState::Text(input) => {
                if input.bytes.len() + value.len() > MAX_SECRET_BYTES {
                    Err(EditError::Full)
                } else {
                    input.bytes.extend_from_slice(value.as_bytes());
                    Ok(())
                }
            }
            EditorState::Notice | EditorState::Radio(_) => Ok(()),
            EditorState::Binary => Err(EditError::UnsupportedPrompt),
            EditorState::Finished => Err(EditError::Finished),
        });
        result.map(|()| match &self.state {
            EditorState::Secret(_) | EditorState::Text(_) => EditOutcome::Changed,
            _ => EditOutcome::Ignored,
        })
    }

    fn backspace(&mut self) -> bool {
        match &mut self.state {
            EditorState::Secret(input) => input.backspace(),
            EditorState::Text(input) => erase_last_scalar(&mut input.bytes),
            EditorState::Notice
            | EditorState::Radio(_)
            | EditorState::Binary
            | EditorState::Finished => false,
        }
    }

    fn select(&mut self, selected: bool) -> EditOutcome {
        let EditorState::Radio(current) = &mut self.state else {
            return EditOutcome::Ignored;
        };
        let changed = *current != selected;
        *current = selected;
        if changed {
            EditOutcome::Changed
        } else {
            EditOutcome::Ignored
        }
    }

    fn toggle_selection(&mut self) -> EditOutcome {
        let EditorState::Radio(selected) = &mut self.state else {
            return EditOutcome::Ignored;
        };
        *selected = !*selected;
        EditOutcome::Changed
    }

    fn submit(&mut self) -> Result<EditOutcome, EditError> {
        if matches!(self.state, EditorState::Binary) {
            return Err(EditError::UnsupportedPrompt);
        }
        let pending = self.pending.take().ok_or(EditError::Finished)?;
        let state = std::mem::replace(&mut self.state, EditorState::Finished);
        let reply = match state {
            EditorState::Secret(input) => Reply::Secret(input.finish()),
            EditorState::Text(input) => Reply::Text(
                TextResponse::from_owned(input.into_bytes())
                    .map_err(|_| EditError::InvalidResponse)?,
            ),
            EditorState::Notice => Reply::Acknowledged,
            EditorState::Radio(selected) => Reply::Radio(selected),
            EditorState::Binary => return Err(EditError::UnsupportedPrompt),
            EditorState::Finished => return Err(EditError::Finished),
        };
        pending.respond(reply).map_err(EditError::from)?;
        Ok(EditOutcome::Completed)
    }

    fn cancel(&mut self) -> Result<EditOutcome, EditError> {
        let pending = self.pending.take().ok_or(EditError::Finished)?;
        self.state = EditorState::Finished;
        pending.cancel().map_err(EditError::from)?;
        Ok(EditOutcome::Cancelled)
    }
}

impl fmt::Debug for PromptEditor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PromptEditor(<redacted>)")
    }
}

enum EditorState {
    Secret(SecretInput),
    Text(TextInput),
    Notice,
    Radio(bool),
    Binary,
    Finished,
}

struct TextInput {
    bytes: Vec<u8>,
}

impl TextInput {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_SECRET_BYTES),
        }
    }

    fn into_bytes(mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

impl Drop for TextInput {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

fn erase_last_scalar(bytes: &mut Vec<u8>) -> bool {
    let Some(index) = std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| value.char_indices().next_back().map(|(index, _)| index))
    else {
        return false;
    };
    bytes[index..].zeroize();
    bytes.truncate(index);
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditOutcome {
    Changed,
    Ignored,
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditError {
    Full,
    UnsupportedPrompt,
    Finished,
    InvalidResponse,
    WorkerUnavailable,
}

impl From<PromptResponseError> for EditError {
    fn from(error: PromptResponseError) -> Self {
        match error {
            PromptResponseError::InvalidResponse => Self::InvalidResponse,
            PromptResponseError::WorkerUnavailable => Self::WorkerUnavailable,
        }
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lock-screen input was not accepted")
    }
}

impl std::error::Error for EditError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pam_broker::conversation_channel;
    use crate::pam_conversation::{Conversation, ConversationError, Request};
    use std::thread;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(2);

    fn text(value: &str) -> DecodedKey {
        DecodedKey::Text(DecodedText::new(value.to_owned()).unwrap())
    }

    #[test]
    fn accepts_bounded_unicode_without_exposing_it_to_debug() {
        let text = DecodedText::new("é🔒".to_owned()).unwrap();
        text.expose(|value| assert_eq!(value, "é🔒"));
        let key = DecodedKey::Text(text);
        let debug = format!("{key:?}");
        assert_eq!(debug, "DecodedKey::Text(<redacted>)");
        assert!(!debug.contains('é'));
    }

    #[test]
    fn rejects_empty_control_and_oversized_fragments() {
        assert!(matches!(
            DecodedText::new(String::new()),
            Err(TextError::Empty)
        ));
        assert!(matches!(
            DecodedText::new("line\nfeed".to_owned()),
            Err(TextError::ControlCharacter)
        ));
        assert!(matches!(
            DecodedText::new("x".repeat(MAX_DECODED_TEXT_BYTES + 1)),
            Err(TextError::TooLong)
        ));
    }

    #[test]
    fn action_diagnostics_contain_no_raw_key_identity() {
        assert_eq!(
            format!("{:?}", DecodedKey::Backspace),
            "DecodedKey::Backspace"
        );
        assert_eq!(format!("{:?}", DecodedKey::Submit), "DecodedKey::Submit");
        assert_eq!(format!("{:?}", DecodedKey::Cancel), "DecodedKey::Cancel");
    }

    #[test]
    fn edits_and_moves_a_unicode_secret_to_pam() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let mut editor = PromptEditor::new(pending);

        assert_eq!(editor.handle(text("sé🔒")).unwrap(), EditOutcome::Changed);
        assert_eq!(editor.character_count(), Some(3));
        assert_eq!(
            editor.handle(DecodedKey::Backspace).unwrap(),
            EditOutcome::Changed
        );
        assert_eq!(editor.handle(text("cure")).unwrap(), EditOutcome::Changed);
        assert_eq!(
            editor.handle(DecodedKey::Submit).unwrap(),
            EditOutcome::Completed
        );

        let Reply::Secret(secret) = worker.join().unwrap().unwrap() else {
            panic!("wrong reply style");
        };
        secret.expose(|value| assert_eq!(value, "sécure".as_bytes()));
        assert_eq!(format!("{editor:?}"), "PromptEditor(<redacted>)");
    }

    #[test]
    fn handles_echo_notice_radio_and_cancel_styles() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOn(c"Login:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let mut editor = PromptEditor::new(pending);
        editor.handle(text("jacob")).unwrap();
        editor.handle(DecodedKey::Submit).unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Reply::Text(_))));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::Info(c"Notice")));
        let mut editor = PromptEditor::new(ui.prompt_timeout(WAIT).unwrap().unwrap());
        editor.handle(DecodedKey::Submit).unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Reply::Acknowledged)));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::Radio(c"Enable?")));
        let mut editor = PromptEditor::new(ui.prompt_timeout(WAIT).unwrap().unwrap());
        editor.handle(DecodedKey::SelectNext).unwrap();
        assert_eq!(editor.radio_selection(), Some(true));
        editor.handle(DecodedKey::Submit).unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Reply::Radio(true))));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let mut editor = PromptEditor::new(ui.prompt_timeout(WAIT).unwrap().unwrap());
        assert_eq!(
            editor.handle(DecodedKey::Cancel).unwrap(),
            EditOutcome::Cancelled
        );
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
    }

    #[test]
    fn rejects_binary_and_overflow_without_partial_edit() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || {
            conversation.respond(Request::Binary {
                kind: 1,
                data: &[2],
            })
        });
        let mut editor = PromptEditor::new(ui.prompt_timeout(WAIT).unwrap().unwrap());
        assert_eq!(editor.handle(text("x")), Err(EditError::UnsupportedPrompt));
        assert_eq!(
            editor.handle(DecodedKey::Submit),
            Err(EditError::UnsupportedPrompt)
        );
        editor.handle(DecodedKey::Cancel).unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let mut editor = PromptEditor::new(ui.prompt_timeout(WAIT).unwrap().unwrap());
        for _ in 0..(MAX_SECRET_BYTES / MAX_DECODED_TEXT_BYTES) {
            editor
                .handle(text(&"x".repeat(MAX_DECODED_TEXT_BYTES)))
                .unwrap();
        }
        assert_eq!(editor.character_count(), Some(MAX_SECRET_BYTES));
        assert_eq!(editor.handle(text("y")), Err(EditError::Full));
        assert_eq!(editor.character_count(), Some(MAX_SECRET_BYTES));
        editor.handle(DecodedKey::Cancel).unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
    }
}
