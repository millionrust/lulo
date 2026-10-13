//! Moonshine (Useful Sensors, MIT; English) run by ONNX Runtime.
//!
//! The model files are sherpa-onnx's ONNX export of Moonshine, and the
//! decoding below follows sherpa-onnx's own `OfflineMoonshineModel` and
//! greedy search (Apache-2.0): preprocess the waveform, encode it once,
//! then decode greedily with the decoder's key/value cache. ADR 0024 §8
//! chose Moonshine via sherpa-onnx; sherpa-onnx's Rust crate itself is not
//! linked because its build downloads prebuilt static archives that also
//! carry espeak-ng (GPL-3.0) for its TTS, which Lulo's MIT binaries must
//! not link, and because a package build may not download anything. ONNX
//! Runtime is the system library (Ubuntu's `libonnxruntime1.23`, MIT),
//! loaded with `dlopen` only when a capture starts, so nothing native is
//! compiled into Lulo and no build-host path can end up in the binary.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use rmac_intelligence::voice::tokens::{self, Vocabulary};
use rmac_intelligence::voice::VoicePaths;

use super::engine::{Steps, Transcriber, VoiceEngineError};

/// Where Ubuntu and Debian install `libonnxruntime.so.<version>`.
const LIBRARY_DIRECTORIES: &[&str] = &[
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/lib64",
    "/usr/lib",
];

/// The `ort` binding speaks ONNX Runtime's 1.23 API; any later 1.x works.
const MINIMUM_MINOR: u32 = 23;

/// Find ONNX Runtime: `ORT_DYLIB_PATH` if set (measurements with a library
/// unpacked outside the system), otherwise the newest
/// `libonnxruntime.so.1.<minor>[.<patch>]` the system has.
pub fn find_onnxruntime() -> Result<PathBuf, VoiceEngineError> {
    if let Some(path) = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from) {
        return if path.is_file() {
            Ok(path)
        } else {
            Err(VoiceEngineError::NoRuntime(format!(
                "{} does not exist",
                path.display()
            )))
        };
    }
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for directory in LIBRARY_DIRECTORIES {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(version) = name
                .to_str()
                .and_then(|name| name.strip_prefix("libonnxruntime.so."))
                .and_then(parse_version)
            else {
                continue;
            };
            if version.first() != Some(&1) || version.get(1).copied().unwrap_or(0) < MINIMUM_MINOR
            {
                continue;
            }
            if best.as_ref().is_none_or(|(current, _)| version > *current) {
                best = Some((version, entry.path()));
            }
        }
    }
    best.map(|(_, path)| path).ok_or_else(|| {
        VoiceEngineError::NoRuntime("libonnxruntime 1.23 or later is not installed".into())
    })
}

fn parse_version(text: &str) -> Option<Vec<u32>> {
    text.split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<u32>>>()
        .filter(|parts| !parts.is_empty())
}

/// Load ONNX Runtime once per process.
fn runtime() -> Result<(), VoiceEngineError> {
    static LOADED: OnceLock<Result<(), VoiceEngineError>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            let path = find_onnxruntime()?;
            let builder = ort::init_from(&path)
                .map_err(|error| VoiceEngineError::NoRuntime(error.to_string()))?
                .with_name("lulo-voice");
            let _ = builder.commit();
            Ok(())
        })
        .clone()
}

fn failed(error: impl std::fmt::Display) -> VoiceEngineError {
    VoiceEngineError::Failed(error.to_string())
}

fn session(path: &Path, threads: usize) -> Result<Session, VoiceEngineError> {
    Session::builder()
        .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
        .and_then(|builder| builder.with_intra_threads(threads))
        .and_then(|builder| builder.with_inter_threads(1))
        // Idle worker threads sleep instead of spinning: ONNX Runtime's
        // default busy-waits a core between runs.
        .and_then(|builder| builder.with_intra_op_spinning(false))
        .and_then(|builder| builder.with_inter_op_spinning(false))
        .and_then(|builder| builder.commit_from_file(path))
        .map_err(|error| failed(format!("{}: {error}", path.display())))
}

pub struct Moonshine {
    preprocess: Session,
    encode: Session,
    uncached_decode: Session,
    cached_decode: Session,
    vocabulary: Vocabulary,
}

impl Moonshine {
    pub fn load(paths: &VoicePaths, threads: usize) -> Result<Self, VoiceEngineError> {
        runtime()?;
        let vocabulary = std::fs::read_to_string(&paths.tokens)
            .map(|text| Vocabulary::parse(&text))
            .map_err(failed)?;
        if vocabulary.is_empty() {
            return Err(failed("the voice vocabulary is empty"));
        }
        Ok(Self {
            preprocess: session(&paths.preprocess, threads)?,
            encode: session(&paths.encode, threads)?,
            uncached_decode: session(&paths.uncached_decode, threads)?,
            cached_decode: session(&paths.cached_decode, threads)?,
            vocabulary,
        })
    }
}

/// Run `session` on positional `inputs` and keep every output, in order.
fn run(session: &mut Session, inputs: Vec<SessionInputValue<'_>>) -> Result<Vec<DynValue>, VoiceEngineError> {
    let outputs = session.run(inputs.as_slice()).map_err(failed)?;
    Ok(outputs.into_iter().map(|(_, value)| value).collect())
}

fn int_tensor(shape: Vec<i64>, values: Vec<i32>) -> Result<SessionInputValue<'static>, VoiceEngineError> {
    Tensor::from_array((shape, values))
        .map(SessionInputValue::from)
        .map_err(failed)
}

/// The most likely token in a `[1, 1, vocab]` logits tensor.
fn best_token(logits: &DynValue) -> Result<i32, VoiceEngineError> {
    let (_, values) = logits.try_extract_tensor::<f32>().map_err(failed)?;
    values
        .iter()
        .enumerate()
        .fold(None, |best: Option<(usize, f32)>, (index, value)| match best {
            Some((_, top)) if top >= *value => best,
            _ => Some((index, *value)),
        })
        .map(|(index, _)| index as i32)
        .ok_or_else(|| failed("the decoder wrote no logits"))
}

impl Transcriber for Moonshine {
    fn transcribe(&mut self, samples: &[f32]) -> Result<(String, Steps), VoiceEngineError> {
        let mut steps = Steps::default();
        if samples.len() < 1_600 {
            // Under 0.1 s: nothing was said.
            return Ok((String::new(), steps));
        }
        let started = Instant::now();
        let audio = Tensor::from_array((vec![1i64, samples.len() as i64], samples.to_vec()))
            .map_err(failed)?;
        let mut features = run(&mut self.preprocess, vec![SessionInputValue::from(audio)])?;
        let features = features.drain(..).next().ok_or_else(|| failed("no features"))?;
        let frames = features.shape().get(1).copied().unwrap_or(0);
        steps.preprocess_ms = started.elapsed().as_secs_f64() * 1000.0;

        let started = Instant::now();
        let mut encoded = run(
            &mut self.encode,
            vec![
                SessionInputValue::from(features),
                int_tensor(vec![1], vec![frames as i32])?,
            ],
        )?;
        let encoder_out = encoded.drain(..).next().ok_or_else(|| failed("no encoding"))?;
        let encoder_frames = encoder_out.shape().get(1).copied().unwrap_or(0).max(0);
        steps.encode_ms = started.elapsed().as_secs_f64() * 1000.0;

        // sherpa-onnx's bound: about six tokens a second of audio.
        let max_tokens = ((encoder_frames as f64 * 384.0 / 16_000.0) * 6.0).ceil() as usize;
        let started = Instant::now();
        let mut ids: Vec<i32> = Vec::new();
        let mut outputs = run(
            &mut self.uncached_decode,
            vec![
                int_tensor(vec![1, 1], vec![tokens::START])?,
                SessionInputValue::from(&encoder_out),
                int_tensor(vec![1], vec![1])?,
            ],
        )?;
        for _ in 0..max_tokens.max(1) {
            if outputs.is_empty() {
                return Err(failed("the decoder returned nothing"));
            }
            let token = best_token(&outputs[0])?;
            if token == tokens::END {
                break;
            }
            ids.push(token);
            let states = outputs.drain(1..);
            let mut inputs = vec![
                int_tensor(vec![1, 1], vec![token])?,
                SessionInputValue::from(&encoder_out),
                int_tensor(vec![1], vec![ids.len() as i32 + 1])?,
            ];
            inputs.extend(states.map(SessionInputValue::from));
            outputs = run(&mut self.cached_decode, inputs)?;
        }
        steps.decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        steps.tokens = ids.len() as u32;
        Ok((self.vocabulary.decode(&ids), steps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_versions_parse() {
        assert_eq!(parse_version("1.23.2"), Some(vec![1, 23, 2]));
        assert_eq!(parse_version("1.23"), Some(vec![1, 23]));
        assert_eq!(parse_version("1.23.2.dbg"), None);
        assert!(vec![1, 24] > vec![1, 23, 2]);
    }
}
