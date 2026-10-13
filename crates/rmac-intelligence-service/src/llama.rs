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
//! Writing Tools tasks work the same way, each with its own prefix
//! (`rmac_intelligence::writing`): its state is read from its own file, or
//! evaluated and saved the first time the task is used, and the most
//! recent one is kept in memory so Try Again starts at once. The answer is
//! sampled token by token and streamed to the caller as it is written.
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
use llama_cpp_2::token::logit_bias::LlamaLogitBias;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::{LlamaStateSeqFlags, SeqState};
use rmac_intelligence::decode::{self, Tables, Token, Vocabulary};
use rmac_intelligence::manifest::Tier;
use rmac_intelligence::prompt::{self, PromptStyle, ANSWER_PREFIX};
use rmac_intelligence::{guard, prefix_state, writing, Intent, Task};

use crate::engine::{
    Calibration, Engine, EngineError, IntentOutcome, LoadReport, Timing, WriteOutcome,
    WriteRequest, WriteTiming,
};

/// Prefix plus the longest request and answer, with room to spare.
const CONTEXT_TOKENS: u32 = prefix_state::CONTEXT_TOKENS;
const BATCH_TOKENS: usize = 1024;
const SEQUENCE: i32 = 0;

#[derive(Clone, Debug)]
pub struct Options {
    pub threads: i32,
    pub style: PromptStyle,
    /// Where the evaluated prefix state is kept between runs; `None`
    /// evaluates it at every load.
    pub state_cache: Option<PathBuf>,
    /// Apply [`guard::check`] to every answer (off only to measure the
    /// model alone).
    pub guard: bool,
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
    /// Each token's non-Latin scripts ([`writing::foreign_scripts`]).
    token_scripts: Vec<u8>,
    style: PromptStyle,
    tier: Tier,
    prefix: SeqState,
    prefix_tokens: i32,
    report: LoadReport,
    threads: i32,
    guard: bool,
    /// How long the last request's restore of the prefix state took.
    last_rewind_ms: f64,
    /// Set once the first request has been answered.
    warm: bool,
    state_cache: Option<PathBuf>,
    /// The last Writing Tools task's prefix state and its length.
    writing: Option<(Task, SeqState, i32)>,
}

/// How a writing task's prefix became ready for one request.
struct WritingPrefix {
    tokens: i32,
    ms: f64,
    evaluated: bool,
}

/// Threads for prompt processing (a request's own text). The same as for
/// decoding unless `RMAC_INTELLIGENCE_BATCH_THREADS` says otherwise, for
/// measuring on the reference laptop.
fn batch_threads(threads: i32) -> i32 {
    std::env::var("RMAC_INTELLIGENCE_BATCH_THREADS")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|count| (1..=16).contains(count))
        .unwrap_or(threads)
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
            .with_n_threads_batch(batch_threads(options.threads));
        let mut context = model.new_context(backend, parameters).map_err(failed)?;
        let vocab = model.vocab();
        let pieces = Pieces {
            model,
            pieces: (0..model.n_vocab())
                .map(|token| vocab.token_to_piece(LlamaToken(token), false, None))
                .collect(),
        };
        let tables = Tables::new(&pieces);
        let token_scripts = pieces
            .pieces
            .iter()
            .map(|piece| writing::foreign_scripts(piece))
            .collect();
        let model_load_ms = started.elapsed().as_secs_f64() * 1000.0;

        let prefix_text = prompt::prefix(options.style);
        let prefix_tokens = vocab.tokenize(prefix_text.as_bytes(), true, true);
        let prefix_started = Instant::now();
        let state_file = options
            .state_cache
            .as_deref()
            .map(|directory| prefix_state::file(directory, tier.model(), options.style));
        let restored = state_file
            .as_deref()
            .is_some_and(|file| restore(&mut context, file, &prefix_tokens));
        let mut batch = LlamaBatch::new(BATCH_TOKENS, 1);
        if !restored {
            context.clear_kv_cache();
            feed(&mut context, &mut batch, &prefix_tokens, 0)?;
            if let (Some(file), Some(directory)) = (&state_file, &options.state_cache) {
                let keep = prefix_state::current_files(directory, tier.model(), options.style);
                save(&context, file, &prefix_tokens, &keep);
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
            token_scripts,
            style: options.style,
            tier,
            prefix,
            prefix_tokens: prefix_tokens.len() as i32,
            report,
            threads: options.threads,
            guard: options.guard,
            last_rewind_ms: 0.0,
            warm: false,
            state_cache: options.state_cache,
            writing: None,
        })
    }

    /// Put the context at "just after `task`'s prefix": from memory when it
    /// was the last writing task, else from its saved state file, else by
    /// evaluating it once (and saving it for the next run).
    fn writing_prefix(&mut self, task: Task) -> Result<WritingPrefix, EngineError> {
        let started = Instant::now();
        let spec = writing::spec(task).ok_or_else(|| failed("not a writing task"))?;
        self.context.clear_kv_cache();
        if let Some((cached, state, tokens)) = &self.writing {
            if *cached == task {
                let tokens = *tokens;
                self.context
                    .state_seq_set(state, SEQUENCE)
                    .map_err(failed)?;
                return Ok(WritingPrefix {
                    tokens,
                    ms: started.elapsed().as_secs_f64() * 1000.0,
                    evaluated: false,
                });
            }
        }
        // Only one writing state is held in memory (each is tens of MB).
        self.writing = None;
        let prefix_tokens = self
            .model
            .vocab()
            .tokenize(spec.prefix().as_bytes(), true, true);
        let file = self
            .state_cache
            .as_deref()
            .and_then(|directory| prefix_state::writing_file(directory, self.tier.model(), task));
        let restored = file
            .as_deref()
            .is_some_and(|file| restore(&mut self.context, file, &prefix_tokens));
        if !restored {
            self.context.clear_kv_cache();
            feed(&mut self.context, &mut self.batch, &prefix_tokens, 0)?;
            if let (Some(file), Some(directory)) = (&file, &self.state_cache) {
                let keep = prefix_state::current_files(directory, self.tier.model(), self.style);
                save(&self.context, file, &prefix_tokens, &keep);
            }
        }
        let state = self
            .context
            .state_seq_get(SEQUENCE, LlamaStateSeqFlags::empty())
            .map_err(failed)?;
        let tokens = prefix_tokens.len() as i32;
        self.writing = Some((task, state, tokens));
        Ok(WritingPrefix {
            tokens,
            ms: started.elapsed().as_secs_f64() * 1000.0,
            evaluated: !restored,
        })
    }

    /// The sampler for one answer. Greedy tasks give the same answer for the
    /// same text; Try Again on a greedy task samples gently instead. Every
    /// token that starts a character in a script `text` does not use is
    /// forbidden ([`writing::foreign_scripts`]), so the answer stays in the
    /// text's language.
    fn sampler(&self, mut sampling: writing::Sampling, attempt: u32, text: &str) -> LlamaSampler {
        let allowed = writing::foreign_scripts(text.as_bytes());
        let banned: Vec<LlamaLogitBias> = self
            .token_scripts
            .iter()
            .enumerate()
            .filter(|(_, scripts)| **scripts & !allowed != 0)
            .map(|(token, _)| LlamaLogitBias::new(LlamaToken(token as i32), f32::NEG_INFINITY))
            .collect();
        let language = LlamaSampler::logit_bias(self.model.n_vocab(), &banned);
        if sampling.temperature == 0.0 && attempt > 0 {
            sampling.temperature = 0.3;
            sampling.top_k = 20;
            sampling.top_p = 0.9;
        }
        if sampling.temperature == 0.0 {
            return LlamaSampler::chain_simple([language, LlamaSampler::greedy()]);
        }
        let seed = 0x5eed_u32.wrapping_add(attempt.wrapping_mul(7919));
        LlamaSampler::chain_simple([
            language,
            LlamaSampler::penalties(self.model.n_vocab(), 64, sampling.repeat_penalty, 0.0, 0.0),
            LlamaSampler::top_k(sampling.top_k),
            LlamaSampler::top_p(sampling.top_p, 1),
            LlamaSampler::temp(sampling.temperature),
            LlamaSampler::dist(seed),
        ])
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
        let started = Instant::now();
        self.rewind()?;
        self.last_rewind_ms = started.elapsed().as_secs_f64() * 1000.0;
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
        if self.style.syntax() != decode::Syntax::Json {
            return Err(failed(
                "the GBNF comparison needs a JSON prompt style (--style list)",
            ));
        }
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
        let intent = if self.guard {
            guard::check(text, intent)
        } else {
            intent
        };
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
                rewind_ms: self.last_rewind_ms,
                ..Timing::default()
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

/// What a [`Stepper`] reports when the request was superseded.
const CANCELLED: &str = "cancelled";

/// The running model as the decoder sees it.
struct Stepper<'a> {
    context: &'a mut LlamaContext<'static>,
    batch: &'a mut LlamaBatch<'static>,
    position: i32,
    /// Checked before every forward pass: a newer request from the same
    /// caller stops this one at the next pass.
    cancelled: &'a dyn Fn() -> bool,
}

impl decode::Model for Stepper<'_> {
    fn feed(&mut self, tokens: &[Token]) -> Result<Vec<f32>, String> {
        if (self.cancelled)() {
            return Err(CANCELLED.into());
        }
        let tokens: Vec<LlamaToken> = tokens.iter().map(|token| LlamaToken(*token)).collect();
        let logits = feed(self.context, self.batch, &tokens, self.position)
            .map_err(|error| error.to_string())?;
        self.position += tokens.len() as i32;
        Ok(logits)
    }
}

impl Engine for LlamaEngine {
    fn intent(
        &mut self,
        text: &str,
        received: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<IntentOutcome, EngineError> {
        if cancelled() {
            return Err(EngineError::Cancelled);
        }
        let cold = !self.warm;
        let started = Instant::now();
        let queued_ms = started.duration_since(received).as_secs_f64() * 1000.0;
        let (logits, request_tokens) = self.start_request(text)?;
        let first_token_ms = received.elapsed().as_secs_f64() * 1000.0;
        let prefill_ms = started.elapsed().as_secs_f64() * 1000.0 - self.last_rewind_ms;
        let decode_started = Instant::now();
        let mut stepper = Stepper {
            context: &mut self.context,
            batch: &mut self.batch,
            position: self.prefix_tokens + request_tokens as i32,
            cancelled,
        };
        let decoded = decode::decode(
            &mut stepper,
            &self.pieces,
            &self.tables,
            logits,
            self.style.syntax(),
        )
        .map_err(|error| match error {
            decode::DecodeError::Model(detail) if detail == CANCELLED => EngineError::Cancelled,
            other => failed(other),
        })?;
        let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
        self.warm = true;
        let intent = if self.guard {
            guard::check(text, decoded.intent)
        } else {
            decoded.intent
        };
        Ok(IntentOutcome {
            intent,
            json: decoded.json,
            timing: Timing {
                first_token_ms,
                total_ms: received.elapsed().as_secs_f64() * 1000.0,
                cold,
                cached_prefix_tokens: self.prefix_tokens as u32,
                request_tokens: request_tokens as u32,
                passes: decoded.passes,
                queued_ms,
                rewind_ms: self.last_rewind_ms,
                prefill_ms,
                decode_ms,
            },
        })
    }

    fn write(
        &mut self,
        request: &WriteRequest<'_>,
        sink: &mut dyn FnMut(&str),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<WriteOutcome, EngineError> {
        let spec = writing::spec(request.task).ok_or_else(|| failed("not a writing task"))?;
        if !spec.offered_on(self.tier) {
            return Err(EngineError::NeedsFasterPc);
        }
        // The text never becomes special tokens: "<|im_end|>" in a note is
        // just characters. Only the template around it is parsed.
        let vocab = self.model.vocab();
        let input = vocab.tokenize(request.text.as_bytes(), false, false);
        if input.len() as u32 > spec.max_input_tokens {
            return Err(EngineError::TooLong);
        }
        if cancelled() {
            return Err(EngineError::Cancelled);
        }
        let cold = !self.warm;
        let started = Instant::now();
        let queued_ms = started.duration_since(request.received).as_secs_f64() * 1000.0;
        let prefix = self.writing_prefix(request.task)?;
        let mut tokens = input.clone();
        tokens.extend(
            self.model
                .vocab()
                .tokenize(spec.request_suffix().as_bytes(), false, true),
        );
        let input_tokens = input.len() as u32;
        let room = CONTEXT_TOKENS as i32 - prefix.tokens - tokens.len() as i32 - 1;
        let most = (spec.max_output_tokens(input_tokens) as i32).min(room);
        if most < 16 {
            return Err(EngineError::TooLong);
        }
        let prefill_started = Instant::now();
        feed(&mut self.context, &mut self.batch, &tokens, prefix.tokens)?;
        let prefill_ms = prefill_started.elapsed().as_secs_f64() * 1000.0;

        let mut sampler = self.sampler(spec.sampling, request.attempt, request.text);
        let mut stream = writing::Utf8Stream::default();
        let mut raw = spec.answer_start.to_owned();
        if !raw.is_empty() {
            sink(&raw);
        }
        let mut position = prefix.tokens + tokens.len() as i32;
        let mut first_token_ms = 0.0;
        let mut decode_started = Instant::now();
        let mut output_tokens = 0u32;
        let mut capped = true;
        for _ in 0..most {
            // `sample` also accepts the token into the penalty sampler.
            let token = sampler.sample(&self.context, self.batch.n_tokens() - 1);
            if output_tokens == 0 {
                first_token_ms = request.received.elapsed().as_secs_f64() * 1000.0;
                decode_started = Instant::now();
            }
            output_tokens += 1;
            if self.model.vocab().is_eog(token) {
                capped = false;
                break;
            }
            let piece = stream.push(self.pieces.piece(token.0));
            if !piece.is_empty() {
                let before = raw.len();
                raw.push_str(&piece);
                // The model starting a new turn in plain text ends it too.
                if let Some(stop) = writing::stop_marker(&raw) {
                    raw.truncate(stop);
                    if raw.len() > before {
                        sink(&raw[before..]);
                    }
                    capped = false;
                    break;
                }
                sink(&piece);
            }
            if cancelled() {
                return Err(EngineError::Cancelled);
            }
            self.batch.clear();
            self.batch
                .add(token, position, &[SEQUENCE], true)
                .map_err(failed)?;
            self.context.decode(&mut self.batch).map_err(failed)?;
            position += 1;
        }
        let tail = stream.finish();
        if !tail.is_empty() {
            raw.push_str(&tail);
            sink(&tail);
        }
        self.warm = true;
        let decode_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
        let decode_tok_s = if output_tokens > 1 {
            f64::from(output_tokens - 1) * 1000.0 / decode_ms.max(1e-3)
        } else {
            0.0
        };
        Ok(WriteOutcome {
            text: writing::finish(request.task, request.text, &raw),
            raw,
            capped,
            timing: WriteTiming {
                first_token_ms,
                total_ms: request.received.elapsed().as_secs_f64() * 1000.0,
                cold,
                queued_ms,
                prefix_ms: prefix.ms,
                prefix_evaluated: prefix.evaluated,
                prefix_tokens: prefix.tokens as u32,
                input_tokens,
                prefill_ms,
                output_tokens,
                decode_ms,
                decode_tok_s,
            },
        })
    }

    fn tier(&self) -> Tier {
        self.tier
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

fn save(context: &LlamaContext<'static>, file: &Path, tokens: &[LlamaToken], keep: &[PathBuf]) {
    let Some(directory) = file.parent() else {
        return;
    };
    if rmac_storage_dir(directory).is_err() {
        return;
    }
    let temporary = file.with_extension(format!("state.{}", std::process::id()));
    // Owner-only whatever the umask (the unit sets 0077; a service started
    // some other way may not): create the file 0600 before llama.cpp opens
    // it for writing, which keeps the mode.
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        if options.open(&temporary).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
        }
    }
    if context
        .state_seq_save_file(&temporary, SEQUENCE, tokens)
        .is_ok()
    {
        if std::fs::rename(&temporary, file).is_ok() {
            // An update changed a prompt or the model: the older states
            // can never be restored again. Every current task's is kept.
            prefix_state::remove_others(directory, keep);
        }
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
