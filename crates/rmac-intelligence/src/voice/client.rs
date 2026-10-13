//! The `org.rmac.Voice1` client: one hold-to-talk capture, from the key
//! going down to the transcript.
//!
//! [`listen`] is async and never blocks a thread: run it on a background
//! executor. It subscribes to the service's signals before it starts the
//! capture, so no level is missed, and reads only signals from the
//! connection that answered `Start` (another program on the bus cannot feed
//! it levels or end it).

use std::time::Duration;

use serde::Deserialize;

use super::{BUS_NAME, INTERFACE_NAME, OBJECT_PATH};

/// Starting may activate the service; stopping waits for the model to
/// finish loading and transcribe. Nothing waits longer.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// What the HUD tells a running capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    /// The key was let go: transcribe what was heard.
    Stop,
    /// Escape: throw it away.
    Cancel,
    /// The key turned out to be tapped, not held: end the capture after the
    /// speech and a pause instead of on release.
    EndOnPause,
}

/// What a running capture reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The microphone is open (the service started capturing).
    Listening,
    /// The meter, 0 to 1, about 20 times a second.
    Level(f64),
    /// The key was let go (or the hands-free pause was heard); the text is
    /// being worked out.
    Transcribing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceError {
    /// No session bus, or the voice service is not installed.
    Unavailable,
    /// Lulo Intelligence or Talk to Lulo is off.
    Off,
    NotDownloaded,
    /// The speech runtime (ONNX Runtime) is not installed.
    NoRuntime,
    /// The microphone could not be opened.
    NoMicrophone,
    /// Another capture is running.
    Busy,
    /// The caller is not a Lulo program.
    Refused,
    LowMemory,
    Cancelled,
    Failed,
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "voice is not available",
            Self::Off => "Talk to Lulo is turned off",
            Self::NotDownloaded => "the voice model is not downloaded",
            Self::NoRuntime => "the speech runtime (ONNX Runtime) is not installed",
            Self::NoMicrophone => "the microphone could not be opened",
            Self::Busy => "Lulo is already listening",
            Self::Refused => "the request was refused",
            Self::LowMemory => "not enough free memory right now",
            Self::Cancelled => "cancelled",
            Self::Failed => "Lulo could not understand that",
        })
    }
}

impl std::error::Error for VoiceError {}

/// The service's error names, `org.rmac.Voice1.Error.<Name>`.
pub mod error_names {
    pub const OFF: &str = "org.rmac.Voice1.Error.Off";
    pub const NOT_DOWNLOADED: &str = "org.rmac.Voice1.Error.NotDownloaded";
    pub const NO_RUNTIME: &str = "org.rmac.Voice1.Error.NoRuntime";
    pub const NO_MICROPHONE: &str = "org.rmac.Voice1.Error.NoMicrophone";
    pub const BUSY: &str = "org.rmac.Voice1.Error.Busy";
    pub const REFUSED: &str = "org.rmac.Voice1.Error.Refused";
    pub const LOW_MEMORY: &str = "org.rmac.Voice1.Error.LowMemory";
    pub const CANCELLED: &str = "org.rmac.Voice1.Error.Cancelled";
    pub const FAILED: &str = "org.rmac.Voice1.Error.Failed";
}

pub fn map_error(error: zbus::Error) -> VoiceError {
    match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            error_names::OFF => VoiceError::Off,
            error_names::NOT_DOWNLOADED => VoiceError::NotDownloaded,
            error_names::NO_RUNTIME => VoiceError::NoRuntime,
            error_names::NO_MICROPHONE => VoiceError::NoMicrophone,
            error_names::BUSY => VoiceError::Busy,
            error_names::REFUSED => VoiceError::Refused,
            error_names::LOW_MEMORY => VoiceError::LowMemory,
            error_names::CANCELLED => VoiceError::Cancelled,
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited" => VoiceError::Unavailable,
            _ => VoiceError::Failed,
        },
        zbus::Error::InputOutput(_) | zbus::Error::Address(_) => VoiceError::Unavailable,
        _ => VoiceError::Failed,
    }
}

/// What the service measured for one capture.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Timing {
    /// Audio captured.
    pub audio_ms: f64,
    /// `Stop` received, to the transcript: what the user waits for after
    /// letting go of the key.
    pub stop_to_text_ms: f64,
    /// Of that, waiting for the model to finish loading.
    pub waited_for_model_ms: f64,
    pub preprocess_ms: f64,
    pub encode_ms: f64,
    pub decode_ms: f64,
    /// Tokens written by the decoder.
    pub tokens: u32,
    /// Loading the model (overlapping the speech).
    pub model_load_ms: f64,
    /// `Start` received, to the microphone delivering audio.
    pub start_to_audio_ms: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Transcript {
    pub text: String,
    pub timing: Timing,
}

/// Parse `Stop`'s reply.
pub fn parse_transcript(json: &str) -> Result<Transcript, VoiceError> {
    serde_json::from_str(json).map_err(|_| VoiceError::Failed)
}

enum Incoming {
    Signal(zbus::Result<zbus::Message>),
    Owner(zbus::Result<zbus::Message>),
    Control(Control),
}

/// One capture: start the microphone, report levels through `on_event`,
/// and on [`Control::Stop`] (or the service's own hands-free end) return
/// the transcript. `hands_free` means the key was tapped, not held: the
/// service ends the capture itself after speech and a pause.
pub async fn listen(
    hands_free: bool,
    controls: async_channel::Receiver<Control>,
    mut on_event: impl FnMut(Event),
) -> Result<Transcript, VoiceError> {
    use futures_util::StreamExt as _;

    let connection = zbus::connection::Builder::session()
        .map_err(|_| VoiceError::Unavailable)?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .await
        .map_err(|_| VoiceError::Unavailable)?;
    let signals_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE_NAME)
        .and_then(|rule| rule.path(OBJECT_PATH))
        .map_err(|_| VoiceError::Failed)?
        .build();
    let signals = zbus::MessageStream::for_match_rule(signals_rule, &connection, Some(256))
        .await
        .map_err(map_error)?;
    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .and_then(|rule| rule.interface("org.freedesktop.DBus"))
        .and_then(|rule| rule.member("NameOwnerChanged"))
        .and_then(|rule| rule.add_arg(BUS_NAME))
        .map_err(|_| VoiceError::Failed)?
        .build();
    let owners = zbus::MessageStream::for_match_rule(owner_rule, &connection, Some(16))
        .await
        .map_err(map_error)?;
    let reply = connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE_NAME),
            "Start",
            &(hands_free,),
        )
        .await
        .map_err(map_error)?;
    let service = reply
        .header()
        .sender()
        .map(|sender| sender.to_owned())
        .ok_or(VoiceError::Failed)?;
    on_event(Event::Listening);

    let call = |method: &'static str| {
        let connection = connection.clone();
        let service = service.clone();
        async move {
            connection
                .call_method(
                    Some(service.as_str()),
                    OBJECT_PATH,
                    Some(INTERFACE_NAME),
                    method,
                    &(),
                )
                .await
                .map_err(map_error)
        }
    };
    let stop = |reply: zbus::Message| -> Result<Transcript, VoiceError> {
        let json: String = reply.body().deserialize().map_err(|_| VoiceError::Failed)?;
        parse_transcript(&json)
    };

    let mut incoming = std::pin::pin!(futures_util::stream::select(
        signals.map(Incoming::Signal),
        futures_util::stream::select(
            owners.map(Incoming::Owner),
            controls.map(Incoming::Control),
        ),
    ));
    while let Some(event) = incoming.next().await {
        match event {
            Incoming::Signal(Ok(message)) => {
                let header = message.header();
                if header.sender() != Some(&service) {
                    continue;
                }
                match header.member().map(|member| member.as_str()) {
                    Some("Level") => {
                        if let Ok(level) = message.body().deserialize::<f64>() {
                            on_event(Event::Level(level.clamp(0.0, 1.0)));
                        }
                    }
                    Some("Ended") => {
                        on_event(Event::Transcribing);
                        return stop(call("Stop").await?);
                    }
                    _ => {}
                }
            }
            Incoming::Signal(Err(_)) => return Err(VoiceError::Failed),
            Incoming::Owner(Ok(message)) => {
                if let Ok((_, _, new_owner)) =
                    message.body().deserialize::<(String, String, String)>()
                {
                    if new_owner != service.as_str() {
                        return Err(VoiceError::Failed);
                    }
                }
            }
            Incoming::Owner(Err(_)) => {}
            Incoming::Control(Control::Stop) => {
                on_event(Event::Transcribing);
                return stop(call("Stop").await?);
            }
            Incoming::Control(Control::Cancel) => {
                let _ = call("Cancel").await;
                return Err(VoiceError::Cancelled);
            }
            Incoming::Control(Control::EndOnPause) => {
                let _ = call("EndOnPause").await;
            }
        }
    }
    // Every control sender is gone: nobody wants the answer.
    let _ = call("Cancel").await;
    Err(VoiceError::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_and_errors_parse() {
        let transcript = parse_transcript(
            r#"{"text":"turn on dark mode","timing":{"audio_ms":1800.0,"stop_to_text_ms":212.5,"tokens":6}}"#,
        )
        .unwrap();
        assert_eq!(transcript.text, "turn on dark mode");
        assert_eq!(transcript.timing.tokens, 6);
        assert_eq!(transcript.timing.stop_to_text_ms, 212.5);
        assert_eq!(parse_transcript("nope"), Err(VoiceError::Failed));
        let error = |name: &str| {
            map_error(zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name).unwrap(),
                None,
                zbus::message::Message::method_call("/", "Ping")
                    .unwrap()
                    .build(&())
                    .unwrap(),
            ))
        };
        assert_eq!(error(error_names::OFF), VoiceError::Off);
        assert_eq!(error(error_names::NO_RUNTIME), VoiceError::NoRuntime);
        assert_eq!(error(error_names::BUSY), VoiceError::Busy);
        assert_eq!(
            error("org.freedesktop.DBus.Error.ServiceUnknown"),
            VoiceError::Unavailable
        );
        assert_eq!(error("org.example.Other"), VoiceError::Failed);
    }
}
