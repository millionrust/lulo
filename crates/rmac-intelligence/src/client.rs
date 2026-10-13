//! The `org.rmac.Intelligence1` client.
//!
//! - [`intent`], [`prepare`] and [`calibrate`] block: call them from a
//!   blocking pool, never a render path.
//! - [`write`] (Writing Tools) is async and streams: the service answers
//!   `Write` with a job number at once, then sends the answer as `Delta`
//!   signals and ends with one `Done`, all addressed to this connection
//!   only. Run it on a background executor; it never blocks a thread.
//!
//! The service is D-Bus activated on the first call and exits 60 s after
//! the last one, so a client never holds it open.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

use crate::{Intent, Task, BUS_NAME, INTERFACE_NAME, OBJECT_PATH};

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
    pub const CANCELLED: &str = "org.rmac.Intelligence1.Error.Cancelled";
    pub const FAILED: &str = "org.rmac.Intelligence1.Error.Failed";
    pub const TOO_LONG: &str = "org.rmac.Intelligence1.Error.TooLong";
    pub const NEEDS_FASTER_PC: &str = "org.rmac.Intelligence1.Error.NeedsFasterPc";
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
    /// A newer request from this program replaced this one, or the caller
    /// stopped it.
    Cancelled,
    /// The text is longer than the task takes.
    TooLong,
    /// This task needs the Standard model, which this PC cannot run.
    NeedsFasterPc,
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
            Self::Cancelled => "the request was stopped",
            Self::TooLong => "the text is too long",
            Self::NeedsFasterPc => "this needs a faster PC",
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
            error_names::CANCELLED => ClientError::Cancelled,
            error_names::TOO_LONG => ClientError::TooLong,
            error_names::NEEDS_FASTER_PC => ClientError::NeedsFasterPc,
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
    /// Waiting behind an earlier request.
    pub queued_ms: f64,
    /// Restoring the saved prefix state.
    pub rewind_ms: f64,
    /// Evaluating the request's own tokens.
    pub prefill_ms: f64,
    /// The schema-guided decode after the request.
    pub decode_ms: f64,
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

/// What the service measured for one Writing Tools answer.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WriteTiming {
    /// Request in, to the first token of the answer.
    pub first_token_ms: f64,
    /// Request in, to the last token.
    pub total_ms: f64,
    /// Whether this request had to load the model first.
    pub cold: bool,
    /// Waiting behind an earlier request.
    pub queued_ms: f64,
    /// Restoring (or, the first time, evaluating) the task's prefix.
    pub prefix_ms: f64,
    /// The prefix was evaluated for this request (no saved state yet).
    pub prefix_evaluated: bool,
    pub prefix_tokens: u32,
    /// The user's text, in tokens.
    pub input_tokens: u32,
    /// Evaluating the user's text.
    pub prefill_ms: f64,
    pub output_tokens: u32,
    /// Generating the answer, after its first token.
    pub decode_ms: f64,
    pub decode_tok_s: f64,
}

/// How a Writing Tools answer ended.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteDone {
    /// The finished answer, cleaned up ([`crate::writing::finish`]). It can
    /// differ from the streamed deltas (a dropped preface line).
    pub text: String,
    /// The answer reached the task's output cap.
    pub capped: bool,
    pub timing: WriteTiming,
}

#[derive(Deserialize)]
struct WireDone {
    #[serde(default)]
    text: String,
    #[serde(default)]
    timing: WriteTiming,
}

/// `Done`'s outcome strings, as the service sends them.
pub mod outcomes {
    pub const FINISHED: &str = "finished";
    pub const CAPPED: &str = "capped";
    pub const CANCELLED: &str = "cancelled";
    pub const TOO_LONG: &str = "too_long";
    pub const NEEDS_FASTER_PC: &str = "needs_faster_pc";
    pub const OFF: &str = "off";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const NOT_DOWNLOADED: &str = "not_downloaded";
    pub const LOW_MEMORY: &str = "low_memory";
    pub const FAILED: &str = "failed";
}

/// Read one `Done(job, outcome, detail)`.
pub fn parse_done(outcome: &str, detail: &str) -> Result<WriteDone, ClientError> {
    let error = match outcome {
        outcomes::FINISHED | outcomes::CAPPED => None,
        outcomes::CANCELLED => Some(ClientError::Cancelled),
        outcomes::TOO_LONG => Some(ClientError::TooLong),
        outcomes::NEEDS_FASTER_PC => Some(ClientError::NeedsFasterPc),
        outcomes::OFF => Some(ClientError::Off),
        outcomes::UNAVAILABLE => Some(ClientError::NotSupported),
        outcomes::NOT_DOWNLOADED => Some(ClientError::NotDownloaded),
        outcomes::LOW_MEMORY => Some(ClientError::LowMemory),
        _ => Some(ClientError::Failed),
    };
    if let Some(error) = error {
        return Err(error);
    }
    let wire: WireDone = serde_json::from_str(detail).map_err(|_| ClientError::Failed)?;
    Ok(WriteDone {
        text: wire.text,
        capped: outcome == outcomes::CAPPED,
        timing: wire.timing,
    })
}

/// The shared async connection for [`write`], opened on first use. Two
/// first calls racing may both connect; one connection is kept.
async fn async_connection() -> Result<zbus::Connection, ClientError> {
    static CONNECTION: OnceLock<zbus::Connection> = OnceLock::new();
    if let Some(connection) = CONNECTION.get() {
        return Ok(connection.clone());
    }
    let connection = zbus::connection::Builder::session()
        .map_err(|_| ClientError::Unavailable)?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .await
        .map_err(|_| ClientError::Unavailable)?;
    Ok(CONNECTION.get_or_init(|| connection).clone())
}

enum Incoming {
    Signal(zbus::Result<zbus::Message>),
    Owner(zbus::Result<zbus::Message>),
    Cancel,
}

/// Run one Writing Tools task on `text`, calling `on_delta` with each piece
/// of the answer as the model writes it. `attempt` is 0 for the first
/// answer and counts up for Try Again, so a sampled task writes something
/// new. Sending on (or closing) `cancel` stops the answer: the service is
/// told at once and the call ends with [`ClientError::Cancelled`].
///
/// Only signals from the connection that answered `Write`, for this job,
/// are read, so another program on the bus cannot inject text; and if the
/// service goes away mid-answer this ends instead of waiting forever.
pub async fn write(
    task: Task,
    text: &str,
    attempt: u32,
    cancel: async_channel::Receiver<()>,
    mut on_delta: impl FnMut(&str),
) -> Result<WriteDone, ClientError> {
    use futures_util::StreamExt as _;

    if !task.is_writing() {
        return Err(ClientError::Refused);
    }
    let connection = async_connection().await?;
    let signals_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE_NAME)
        .and_then(|rule| rule.path(OBJECT_PATH))
        .map_err(|_| ClientError::Failed)?
        .build();
    let signals = zbus::MessageStream::for_match_rule(signals_rule, &connection, Some(4096))
        .await
        .map_err(map_error)?;
    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .and_then(|rule| rule.interface("org.freedesktop.DBus"))
        .and_then(|rule| rule.member("NameOwnerChanged"))
        .and_then(|rule| rule.add_arg(BUS_NAME))
        .map_err(|_| ClientError::Failed)?
        .build();
    let owners = zbus::MessageStream::for_match_rule(owner_rule, &connection, Some(16))
        .await
        .map_err(map_error)?;
    let reply = connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE_NAME),
            "Write",
            &(task.as_str(), text, attempt),
        )
        .await
        .map_err(map_error)?;
    let service = reply
        .header()
        .sender()
        .map(|sender| sender.to_owned())
        .ok_or(ClientError::Failed)?;
    let job: u64 = reply
        .body()
        .deserialize()
        .map_err(|_| ClientError::Failed)?;

    let mut incoming = std::pin::pin!(futures_util::stream::select(
        signals.map(Incoming::Signal),
        futures_util::stream::select(
            owners.map(Incoming::Owner),
            cancel.map(|_| Incoming::Cancel),
        ),
    ));
    let mut cancelling = false;
    while let Some(event) = incoming.next().await {
        match event {
            Incoming::Signal(Ok(message)) => {
                let header = message.header();
                if header.sender() != Some(&service) {
                    continue;
                }
                match header.member().map(|member| member.as_str()) {
                    Some("Delta") => {
                        if let Ok((id, piece)) = message.body().deserialize::<(u64, String)>() {
                            if id == job && !cancelling {
                                on_delta(&piece);
                            }
                        }
                    }
                    Some("Done") => {
                        if let Ok((id, outcome, detail)) =
                            message.body().deserialize::<(u64, String, String)>()
                        {
                            if id == job {
                                return parse_done(&outcome, &detail);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Incoming::Signal(Err(_)) => return Err(ClientError::Failed),
            Incoming::Owner(Ok(message)) => {
                if let Ok((_, _, new_owner)) =
                    message.body().deserialize::<(String, String, String)>()
                {
                    if new_owner != service.as_str() {
                        // The service exited (or crashed) mid-answer.
                        return Err(if cancelling {
                            ClientError::Cancelled
                        } else {
                            ClientError::Failed
                        });
                    }
                }
            }
            Incoming::Owner(Err(_)) => {}
            Incoming::Cancel => {
                if !cancelling {
                    cancelling = true;
                    let _ = connection
                        .call_method(
                            Some(service.as_str()),
                            OBJECT_PATH,
                            Some(INTERFACE_NAME),
                            "Cancel",
                            &(job,),
                        )
                        .await;
                }
            }
        }
    }
    Err(ClientError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_signals_map_to_results() {
        let done = parse_done(
            outcomes::FINISHED,
            r#"{"text":"The cat sat.","timing":{"first_token_ms":3200.0,"output_tokens":5}}"#,
        )
        .unwrap();
        assert_eq!(done.text, "The cat sat.");
        assert!(!done.capped);
        assert_eq!(done.timing.output_tokens, 5);
        assert!(
            parse_done(outcomes::CAPPED, r#"{"text":"x"}"#)
                .unwrap()
                .capped
        );
        assert_eq!(
            parse_done(outcomes::CANCELLED, ""),
            Err(ClientError::Cancelled)
        );
        assert_eq!(
            parse_done(outcomes::TOO_LONG, ""),
            Err(ClientError::TooLong)
        );
        assert_eq!(
            parse_done(outcomes::NEEDS_FASTER_PC, ""),
            Err(ClientError::NeedsFasterPc)
        );
        assert_eq!(parse_done(outcomes::OFF, ""), Err(ClientError::Off));
        assert_eq!(parse_done("surprise", "{}"), Err(ClientError::Failed));
        assert_eq!(
            parse_done(outcomes::FINISHED, "not json"),
            Err(ClientError::Failed)
        );
    }

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
