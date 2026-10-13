//! The speech-to-text engine behind one small trait: Moonshine through ONNX
//! Runtime on Linux, or the fixture for behaviour tests and CI, which have
//! no model file.

use std::fmt;
use std::time::Instant;

use rmac_intelligence::voice::VoiceModel;
use serde::Serialize;

/// What one transcription cost.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Steps {
    pub preprocess_ms: f64,
    pub encode_ms: f64,
    pub decode_ms: f64,
    pub tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceEngineError {
    /// ONNX Runtime (`libonnxruntime`) is not installed.
    NoRuntime(String),
    NotDownloaded,
    Failed(String),
}

impl fmt::Display for VoiceEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRuntime(detail) => write!(formatter, "ONNX Runtime is not available: {detail}"),
            Self::NotDownloaded => formatter.write_str("the voice model is not downloaded"),
            Self::Failed(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for VoiceEngineError {}

pub trait Transcriber: Send {
    /// The text for 16 kHz mono `samples`.
    fn transcribe(&mut self, samples: &[f32]) -> Result<(String, Steps), VoiceEngineError>;
}

/// `RMAC_VOICE_ENGINE=fixture`: the deterministic stand-in.
pub fn fixture_requested() -> bool {
    std::env::var("RMAC_VOICE_ENGINE").is_ok_and(|value| value == "fixture")
}

/// The fixture: it hears whatever `RMAC_VOICE_FIXTURE_TEXT` says (or, when
/// that is unset, the text in the capture file's `.txt` sidecar), provided
/// the audio is not silent. It still costs a little time per second of
/// audio, so the HUD's "working" state is visible in behaviour runs.
pub struct FixtureTranscriber {
    pub text: String,
}

impl FixtureTranscriber {
    pub fn from_environment(capture_file: Option<&std::path::Path>) -> Self {
        let text = std::env::var("RMAC_VOICE_FIXTURE_TEXT")
            .ok()
            .or_else(|| {
                capture_file
                    .map(|path| path.with_extension("txt"))
                    .and_then(|path| std::fs::read_to_string(path).ok())
            })
            .unwrap_or_default();
        Self {
            text: text.trim().to_owned(),
        }
    }
}

impl Transcriber for FixtureTranscriber {
    fn transcribe(&mut self, samples: &[f32]) -> Result<(String, Steps), VoiceEngineError> {
        let started = Instant::now();
        let heard = rmac_intelligence::voice::audio::rms(samples) > 0.005;
        std::thread::sleep(std::time::Duration::from_millis(
            (samples.len() as u64 / 16).min(150) / 10,
        ));
        let text = if heard {
            self.text.clone()
        } else {
            String::new()
        };
        Ok((
            text.clone(),
            Steps {
                decode_ms: started.elapsed().as_secs_f64() * 1000.0,
                tokens: text.split_whitespace().count() as u32,
                ..Steps::default()
            },
        ))
    }
}

/// Physical cores only, at most four (ADR 0024 §4), as for the language
/// model.
pub fn threads() -> usize {
    rmac_intelligence::gate::current_facts()
        .physical_cores
        .clamp(1, 4) as usize
}

/// Load `model`'s verified files into the real engine.
pub fn open(model: &VoiceModel) -> Result<Box<dyn Transcriber>, VoiceEngineError> {
    let models = rmac_intelligence::paths::models_dir().ok_or(VoiceEngineError::NotDownloaded)?;
    let paths = model.verified(&models).map_err(|error| match error {
        rmac_intelligence::verify::VerifyError::Missing => VoiceEngineError::NotDownloaded,
        other => VoiceEngineError::Failed(other.to_string()),
    })?;
    load(&paths)
}

#[cfg(target_os = "linux")]
fn load(
    paths: &rmac_intelligence::voice::VoicePaths,
) -> Result<Box<dyn Transcriber>, VoiceEngineError> {
    Ok(Box::new(crate::voice::moonshine::Moonshine::load(
        paths,
        threads(),
    )?))
}

#[cfg(not(target_os = "linux"))]
fn load(
    _paths: &rmac_intelligence::voice::VoicePaths,
) -> Result<Box<dyn Transcriber>, VoiceEngineError> {
    Err(VoiceEngineError::NoRuntime(
        "Lulo voice runs on Linux".into(),
    ))
}

/// Whether the speech runtime can be found, without loading it: the
/// service answers `Start` with "no runtime" at once instead of after the
/// user has spoken.
#[cfg(target_os = "linux")]
pub fn runtime_present() -> Result<(), VoiceEngineError> {
    crate::voice::moonshine::find_onnxruntime().map(|_| ())
}

#[cfg(not(target_os = "linux"))]
pub fn runtime_present() -> Result<(), VoiceEngineError> {
    Err(VoiceEngineError::NoRuntime(
        "Lulo voice runs on Linux".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_hears_its_text_only_over_sound() {
        let mut fixture = FixtureTranscriber {
            text: "turn on dark mode".into(),
        };
        let (text, steps) = fixture.transcribe(&[0.2f32; 16_000]).unwrap();
        assert_eq!(text, "turn on dark mode");
        assert_eq!(steps.tokens, 4);
        let (text, _) = fixture.transcribe(&[0.0f32; 16_000]).unwrap();
        assert_eq!(text, "");
    }
}
