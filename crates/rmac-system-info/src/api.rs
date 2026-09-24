use std::fmt;

use crate::host::{
    normalize_static_hostname, system_set_static_hostname, system_snapshot, verify_static_hostname,
};

const MAX_FACT_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub hostname: String,
    pub static_hostname: Option<String>,
    pub pretty_hostname: Option<String>,
    pub hostname_mutable: bool,
    pub hostname_unavailable_reason: Option<String>,
    pub operating_system: String,
    pub kernel: String,
    pub architecture: String,
    pub hardware_vendor: Option<String>,
    pub hardware_model: Option<String>,
    pub processor: Option<String>,
    pub memory: Option<String>,
    pub graphics: Option<String>,
    pub session: Option<String>,
    pub desktop: Option<String>,
}

impl Snapshot {
    pub fn display_hostname(&self) -> &str {
        self.static_hostname
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.hostname)
    }

    /// A deliberately bounded report suitable for the clipboard or a bug.
    pub fn diagnostic_report(&self) -> String {
        let mut lines = vec!["Lulo OS system report".to_string()];
        push_fact(&mut lines, "Operating system", Some(&self.operating_system));
        push_fact(&mut lines, "Kernel", Some(&self.kernel));
        push_fact(&mut lines, "Architecture", Some(&self.architecture));
        push_fact(
            &mut lines,
            "Hardware vendor",
            self.hardware_vendor.as_deref(),
        );
        push_fact(&mut lines, "Hardware model", self.hardware_model.as_deref());
        push_fact(&mut lines, "Processor", self.processor.as_deref());
        push_fact(&mut lines, "Memory", self.memory.as_deref());
        push_fact(&mut lines, "Graphics", self.graphics.as_deref());
        push_fact(&mut lines, "Session", self.session.as_deref());
        push_fact(&mut lines, "Desktop", self.desktop.as_deref());
        lines.join("\n") + "\n"
    }
}

fn push_fact(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| valid_fact(value)) {
        lines.push(format!("{label}: {value}"));
    }
}

pub(crate) fn valid_fact(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= MAX_FACT_BYTES && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidName,
    Unavailable,
    Authorization,
    Mutation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            operation,
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Injectable boundary used by the UI and fixture-backed consumers.
pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(system_snapshot())
    }

    fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error> {
        let hostname = normalize_static_hostname(hostname)?;
        system_set_static_hostname(&hostname)?;
        let snapshot = self.snapshot()?;
        verify_static_hostname(snapshot, &hostname)
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_static_hostname(hostname: &str) -> Result<Snapshot, Error> {
    SystemService.set_static_hostname(hostname)
}

pub fn validate_static_hostname(hostname: &str) -> Result<(), Error> {
    normalize_static_hostname(hostname).map(drop)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}
