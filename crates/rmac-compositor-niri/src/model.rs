//! Stable niri connection policy and error model.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub maximum_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            maximum_delay: Duration::from_secs(5),
        }
    }
}

impl ReconnectPolicy {
    pub(super) fn next_delay(self, current: Duration) -> Duration {
        current.saturating_mul(2).min(self.maximum_delay)
    }
}

#[derive(Debug)]
pub enum Error {
    MissingSocketPath,
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    UnexpectedResponse(&'static str),
    InitialStateIncomplete,
    ConsumerClosed,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSocketPath => write!(formatter, "{SOCKET_PATH_ENV} is not set"),
            Self::Io(error) => write!(formatter, "niri IPC I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "niri IPC JSON failed: {error}"),
            Self::Protocol(error) => write!(formatter, "niri IPC protocol failed: {error}"),
            Self::UnexpectedResponse(request) => {
                write!(formatter, "niri returned an unexpected {request} response")
            }
            Self::InitialStateIncomplete => {
                write!(
                    formatter,
                    "niri event stream omitted its initial workspace/window/overview state"
                )
            }
            Self::ConsumerClosed => write!(formatter, "compositor event consumer closed"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
