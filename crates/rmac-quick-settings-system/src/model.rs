use std::fmt;

use rmac_quick_settings::Control;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Mutate,
    Refresh,
}

impl fmt::Display for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Mutate => "change",
            Self::Refresh => "refresh",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub control: Control,
    pub phase: Phase,
    detail: String,
}

impl Error {
    pub(crate) fn new(control: Control, phase: Phase, detail: impl Into<String>) -> Self {
        Self {
            control,
            phase,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not {} {}: {}",
            self.phase,
            self.control.label(),
            self.detail
        )
    }
}

impl std::error::Error for Error {}
