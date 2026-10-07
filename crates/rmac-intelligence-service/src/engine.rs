//! The model behind the service, as one small trait (ADR 0024 §3: llama.cpp
//! today; candle or mistral.rs could replace it).

use std::fmt;
use std::time::Instant;

use rmac_intelligence::config::Config;
use rmac_intelligence::gate::{self, Decision};
use rmac_intelligence::manifest::Tier;
use rmac_intelligence::{paths, verify, Intent};
use serde::Serialize;

/// What one intent request cost. Field names match
/// `rmac_intelligence::client::Timing`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Timing {
    pub first_token_ms: f64,
    pub total_ms: f64,
    pub cold: bool,
    pub cached_prefix_tokens: u32,
    pub request_tokens: u32,
    pub passes: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntentOutcome {
    pub intent: Intent,
    /// The JSON the model wrote.
    pub json: String,
    pub timing: Timing,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Calibration {
    pub tier: String,
    pub decode_tok_s: f64,
    pub prefill_tok_s: f64,
}

/// How the model and its prompt state became ready.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LoadReport {
    pub model_load_ms: f64,
    pub prefix_tokens: u32,
    /// Prefix state read back from disk instead of evaluated.
    pub prefix_restored: bool,
    pub prefix_ms: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// The user has not turned Lulo Intelligence on.
    Off,
    /// This PC cannot run it (hardware gate), or no model runtime here.
    NotSupported(String),
    NotDownloaded,
    LowMemory,
    Failed(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => formatter.write_str("Lulo Intelligence is turned off"),
            Self::NotSupported(reason) => formatter.write_str(reason),
            Self::NotDownloaded => formatter.write_str("the model is not downloaded"),
            Self::LowMemory => formatter.write_str("not enough free memory right now"),
            Self::Failed(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for EngineError {}

pub trait Engine {
    /// The intent for one request. `received` is when the request arrived,
    /// so the timings include any queueing.
    fn intent(&mut self, text: &str, received: Instant) -> Result<IntentOutcome, EngineError>;
    fn calibrate(&mut self) -> Result<Calibration, EngineError>;
    fn load_report(&self) -> LoadReport;
}

/// Which engine `open` builds. The fixture engine is a deterministic
/// stand-in for behaviour tests (`RMAC_INTELLIGENCE_ENGINE=fixture`); it
/// still honours the user's on/off setting.
pub fn fixture_requested() -> bool {
    std::env::var("RMAC_INTELLIGENCE_ENGINE").is_ok_and(|value| value == "fixture")
}

/// The user's setting, re-read on every request so turning Lulo
/// Intelligence off takes effect at once.
pub fn enabled() -> bool {
    Config::load().enabled
}

/// Build the engine for this PC: check the setting, the hardware gate,
/// free memory and the model's checksum, then load it.
pub fn open() -> Result<Box<dyn Engine>, EngineError> {
    let config = Config::load();
    if !config.enabled {
        return Err(EngineError::Off);
    }
    if fixture_requested() {
        return Ok(Box::new(crate::fixture::FixtureEngine::default()));
    }
    let facts = gate::current_facts();
    let decision = config.decision(&facts);
    if let Decision::NotOffered(reason) = &decision {
        return Err(EngineError::NotSupported(reason.clone()));
    }
    let tier = config.tier(&facts).unwrap_or(Tier::Tiny);
    let model = tier.model();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    if gate::memory_available_mib(&meminfo)
        .is_some_and(|available| available < gate::memory_needed_mib(model.memory_budget_mib))
    {
        return Err(EngineError::LowMemory);
    }
    let models = paths::models_dir().ok_or(EngineError::NotDownloaded)?;
    let path = verify::verified_model(&models, model).map_err(|error| match error {
        verify::VerifyError::Missing => EngineError::NotDownloaded,
        other => EngineError::Failed(other.to_string()),
    })?;
    load_llama(&path, tier, threads_for(&facts))
}

/// Physical cores only (ADR 0024 §4), at most four.
pub fn threads_for(facts: &gate::Facts) -> i32 {
    facts.physical_cores.clamp(1, 4) as i32
}

#[cfg(target_os = "linux")]
fn load_llama(
    path: &std::path::Path,
    tier: Tier,
    threads: i32,
) -> Result<Box<dyn Engine>, EngineError> {
    let engine = crate::llama::LlamaEngine::load(
        path,
        tier,
        crate::llama::Options {
            threads,
            style: rmac_intelligence::prompt::PromptStyle::DEFAULT,
            state_cache: paths::cache_dir(),
        },
    )?;
    Ok(Box::new(engine))
}

#[cfg(not(target_os = "linux"))]
fn load_llama(
    _path: &std::path::Path,
    _tier: Tier,
    _threads: i32,
) -> Result<Box<dyn Engine>, EngineError> {
    Err(EngineError::NotSupported(
        "Lulo Intelligence runs on Linux".into(),
    ))
}
