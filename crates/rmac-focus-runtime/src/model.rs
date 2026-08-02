use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceHealth {
    Healthy,
    RecoveredLastGood,
    RecoveredDefaults,
    SaveFailed,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Projection {
    pub enabled: bool,
    pub mode_name: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

impl fmt::Debug for Projection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Projection")
            .field("enabled", &self.enabled)
            .field("mode_name", &self.mode_name.as_ref().map(|_| "<redacted>"))
            .field("ends_at_unix_ms", &self.ends_at_unix_ms)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub evaluation: rmac_focus::Evaluation,
    pub projection: Projection,
    pub persistence: PersistenceHealth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Load,
    Invalid,
    Clock,
}
