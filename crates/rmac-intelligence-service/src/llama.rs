//! The llama.cpp engine, with the prompt-prefix cache (ADR 0024 §4).
//!
//! Qwen3.5 is a hybrid model: its Gated DeltaNet layers keep a recurrent
//! state per sequence that only moves forward, so a context cannot be
//! rewound to "just after the system prompt" by trimming the KV cache
//! (phase 0 found that reusing a context across prompts fails outright).
//! Instead:
//!
//! 1. at load, the fixed prefix (system prompt, schema, examples) is
//!    evaluated once on sequence 0 — or read back from a state file saved by
//!    an earlier run of the same model and prompt;
//! 2. the whole sequence state (attention KV *and* recurrent state) is
//!    captured with `llama_state_seq_get_data_ext`;
//! 3. every request clears the context, restores that state into sequence
//!    0 (`llama_state_seq_set_data_ext`), and evaluates only its own tokens
//!    from the prefix's end position.
//!
//! The model and backend are leaked on purpose: the service never unloads
//! without exiting, and exiting returns everything to the system.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::{LlamaStateSeqFlags, SeqState};
use rmac_intelligence::decode::{self, Tables, Token, Vocabulary};
use rmac_intelligence::manifest::Tier;
use rmac_intelligence::prompt::{self, PromptStyle, ANSWER_PREFIX};
use rmac_intelligence::Intent;
use sha2::{Digest, Sha256};

use crate::engine::{Calibration, Engine, EngineError, IntentOutcome, LoadReport, Timing};

/// Prefix plus the longest request and answer, with room to spare.
const CONTEXT_TOKENS: u32 = 1536;
const BATCH_TOKENS: usize = 1024;
const SEQUENCE: i32 = 0;

#[derive(Clone, Debug)]
pub struct Options {
    pub threads: i32,
    pub style: PromptStyle,
    /// Where the evaluated prefix state is kept between runs; `None`
    /// evaluates it at every load.
    pub state_cache: Option<PathBuf>,
}

/// The tokenizer, with every token's text read once at load.
pub struct Pieces {
    model: &'static LlamaModel,
    pieces: Vec<Vec<u8>>,
}

impl Vocabulary for Pieces {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        self.model
            .vocab()
            .tokenize(text.as_bytes(), false, false)
            .into_iter()
            .map(|token| token.0)
            .collect()
    }

    fn piece(&self, token: Token) -> &[u8] {
        usize::try_from(token)
            .ok()
            .and_then(|index| self.pieces.get(index))
            .map_or(&[], Vec::as_slice)
    }

    fn len(&self) -> usize {
        self.pieces.len()
    }
}

pub struct LlamaEngine {
    model: &'static LlamaModel,
    context: LlamaContext<'static>,
    batch: LlamaBatch<'static>,
    pieces: Pieces,
    tables: Tables,
    style: PromptStyle,
    tier: Tier,
    prefix: SeqState,
    prefix_tokens: i32,
    report: LoadReport,
    threads: i32,
    /// Set once the first request has been answered.
    warm: bool,
}

fn failed(error: impl std::fmt::Display) -> EngineError {
    EngineError::Failed(error.to_string())
}

impl LlamaEngine {
    pub fn load(path: &Path, tier: Tier, options: Options) -> Result<Self, EngineError> {
        let started = Instant::now();
        let mut backend = LlamaBackend::init().map_err(failed)?;
        backend.void_logs();
        let backend: &'static LlamaBackend = Box::leak(Box::new(backend));
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default())
            .map_err(failed)?;
        let model: &'static LlamaModel = Box::leak(Box::new(model));
        let parameters = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(CONTEXT_TOKENS))
            .with_n_batch(BATCH_TOKENS as u32)
            .with_n_threads(options.threads)
            .with_n_threads_batch(options.threads);
        let mut context = model.new_context(backend, parameters).map_err(failed)?;
        let vocab = model.vocab();
        let pieces = Pieces {
            model,
            pieces: (0..model.n_vocab())
                .map(|token| vocab.token_to_piece(LlamaToken(token), false, None))
                .collect(),
        };
        let tables = Tables::new(&pieces);
        let model_load_ms = started.elapsed().as_secs_f64() * 1000.0;

        let prefix_text = prompt::prefix(options.style);
        let prefix_tokens = vocab.tokenize(prefix_text.as_bytes(), true, true);
        let prefix_started = Instant::now();
        let state_file = options
            .state_cache
            .as_deref()
            .map(|directory| state_file(directory, path, &prefix_text, options.style));
        let restored = state_file
            .as_deref()
            .is_some_and(|file| restore(&mut context, file, &prefix_tokens));
        let mut batch = LlamaBatch::new(BATCH_TOKENS, 1);
        if !restored {
            context.clear_kv_cache();
            feed(&mut context, &mut batch, &prefix_tokens, 0)?;
            if let Some(file) = &state_file {
                save(&context, file, &prefix_tokens);
            }
        }
        let prefix = context
            .state_seq_get(SEQUENCE, LlamaStateSeqFlags::empty())
            .map_err(failed)?;
        let report = LoadReport {
            model_load_ms,
            prefix_tokens: prefix_tokens.len() as u32,
            prefix_restored: restored,
            prefix_ms: prefix_started.elapsed().as_secs_f64() * 1000.0,
        };
        Ok(Self {
            model,
            context,
            batch,
            pieces,
            tables,
            style: options.style,
            tier,
            prefix,
            prefix_tokens: prefix_tokens.len() as i32,
            report,
            threads: options.threads,
            warm: false,
        })
    }

    pub fn threads(&self) -> i32 {
        self.threads
    }

    /// Back to "just after the prefix".
    fn rewind(&mut self) -> Result<(), EngineError> {
        self.context.clear_kv_cache();
        self.context
            .state_seq_set(&self.prefix, SEQUENCE)
            .map_err(failed)
    }

    /// Evaluate the request's tokens after the prefix; returns the logits
    /// for the first answer token and the request's token count.
    fn start_request(&mut self, text: &str) -> Result<(Vec<f32>, usize), EngineError> {
        self.rewind()?;
        let request = prompt::request(self.style, text);
        let tokens = self.model.vocab().tokenize(request.as_bytes(), false, true);
        let logits = feed(
            &mut self.context,
            &mut self.batch,
            &tokens,
            self.prefix_tokens,
        )?;
        Ok((logits, tokens.len()))
    }

    /// The same request decoded with llama.cpp's own GBNF grammar sampler,
    /// one token per forward pass: the comparison `rmac-intelligence-bench
    /// --decoder gbnf` reports against the schema-guided decoder.
    pub fn intent_gbnf(&mut self, text: &str) -> Result<IntentOutcome, EngineError> {
        let received = Instant::now();
        let (_, request_tokens) = self.start_request(text)?;
        let first_token_ms = received.elapsed().as_secs_f64() * 1000.0;
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::grammar(self.model, GBNF, "root").map_err(failed)?,
            LlamaSampler::greedy(),
        ]);
        let start = self.prefix_tokens + request_tokens as i32;
        let mut json = ANSWER_PREFIX.to_owned();
        let mut passes = 0;
        for position in start..start + 96 {
            // `sample` also accepts the token into the grammar.
            let token = sampler.sample(&self.context, self.batch.n_tokens() - 1);
            if self.model.vocab().is_eog(token) {
                break;
            }
            json.push_str(&String::from_utf8_lossy(self.pieces.piece(token.0)));
            self.batch.clear();
            self.batch
                .add(token, position, &[SEQUENCE], true)
                .map_err(failed)?;
            self.context.decode(&mut self.batch).map_err(failed)?;
            passes += 1;
        }
        let intent = Intent::parse(&json).map_err(|error| failed(format!("{error}: {json}")))?;
        Ok(IntentOutcome {
            intent,
            json,
            timing: Timing {
                first_token_ms,
                total_ms: received.elapsed().as_secs_f64() * 1000.0,
                cold: false,
                cached_prefix_tokens: self.prefix_tokens as u32,
                request_tokens: request_tokens as u32,
                passes,
            },
        })
    }
}

/// The schema as GBNF, after the answer prefix `{"intent":"`. Equivalent to
/// the schema-guided decoder except that it cannot bound a timer by its
/// unit (the strict parser rejects more than 23 hours afterwards).
pub const GBNF: &str = r#"root ::= "open_app\",\"app\":\"" text "\"}" | "appearance\",\"mode\":\"" ("dark" | "light") "\"}" | "volume\",\"" ("level\":" pct "}" | "change\":\"" ("up" | "down" | "mute" | "unmute") "\"}") | "brightness\",\"" ("level\":" pct "}" | "change\":\"" ("up" | "down") "\"}") | ("wifi" | "bluetooth" | "do_not_disturb") "\",\"on\":" ("true" | "false") "}" | "timer\",\"amount\":" amount ",\"unit\":\"" ("seconds" | "minutes" | "hours") "\"}" | "search_files\",\"query\":\"" text "\"}" | "none\"}"
text ::= [^"\\\n\r\t] [^"\\\n\r\t]*
pct ::= "100" | [1-9] [0-9] | [0-9]
amount ::= [1-9] [0-9] [0-9] | [1-9] [0-9] | [1-9]
"#;

/// Feed `tokens` from `position`, in batches, asking for logits only at the
/// last one; returns those logits.
fn feed(
    context: &mut LlamaContext<'static>,
    batch: &mut LlamaBatch<'static>,
    tokens: &[LlamaToken],
    position: i32,
) -> Result<Vec<f32>, EngineError> {
    let mut position = position;
    let chunks = tokens.chunks(BATCH_TOKENS);
    let count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        batch.clear();
        for (offset, token) in chunk.iter().enumerate() {
            let last = index + 1 == count && offset + 1 == chunk.len();
            batch
                .add(*token, position, &[SEQUENCE], last)
                .map_err(failed)?;
            position += 1;
        }
        context.decode(batch).map_err(failed)?;
    }
    Ok(context.get_logits_ith(batch.n_tokens() - 1).to_vec())
}

/// The running model as the decoder sees it.
struct Stepper<'a> {
    context: &'a mut LlamaContext<'static>,
    batch: &'a mut LlamaBatch<'static>,
    position: i32,
}

impl decode::Model for Stepper<'_> {
    fn feed(&mut self, tokens: &[Token]) -> Result<Vec<f32>, String> {
        let tokens: Vec<LlamaToken> = tokens.iter().map(|token| LlamaToken(*token)).collect();
        let logits = feed(self.context, self.batch, &tokens, self.position)
            .map_err(|error| error.to_string())?;
        self.position += tokens.len() as i32;
        Ok(logits)
    }
}

impl Engine for LlamaEngine {
    fn intent(&mut self, text: &str, received: Instant) -> Result<IntentOutcome, EngineError> {
        let cold = !self.warm;
        let (logits, request_tokens) = self.start_request(text)?;
        let first_token_ms = received.elapsed().as_secs_f64() * 1000.0;
        let mut stepper = Stepper {
            context: &mut self.context,
            batch: &mut self.batch,
            position: self.prefix_tokens + request_tokens as i32,
        };
        let decoded =
            decode::decode(&mut stepper, &self.pieces, &self.tables, logits).map_err(failed)?;
        self.warm = true;
        Ok(IntentOutcome {
            intent: decoded.intent,
            json: decoded.json,
            timing: Timing {
                first_token_ms,
                total_ms: received.elapsed().as_secs_f64() * 1000.0,
                cold,
                cached_prefix_tokens: self.prefix_tokens as u32,
                request_tokens: request_tokens as u32,
                passes: decoded.passes,
            },
        })
    }

    /// Decode 32 tokens one at a time (greedy, unconstrained) after a short
    /// request: the decode rate the hardware gate compares with its floor.
    fn calibrate(&mut self) -> Result<Calibration, EngineError> {
        let started = Instant::now();
        let (mut logits, request_tokens) =
            self.start_request("write a short sentence about the sea")?;
        let prefill_tok_s = request_tokens as f64 / started.elapsed().as_secs_f64().max(1e-6);
        let start = self.prefix_tokens + request_tokens as i32;
        let steps = 32;
        let decode_started = Instant::now();
        for position in start..start + steps {
            let token = logits
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| LlamaToken(index as i32))
                .ok_or_else(|| failed("no logits"))?;
            logits = feed(&mut self.context, &mut self.batch, &[token], position)?;
        }
        let decode_tok_s = f64::from(steps) / decode_started.elapsed().as_secs_f64().max(1e-6);
        Ok(Calibration {
            tier: self.tier.as_str().into(),
            decode_tok_s,
            prefill_tok_s,
        })
    }

    fn load_report(&self) -> LoadReport {
        self.report.clone()
    }
}

/// The state file for one model, prompt and context shape. Any change to
/// them (or to llama.cpp, by crate version) names a different file.
fn state_file(directory: &Path, model: &Path, prefix: &str, style: PromptStyle) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(model.as_os_str().as_encoded_bytes());
    hasher.update(prompt::PROMPT_VERSION.to_le_bytes());
    hasher.update(style.as_str().as_bytes());
    hasher.update(prefix.as_bytes());
    hasher.update(CONTEXT_TOKENS.to_le_bytes());
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"llama-cpp-2 0.1.158");
    let digest = rmac_intelligence::verify::hex(&hasher.finalize());
    directory.join(format!("prefix-{}.state", &digest[..24]))
}

/// Read a saved prefix state; true only if it holds exactly this prefix.
fn restore(context: &mut LlamaContext<'static>, file: &Path, tokens: &[LlamaToken]) -> bool {
    if !file.is_file() {
        return false;
    }
    context.clear_kv_cache();
    match context.state_seq_load_file(file, SEQUENCE, tokens.len() + 1) {
        Ok((saved, _)) if saved == tokens => true,
        _ => {
            context.clear_kv_cache();
            let _ = std::fs::remove_file(file);
            false
        }
    }
}

fn save(context: &LlamaContext<'static>, file: &Path, tokens: &[LlamaToken]) {
    let Some(directory) = file.parent() else {
        return;
    };
    if rmac_storage_dir(directory).is_err() {
        return;
    }
    let temporary = file.with_extension(format!("state.{}", std::process::id()));
    if context
        .state_seq_save_file(&temporary, SEQUENCE, tokens)
        .is_ok()
    {
        let _ = std::fs::rename(&temporary, file);
    } else {
        let _ = std::fs::remove_file(&temporary);
    }
}

fn rmac_storage_dir(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
