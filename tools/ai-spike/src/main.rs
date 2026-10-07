//! ADR 0024 phase 0 measurement spike.
//!
//! STATUS: built and run on the reference laptop (jacob@192.168.18.52) on
//! 2026-10-07 once disk was freed (114 GB free at the start). Results are
//! recorded in `docs/decisions/0024-lulo-intelligence.md` ("Phase 0
//! results"), including methodology caveats worth reading before trusting
//! any number this binary prints: in particular, every prompt starts with
//! `ctx.clear_kv_cache()` (required - see that section's architecture
//! note on Qwen3.5's recurrent memory layers), so "warm" here means
//! "repeated full prefill, no prompt-prefix cache", not the ADR §4 design's
//! cached-prefix warm path. Built with llama-cpp-2 0.1.158 against the
//! llama-cpp-2 API documented on docs.rs and the upstream `examples/simple`
//! example (github.com/utilityai/llama-cpp-rs); CMake was not preinstalled
//! on the laptop and there is no sudo, so the build used a portable
//! `cmake` binary unpacked under `~/rmac-ai-spike/tools/` (not a system
//! install) - see the ADR section for the exact version and checksum.
//!
//! Subcommands (see `print_usage`):
//!   bench    <model.gguf> [threads] [runs]   - the three realistic prompts
//!   quality  <model.gguf>                    - the 20-case intent test
//!   unload   <model.gguf>                    - peak RSS, then RSS after drop
//!
//! Scope note: this binary is a one-shot CLI, not the real D-Bus service
//! (ADR §4). "RSS and CPU after unload must return to 0, process exit" is a
//! service-level budget; here it is approximated by measuring RSS right
//! after the model/context/backend are dropped, then letting the process
//! exit normally (exit code checked by the caller), which is the strongest
//! thing a one-shot binary can show.

use std::env;
use std::fs;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::{Duration, Instant};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;

/// The three ADR phase-0 prompts, used verbatim across all models and runs.
const SPOTLIGHT_SYSTEM: &str = r#"You turn one sentence into a single JSON tool call. Reply with ONLY the JSON object, no prose, matching this schema:
{"action": one of ["open_app","set_dark_mode","set_volume","set_timer","search_files","open_setting","none"],
 "args": {"app"?: string, "on"?: bool, "level"?: integer 0-100, "minutes"?: integer, "query"?: string, "pane"?: string}}
If the sentence does not map to one of these actions, reply {"action":"none","args":{}}."#;

const SPOTLIGHT_PROMPT: &str = "turn on dark mode";

const WRITING_SYSTEM: &str = "You rewrite the user's paragraph to be clearer and more concise, keeping the meaning and length roughly the same. Reply with only the rewritten paragraph.";

// Deliberately ~150 words, matching the ADR's Writing Tools spike prompt.
const WRITING_PROMPT: &str = "So basically what happened is that the printer on the third floor, the one near the window, it just stopped working again yesterday afternoon and nobody really knows why because it was working completely fine in the morning and then after lunch it just started showing this error message about a paper jam even though there is no paper jam that anyone can actually find anywhere in the machine, we checked the trays and the rollers and everything looks normal, and IT already came by once but they could not figure it out either and said they would come back today or maybe tomorrow depending on how busy they are, so in the meantime everyone on that floor has been walking down to the second floor to print things which is obviously not a great long-term solution for anyone involved.";

const TERMINAL_SYSTEM: &str = "You explain one shell command in two or three plain sentences, grounded only in what each flag does. Never suggest running it or any other command.";
const TERMINAL_PROMPT: &str = "find . -name '*.log' -mtime +7 -delete";

/// The 20-case intent quality set (ADR §1, feature 1; §6 dataset shape).
/// `expected_action` is checked against the model's JSON `action` field;
/// quality here is pass/fail on the action id alone, not every slot, which
/// matches the ADR's "intent exact match" framing closely enough for a
/// one-week spike.
struct IntentCase {
    phrase: &'static str,
    expected_action: &'static str,
}

const INTENT_CASES: &[IntentCase] = &[
    IntentCase { phrase: "turn on dark mode", expected_action: "set_dark_mode" },
    IntentCase { phrase: "switch to light mode please", expected_action: "set_dark_mode" },
    IntentCase { phrase: "open notes", expected_action: "open_app" },
    IntentCase { phrase: "launch the terminal", expected_action: "open_app" },
    IntentCase { phrase: "set volume to 40", expected_action: "set_volume" },
    IntentCase { phrase: "mute the sound", expected_action: "set_volume" },
    IntentCase { phrase: "turn it up a bit", expected_action: "set_volume" },
    IntentCase { phrase: "set a timer for 10 minutes", expected_action: "set_timer" },
    IntentCase { phrase: "remind me in 5 minutes", expected_action: "set_timer" },
    IntentCase { phrase: "find files named invoice", expected_action: "search_files" },
    IntentCase { phrase: "search for last week's photos", expected_action: "search_files" },
    IntentCase { phrase: "open bluetooth settings", expected_action: "open_setting" },
    IntentCase { phrase: "take me to the display settings", expected_action: "open_setting" },
    IntentCase { phrase: "i need to change my wallpaper", expected_action: "open_setting" },
    IntentCase { phrase: "go dark", expected_action: "set_dark_mode" },
    IntentCase { phrase: "crank the volume all the way up", expected_action: "set_volume" },
    IntentCase { phrase: "what's the capital of France", expected_action: "none" },
    IntentCase { phrase: "open the app drawer", expected_action: "open_app" },
    IntentCase { phrase: "start a 25 minute timer for my eggs", expected_action: "set_timer" },
    IntentCase { phrase: "pull up the files with 'budget' in the name", expected_action: "search_files" },
];

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        print_usage();
        std::process::exit(2);
    }
    let cmd = args[1].as_str();
    let model_path = args[2].clone();

    match cmd {
        "bench" => {
            let threads: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
            let runs: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(3);
            run_bench(&model_path, threads, runs);
        }
        "quality" => run_quality(&model_path),
        "unload" => run_unload(&model_path),
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: ai-spike <bench|quality|unload> <model.gguf> [threads] [runs]\n\
         \n\
         bench:   times the Spotlight/Writing/Terminal prompts, `runs` times each,\n\
         and prints the median of each metric as a markdown table row.\n\
         quality: runs the 20 intent cases and prints pass/20.\n\
         unload:  prints peak RSS during inference, then RSS right after the\n\
         model/context/backend are dropped (before process exit)."
    );
}

fn read_rss_kb() -> u64 {
    read_status_field("VmRSS:")
}

fn read_peak_rss_kb() -> u64 {
    read_status_field("VmHWM:")
}

fn read_status_field(key: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            return rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }
    0
}

/// Loads the backend + model once. Returns the load wall-clock time and the
/// loaded model, so callers can reuse one load across several timed prompts
/// (that is what "warm" means below: same process, same loaded model and KV
/// state, a second request).
fn load_model(backend: &LlamaBackend, path: &str) -> (LlamaModel, f64) {
    let model_params = LlamaModelParams::default();
    let t0 = Instant::now();
    let model = LlamaModel::load_from_file(backend, Path::new(path), &model_params)
        .expect("failed to load model - check the .gguf path and that it matches the manifest SHA-256");
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    (model, load_ms)
}

fn new_context<'a>(
    backend: &'a LlamaBackend,
    model: &'a LlamaModel,
    threads: i32,
) -> LlamaContext<'a> {
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(4096))
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    model
        .new_context(backend, ctx_params)
        .expect("failed to create llama context")
}

/// Runs one system+user prompt to completion (up to `max_new`), returning
/// (time to first token in ms, prefill tok/s, decode tok/s, generated text).
fn run_one(
    backend: &LlamaBackend,
    model: &LlamaModel,
    ctx: &mut LlamaContext,
    system: &str,
    user: &str,
    max_new: i32,
) -> (f64, f64, f64, String) {
    let _ = backend; // kept for symmetry with load_model's signature

    // Each call starts a fresh prompt at position 0. Qwen3.5's hybrid
    // Gated DeltaNet/attention layers keep a recurrent memory module that
    // requires strictly increasing positions per sequence, so without this
    // a second distinct prompt reusing the same context (simulating "warm")
    // fails with "inconsistent sequence positions". This does throw away
    // the cached system-prompt prefix between prompts/reps, which is the
    // one simplification phase 1's real cached-prefix design (ADR §4) must
    // not repeat - that is the whole point of caching the prefix there.
    ctx.clear_kv_cache();

    let prompt = format!(
        "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
    );
    let vocab = model.vocab();
    let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
    let n_prompt = tokens.len() as i32;

    let mut batch = LlamaBatch::new(512, 1);
    for (i, token) in tokens.iter().enumerate() {
        let is_last = i == tokens.len() - 1;
        batch
            .add(*token, i as i32, &[0], is_last)
            .expect("batch add failed - prompt may exceed n_ctx");
    }

    let t_prefill0 = Instant::now();
    ctx.decode(&mut batch).expect("prefill decode failed");
    let prefill_s = t_prefill0.elapsed().as_secs_f64();
    let prefill_tok_per_s = if prefill_s > 0.0 {
        n_prompt as f64 / prefill_s
    } else {
        0.0
    };

    let mut sampler = LlamaSampler::chain_simple([LlamaSampler::dist(1234), LlamaSampler::greedy()]);

    let mut n_cur = batch.n_tokens();
    let mut generated = String::new();
    let mut ttft_ms = 0.0;
    let mut produced = 0i32;
    // Starts only once the first token is sampled (see below), so it times
    // exactly the decode-phase work: the per-token decode() calls that
    // produce tokens 2..produced. Kept separate from prefill's timer so the
    // two durations are never subtracted from each other.
    let mut decode_phase_start: Option<Instant> = None;

    loop {
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if produced == 0 {
            // Time to first token = prefill (the forward pass over the whole
            // prompt, which also yields the first token's logits). The
            // sampling step itself is a negligible, un-timed CPU op - an
            // earlier version of this harness measured only that step and
            // reported TTFT as near zero, which was wrong.
            ttft_ms = prefill_s * 1000.0;
            decode_phase_start = Some(Instant::now());
        }

        if vocab.is_eog(token) || produced >= max_new {
            break;
        }

        let piece_bytes = vocab.token_to_piece(token, true, None);
        generated.push_str(&String::from_utf8_lossy(&piece_bytes));

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .expect("batch add failed during decode");
        n_cur += 1;
        produced += 1;

        ctx.decode(&mut batch).expect("decode failed");
    }

    let decode_s = decode_phase_start.map_or(0.0, |t| t.elapsed().as_secs_f64());
    let decode_tok_per_s = if produced > 1 && decode_s > 0.0 {
        (produced - 1) as f64 / decode_s
    } else {
        0.0
    };

    (ttft_ms, prefill_tok_per_s, decode_tok_per_s, generated)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn run_bench(model_path: &str, threads: i32, runs: usize) {
    let backend = LlamaBackend::init().expect("backend init failed");

    // Cold load: this is the only load in the process. "Warm" below means a
    // second request against the SAME loaded model/context, not a second
    // process start - that distinction matters because the ADR's cold-start
    // budget is specifically about mmap + first decode, not re-exec.
    let (model, cold_load_ms) = load_model(&backend, model_path);
    let mut ctx = new_context(&backend, &model, threads);

    println!("## Bench: {model_path} (threads={threads}, runs={runs})\n");
    println!("cold load: {cold_load_ms:.0} ms\n");

    let prompts: [(&str, &str, &str, i32); 3] = [
        ("spotlight_intent", SPOTLIGHT_SYSTEM, SPOTLIGHT_PROMPT, 64),
        ("writing_rewrite", WRITING_SYSTEM, WRITING_PROMPT, 220),
        ("terminal_explain", TERMINAL_SYSTEM, TERMINAL_PROMPT, 160),
    ];

    println!("| prompt | ttft cold (ms) | ttft warm median (ms) | decode median (tok/s) | prefill median (tok/s) | peak RSS (MiB) |");
    println!("|---|---|---|---|---|---|");

    for (name, system, user, max_new) in prompts {
        let mut ttfts = Vec::with_capacity(runs);
        let mut decodes = Vec::with_capacity(runs);
        let mut prefills = Vec::with_capacity(runs);
        let mut ttft_cold = 0.0;

        for i in 0..runs {
            let (ttft, prefill_tps, decode_tps, _text) =
                run_one(&backend, &model, &mut ctx, system, user, max_new);
            if i == 0 {
                ttft_cold = ttft; // first run after process start / cold cache
            }
            ttfts.push(ttft);
            decodes.push(decode_tps);
            prefills.push(prefill_tps);
        }

        let peak_rss_mib = read_peak_rss_kb() as f64 / 1024.0;
        println!(
            "| {name} | {ttft_cold:.0} | {warm_ttft:.0} | {decode:.1} | {prefill:.1} | {rss:.0} |",
            warm_ttft = median(ttfts),
            decode = median(decodes),
            prefill = median(prefills),
            rss = peak_rss_mib,
        );
    }
}

fn run_quality(model_path: &str) {
    let backend = LlamaBackend::init().expect("backend init failed");
    let (model, _load_ms) = load_model(&backend, model_path);
    let mut ctx = new_context(&backend, &model, 2);

    let mut pass = 0usize;
    for case in INTENT_CASES {
        // 300, not 64: the base (not fine-tuned) Qwen3.5 models emit a
        // <think>...</think> block before the JSON answer for this prompt
        // style even though Unsloth's docs describe non-thinking as the
        // small models' default - that default evidently needs the chat
        // template's enable_thinking=False flag, which this hand-built
        // prompt does not set. 64 tokens let the model get cut off
        // mid-think, which looked like a correctness failure but was
        // really a budget failure; 300 gives the think block room to
        // finish before the actual answer.
        let (_ttft, _pre, _dec, text) =
            run_one(&backend, &model, &mut ctx, SPOTLIGHT_SYSTEM, case.phrase, 300);
        let got_action = extract_action_field(&text);
        let ok = got_action.as_deref() == Some(case.expected_action);
        if ok {
            pass += 1;
        }
        println!(
            "{status}  {phrase:<55}  expected={expected:<16}  got={got:?}  raw={raw}",
            status = if ok { "PASS" } else { "FAIL" },
            phrase = case.phrase,
            expected = case.expected_action,
            got = got_action,
            raw = text.trim(),
        );
    }
    println!("\n{pass}/{total} correct", total = INTENT_CASES.len());
}

/// Pulls `"action":"..."` out of the model's reply without a full JSON
/// parser. A spike tool, not the shipped grammar-constrained decoder (ADR
/// §3's `GBNF`/`llguidance` path), so this intentionally tolerates minor
/// formatting slop (extra whitespace, a leading code fence) the way a human
/// grading the output by eye would.
fn extract_action_field(text: &str) -> Option<String> {
    let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```");
    let key = "\"action\"";
    let idx = cleaned.find(key)?;
    let after_key = &cleaned[idx + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let after_colon = after_colon.trim_start_matches('"');
    let end = after_colon.find(['"', ',', '}'])?;
    Some(after_colon[..end].to_string())
}

fn run_unload(model_path: &str) {
    let backend = LlamaBackend::init().expect("backend init failed");
    let (model, load_ms) = load_model(&backend, model_path);
    let mut ctx = new_context(&backend, &model, 2);

    let (_ttft, _pre, _dec, _text) =
        run_one(&backend, &model, &mut ctx, SPOTLIGHT_SYSTEM, SPOTLIGHT_PROMPT, 64);

    let peak_rss_mib = read_peak_rss_kb() as f64 / 1024.0;
    let rss_loaded_mib = read_rss_kb() as f64 / 1024.0;
    println!("load: {load_ms:.0} ms");
    println!("peak RSS while loaded: {peak_rss_mib:.0} MiB");
    println!("RSS just before drop: {rss_loaded_mib:.0} MiB");

    drop(ctx);
    drop(model);
    drop(backend);

    // Give the allocator a moment to actually return pages before reading
    // RSS again; mmap teardown and malloc_trim-style behaviour are not
    // synchronous with `drop` returning.
    std::thread::sleep(Duration::from_millis(200));
    let rss_after_drop_mib = read_rss_kb() as f64 / 1024.0;
    println!("RSS after drop (same process, before exit): {rss_after_drop_mib:.0} MiB");
    println!(
        "note: the real budget is RSS == 0 after the SERVICE PROCESS EXITS \
         (ADR §4 'Load, unload and memory'), not after an in-process drop. \
         This number only shows whether llama.cpp releases its own \
         allocations; process exit itself is what phase 1's systemd unit \
         must guarantee."
    );
}
