//! The `org.rmac.Intelligence1` client. Blocking: call it from a blocking
//! pool, never a render path.
//!
//! The service is D-Bus activated on the first call and exits 60 s after
//! the last one, so a client never holds it open.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

use crate::{Intent, Task};

/// A cold call loads the model and evaluates the prompt prefix once; on
/// the reference laptop that is a few seconds. Nothing waits longer.
const CALL_TIMEOUT: Duration = Duration::from_secs(45);

/// Error names the service replies with (`<interface>.Error.<Name>`).
pub mod error_names {
    pub const OFF: &str = "org.rmac.Intelligence1.Error.Off";
    pub const UNAVAILABLE: &str = "org.rmac.Intelligence1.Error.Unavailable";
    pub const NOT_DOWNLOADED: &str = "org.rmac.Intelligence1.Error.NotDownloaded";
    pub const LOW_MEMORY: &str = "org.rmac.Intelligence1.Error.LowMemory";
    pub const REFUSED: &str = "org.rmac.Intelligence1.Error.Refused";
    pub const FAILED: &str = "org.rmac.Intelligence1.Error.Failed";
}

#[zbus::proxy(
    interface = "org.rmac.Intelligence1",
    default_service = "org.rmac.Intelligence1",
    default_path = "/org/rmac/Intelligence1"
)]
trait Intelligence {
    /// Load the model and its saved prompt state now (Spotlight calls this
    /// as soon as a query looks like a request, so loading overlaps typing).
    fn prepare(&self) -> zbus::Result<()>;
    /// Run one task from the closed list on `text`; replies with JSON.
    fn run(&self, task: &str, text: &str) -> zbus::Result<String>;
    /// Measure decode speed for the hardware gate; replies with JSON.
    fn calibrate(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    /// No session bus, or the service is not installed.
    Unavailable,
    /// The user has not turned Lulo Intelligence on.
    Off,
    NotDownloaded,
    /// Not enough free memory to load the model right now.
    LowMemory,
    /// This PC cannot run the model (hardware gate).
    NotSupported,
    /// The caller is not a Lulo program, or the task is not on the list.
    Refused,
    Failed,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "Lulo Intelligence is not available",
            Self::Off => "Lulo Intelligence is turned off",
            Self::NotDownloaded => "the Lulo Intelligence model is not downloaded",
            Self::LowMemory => "not enough free memory right now",
            Self::NotSupported => "this PC cannot run Lulo Intelligence",
            Self::Refused => "the request was refused",
            Self::Failed => "Lulo Intelligence could not answer",
        })
    }
}

impl std::error::Error for ClientError {}

fn map_error(error: zbus::Error) -> ClientError {
    match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            error_names::OFF => ClientError::Off,
            error_names::NOT_DOWNLOADED => ClientError::NotDownloaded,
            error_names::LOW_MEMORY => ClientError::LowMemory,
            error_names::UNAVAILABLE => ClientError::NotSupported,
            error_names::REFUSED => ClientError::Refused,
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited" => ClientError::Unavailable,
            _ => ClientError::Failed,
        },
        zbus::Error::InputOutput(_) | zbus::Error::Address(_) => ClientError::Unavailable,
        _ => ClientError::Failed,
    }
}

/// One connection with its own call timeout, opened on first use.
fn connection() -> Result<zbus::blocking::Connection, ClientError> {
    static CONNECTION: OnceLock<zbus::blocking::Connection> = OnceLock::new();
    if let Some(connection) = CONNECTION.get() {
        return Ok(connection.clone());
    }
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.method_timeout(CALL_TIMEOUT).build())
        .map_err(|_| ClientError::Unavailable)?;
    Ok(CONNECTION.get_or_init(|| connection).clone())
}

fn proxy() -> Result<IntelligenceProxyBlocking<'static>, ClientError> {
    IntelligenceProxyBlocking::new(&connection()?).map_err(|_| ClientError::Unavailable)
}

/// What the service measured for one request.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Timing {
    /// Request in, to the first token chosen.
    pub first_token_ms: f64,
    /// Request in, to the parsed intent.
    pub total_ms: f64,
    /// Whether this request had to load the model first.
    pub cold: bool,
    /// Prompt-prefix tokens restored from the saved state.
    pub cached_prefix_tokens: u32,
    /// Tokens evaluated for the request itself.
    pub request_tokens: u32,
    /// Forward passes after the request.
    pub passes: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub intent: Intent,
    pub timing: Timing,
}

#[derive(Deserialize)]
struct WireReply {
    intent: serde_json::Value,
    #[serde(default)]
    timing: Timing,
}

/// Parse a `Run("intent", …)` reply.
pub fn parse_reply(json: &str) -> Result<Reply, ClientError> {
    let wire: WireReply = serde_json::from_str(json).map_err(|_| ClientError::Failed)?;
    let intent = Intent::from_value(&wire.intent).map_err(|_| ClientError::Failed)?;
    Ok(Reply {
        intent,
        timing: wire.timing,
    })
}

/// Turn one Spotlight sentence into an intent.
pub fn intent(text: &str) -> Result<Reply, ClientError> {
    let reply = proxy()?
        .run(Task::Intent.as_str(), text)
        .map_err(map_error)?;
    parse_reply(&reply)
}

/// Start loading the model now; returns once it is ready.
pub fn prepare() -> Result<(), ClientError> {
    proxy()?.prepare().map_err(map_error)
}

/// The measured decode speeds, for the hardware gate.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct CalibrationReply {
    pub tier: String,
    pub decode_tok_s: f64,
    pub prefill_tok_s: f64,
}

pub fn calibrate() -> Result<CalibrationReply, ClientError> {
    let reply = proxy()?.calibrate().map_err(map_error)?;
    serde_json::from_str(&reply).map_err(|_| ClientError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_validated_like_any_intent() {
        let reply = parse_reply(
            r#"{"intent":{"intent":"appearance","mode":"dark"},"timing":{"first_token_ms":210.5,"total_ms":330.0,"cold":false,"cached_prefix_tokens":512,"request_tokens":12,"passes":1}}"#,
        )
        .unwrap();
        assert_eq!(
            reply.intent,
            Intent::Appearance {
                mode: crate::AppearanceMode::Dark
            }
        );
        assert_eq!(reply.timing.cached_prefix_tokens, 512);
        assert_eq!(
            parse_reply(r#"{"intent":{"intent":"reboot"}}"#),
            Err(ClientError::Failed)
        );
        assert_eq!(parse_reply("not json"), Err(ClientError::Failed));
    }
}
