//! Typed, bounded messages exchanged between a PAM worker and lock UI.
//!
//! The Linux FFI adapter validates raw memory before constructing these values.
//! Implementations may block while the UI collects a response, but the whole
//! conversation runs on the dedicated PAM worker rather than the render loop.

use std::ffi::CStr;
use std::fmt;

use zeroize::Zeroize as _;

use crate::{SecretResponse, MAX_SECRET_BYTES};

pub const MAX_BINARY_RESPONSE_BYTES: usize = MAX_SECRET_BYTES - 5;

#[derive(Clone, Copy)]
pub enum Request<'a> {
    EchoOn(&'a CStr),
    EchoOff(&'a CStr),
    Info(&'a CStr),
    Error(&'a CStr),
    Radio(&'a CStr),
    Binary { kind: u8, data: &'a [u8] },
}

impl Request<'_> {
    pub fn kind(self) -> RequestKind {
        match self {
            Self::EchoOn(_) => RequestKind::EchoOn,
            Self::EchoOff(_) => RequestKind::EchoOff,
            Self::Info(_) => RequestKind::Info,
            Self::Error(_) => RequestKind::Error,
            Self::Radio(_) => RequestKind::Radio,
            Self::Binary { .. } => RequestKind::Binary,
        }
    }
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PamRequest")
            .field("kind", &self.kind())
            .field("content", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestKind {
    EchoOn,
    EchoOff,
    Info,
    Error,
    Radio,
    Binary,
}

pub enum Reply {
    Text(TextResponse),
    Secret(SecretResponse),
    Acknowledged,
    Radio(bool),
    Binary(BinaryResponse),
}

impl Reply {
    pub fn matches(&self, request: RequestKind) -> bool {
        matches!(
            (request, self),
            (RequestKind::EchoOn, Self::Text(_))
                | (RequestKind::EchoOff, Self::Secret(_))
                | (RequestKind::Info | RequestKind::Error, Self::Acknowledged)
                | (RequestKind::Radio, Self::Radio(_))
                | (RequestKind::Binary, Self::Binary(_))
        )
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Text(_) => "PamReply::Text(<redacted>)",
            Self::Secret(_) => "PamReply::Secret(<redacted>)",
            Self::Acknowledged => "PamReply::Acknowledged",
            Self::Radio(_) => "PamReply::Radio(<redacted>)",
            Self::Binary(_) => "PamReply::Binary(<redacted>)",
        })
    }
}

pub struct TextResponse {
    bytes: Vec<u8>,
}

impl TextResponse {
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, ResponseError> {
        let value = value.as_ref();
        validate_text(value)?;
        Ok(Self {
            bytes: value.to_vec(),
        })
    }

    pub fn expose<R>(&self, use_response: impl FnOnce(&[u8]) -> R) -> R {
        use_response(&self.bytes)
    }
}

impl fmt::Debug for TextResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TextResponse(<redacted>)")
    }
}

impl Drop for TextResponse {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

pub struct BinaryResponse {
    kind: u8,
    bytes: Vec<u8>,
}

impl BinaryResponse {
    pub fn new(kind: u8, value: impl AsRef<[u8]>) -> Result<Self, ResponseError> {
        let value = value.as_ref();
        if value.len() > MAX_BINARY_RESPONSE_BYTES {
            return Err(ResponseError::TooLong);
        }
        Ok(Self {
            kind,
            bytes: value.to_vec(),
        })
    }

    pub fn kind(&self) -> u8 {
        self.kind
    }

    pub fn expose<R>(&self, use_response: impl FnOnce(&[u8]) -> R) -> R {
        use_response(&self.bytes)
    }
}

impl fmt::Debug for BinaryResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BinaryResponse(<redacted>)")
    }
}

impl Drop for BinaryResponse {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

pub trait Conversation: Send {
    fn respond(&mut self, request: Request<'_>) -> Result<Reply, ConversationError>;
}

impl<T: Conversation + ?Sized> Conversation for Box<T> {
    fn respond(&mut self, request: Request<'_>) -> Result<Reply, ConversationError> {
        (**self).respond(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationError {
    Cancelled,
    Unavailable,
    InvalidResponse,
}

impl fmt::Display for ConversationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PAM conversation did not produce a valid response")
    }
}

impl std::error::Error for ConversationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseError {
    TooLong,
    InteriorNul,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PAM response is not valid")
    }
}

impl std::error::Error for ResponseError {}

fn validate_text(value: &[u8]) -> Result<(), ResponseError> {
    if value.len() > MAX_SECRET_BYTES {
        return Err(ResponseError::TooLong);
    }
    if value.contains(&0) {
        return Err(ResponseError::InteriorNul);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretInput;

    #[test]
    fn reply_types_match_only_their_protocol_style() {
        let text = Reply::Text(TextResponse::new("jacob").unwrap());
        assert!(text.matches(RequestKind::EchoOn));
        assert!(!text.matches(RequestKind::EchoOff));

        let mut input = SecretInput::new();
        input.push('s').unwrap();
        let secret = Reply::Secret(input.finish());
        assert!(secret.matches(RequestKind::EchoOff));
        assert!(!secret.matches(RequestKind::EchoOn));

        assert!(Reply::Acknowledged.matches(RequestKind::Info));
        assert!(Reply::Acknowledged.matches(RequestKind::Error));
        assert!(Reply::Radio(true).matches(RequestKind::Radio));
        assert!(Reply::Binary(BinaryResponse::new(4, [1, 2]).unwrap()).matches(RequestKind::Binary));
    }

    #[test]
    fn bounded_text_rejects_nul_and_overflow() {
        assert!(matches!(
            TextResponse::new(b"bad\0response"),
            Err(ResponseError::InteriorNul)
        ));
        assert!(matches!(
            TextResponse::new(vec![b'x'; MAX_SECRET_BYTES + 1]),
            Err(ResponseError::TooLong)
        ));
    }

    #[test]
    fn bounded_binary_accounts_for_the_linux_pam_header() {
        assert!(BinaryResponse::new(7, vec![0; MAX_BINARY_RESPONSE_BYTES]).is_ok());
        assert!(matches!(
            BinaryResponse::new(7, vec![0; MAX_BINARY_RESPONSE_BYTES + 1]),
            Err(ResponseError::TooLong)
        ));
    }

    #[test]
    fn diagnostics_redact_prompts_and_every_response() {
        let prompt = c"private prompt";
        let request = Request::EchoOff(prompt);
        let response = Reply::Text(TextResponse::new("private response").unwrap());
        let binary = BinaryResponse::new(9, b"private binary").unwrap();
        let debug = format!("{request:?} {response:?} {binary:?}");
        assert!(!debug.contains("private"));
        assert!(debug.contains("<redacted>"));
    }
}
