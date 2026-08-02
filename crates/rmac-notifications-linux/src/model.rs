use std::fmt;

use crate::media;

pub const FREEDESKTOP_CAPABILITIES: &[&str] = &["actions", "body", "persistence"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    WrongType,
    InvalidValue,
    InvalidMarkup,
    InvalidTarget,
    InvalidMedia(media::ErrorKind),
    Domain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub field: &'static str,
    pub kind: ErrorKind,
}

impl Error {
    pub(crate) fn new(field: &'static str, kind: ErrorKind) -> Self {
        Self { field, kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid notification field {} ({:?})",
            self.field, self.kind
        )
    }
}

impl std::error::Error for Error {}
