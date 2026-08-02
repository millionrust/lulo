use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    LaunchApplication,
    RevealApplication,
    OpenSetting,
    OpenFile,
    RevealFile,
    CopyText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    InvalidAction,
    Io(std::io::ErrorKind),
    Unavailable,
    Rejected,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    pub kind: FailureKind,
    pub(crate) detail: String,
}

impl BackendError {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub activation: ActivationId,
    pub operation: Operation,
    pub kind: FailureKind,
    pub(crate) detail: String,
}

impl Error {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.operation {
            Operation::LaunchApplication => "Could not launch the application",
            Operation::RevealApplication => "Could not show the application",
            Operation::OpenSetting => "Could not open Settings",
            Operation::OpenFile => "Could not open the file",
            Operation::RevealFile => "Could not reveal the file",
            Operation::CopyText => "Could not copy the result",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    ApplicationLaunched { launch: rmac_app_launch::Outcome },
    ApplicationRevealed,
    SettingOpened,
    FileOpened,
    FileRevealed,
    TextCopied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub activation: ActivationId,
    pub outcome: Outcome,
}
