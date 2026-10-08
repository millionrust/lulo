//! Accuracy and latency measurements for ADR 0024 phase 1, on the real
//! model. Not packaged; run on the reference laptop:
//!
//! ```text
//! rmac-intelligence-bench eval    --model M.gguf [--set dev|heldout|all] [--style compact|list|chat]
//!                                 [--decoder schema|gbnf (JSON styles)] [--threads N] [--state-cache DIR|none]
//!                                 [--no-guard]   # the model alone, without guard::check
//! rmac-intelligence-bench latency --model M.gguf [--runs N] [--threads N] [--state-cache DIR|none]
//! rmac-intelligence-bench service [--runs N] [QUERY ...]   # through the real service
//! ```

#[cfg(target_os = "linux")]
mod bench {
    use std::path::PathBuf;
    use std::time::Instant;

    use rmac_intelligence::eval::{self, Score};
    use rmac_intelligence::manifest::Tier;
    use rmac_intelligence::prompt::PromptStyle;
    use rmac_intelligence_service::engine::Engine as _;
    use rmac_intelligence_service::llama::{LlamaEngine, Options};

    const DEV: &str = include_str!("../../../../tests/intelligence/intents-dev.jsonl");
    const HELD_OUT: &str = include_str!("../../../../tests/intelligence/intents-heldout.jsonl");
    const BRIEF: [&str; 4] = [
        "turn on dark mode",
        "set a timer for 10 minutes",
        "open Notes",
        "volume 30%",
    ];

    struct Arguments {
        command: String,
        model: Option<PathBuf>,
        tier: Tier,
        style: PromptStyle,
        gbnf: bool,
        threads: i32,
        set: String,
        state_cache: Option<PathBuf>,
        runs: usize,
        rest: Vec<String>,
        verbose: bool,
        guard: bool,
    }

    fn arguments() -> Arguments {
        let mut iter = std::env::args().skip(1);
        let mut parsed = Arguments {
            command: iter.next().unwrap_or_default(),
            model: None,
            tier: Tier::Tiny,
            style: PromptStyle::DEFAULT,
            gbnf: false,
            threads: 2,
            set: "all".into(),
            state_cache: rmac_intelligence::paths::cache_dir(),
            runs: 20,
            rest: Vec::new(),
            verbose: false,
            guard: true,
        };
        while let Some(argument) = iter.next() {
            let mut value = || iter.next().unwrap_or_default();
            match argument.as_str() {
                "--model" => parsed.model = Some(PathBuf::from(value())),
                "--tier" => parsed.tier = Tier::parse(&value()).unwrap_or(Tier::Tiny),
                "--style" => {
                    parsed.style = PromptStyle::parse(&value()).unwrap_or(PromptStyle::DEFAULT)
                }
                "--decoder" => parsed.gbnf = value() == "gbnf",
                "--threads" => parsed.threads = value().parse().unwrap_or(2),
                "--set" => parsed.set = value(),
                "--runs" => parsed.runs = value().parse().unwrap_or(20),
                "--state-cache" => {
                    let directory = value();
                    parsed.state_cache = (directory != "none").then(|| PathBuf::from(directory));
                }
                "--verbose" => parsed.verbose = true,
                "--no-guard" => parsed.guard = false,
                _ => parsed.rest.push(argument),
            }
        }
        parsed
    }

    fn peak_rss_mib() -> f64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|rest| {
                rest.trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .map_or(0.0, |kib| kib / 1024.0)
    }

    fn percentile(values: &[f64], share: f64) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let index = ((sorted.len() - 1) as f64 * share).round() as usize;
        sorted[index]
    }

    fn load(arguments: &Arguments) -> (LlamaEngine, f64) {
        let model = arguments.model.clone().unwrap_or_else(|| {
            eprintln!("--model is required");
            std::process::exit(2);
        });
        let started = Instant::now();
        let engine = LlamaEngine::load(
            &model,
            arguments.tier,
            Options {
                threads: arguments.threads,
                style: arguments.style,
                state_cache: arguments.state_cache.clone(),
                guard: arguments.guard,
            },
        )
        .unwrap_or_else(|error| {
            eprintln!("load failed: {error}");
            std::process::exit(1);
        });
        let load_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = engine.load_report();
        println!(
            "load: {load_ms:.0} ms (model {:.0} ms, prefix {} tokens {} in {:.0} ms), threads {}, style {}",
            report.model_load_ms,
            report.prefix_tokens,
            if report.prefix_restored { "restored" } else { "evaluated" },
            report.prefix_ms,
            engine.threads(),
            arguments.style.as_str(),
        );
        (engine, load_ms)
    }

    fn evaluate(arguments: &Arguments) {
        let (mut engine, load_ms) = load(arguments);
        let sets: Vec<(&str, &str)> = match arguments.set.as_str() {
            "dev" => vec![("dev", DEV)],
            "heldout" => vec![("heldout", HELD_OUT)],
            _ => vec![("dev", DEV), ("heldout", HELD_OUT)],
        };
        let mut summary = Vec::new();
        for (name, text) in sets {
            let cases = eval::parse_cases(text).expect("evaluation set parses");
            let mut score = Score::default();
            let mut invalid = 0usize;
            let mut first = Vec::new();
            let mut total = Vec::new();
            for case in &cases {
                let outcome = if arguments.gbnf {
                    engine.intent_gbnf(&case.request)
                } else {
                    engine.intent(&case.request, Instant::now(), &|| false)
                };
                match outcome {
                    Ok(outcome) => {
                        let ok = eval::matches(&case.expect, &outcome.intent);
                        score.add(&case.expect, &outcome.intent);
                        first.push(outcome.timing.first_token_ms);
                        total.push(outcome.timing.total_ms);
                        if arguments.verbose || !ok {
                            println!(
                                "{} {name} {:<48} expected={} got={} ({:.0}/{:.0} ms)",
                                if ok { "PASS" } else { "FAIL" },
                                case.request,
                                case.expect.to_json(),
                                outcome.json,
                                outcome.timing.first_token_ms,
                                outcome.timing.total_ms,
                            );
                        }
                    }
                    Err(error) => {
                        invalid += 1;
                        score.add(&case.expect, &rmac_intelligence::Intent::None);
                        println!("INVALID {name} {:<48} {error}", case.request);
                    }
                }
            }
            let line = serde_json::json!({
                "set": name,
                "decoder": if arguments.gbnf { "gbnf" } else { "schema" },
                "style": arguments.style.as_str(),
                "threads": arguments.threads,
                "cases": score.cases,
                "correct": score.correct,
                "percent": (score.percent() * 10.0).round() / 10.0,
                "wrong_action": score.wrong_action,
                "missed": score.missed,
                "invalid": invalid,
                "first_token_ms_p50": percentile(&first, 0.5).round(),
                "first_token_ms_p90": percentile(&first, 0.9).round(),
                "total_ms_p50": percentile(&total, 0.5).round(),
                "total_ms_p90": percentile(&total, 0.9).round(),
            });
            summary.push(line);
        }
        for line in &summary {
            println!("RESULT {line}");
        }
        println!(
            "RESULT {}",
            serde_json::json!({"load_ms": load_ms.round(), "peak_rss_mib": peak_rss_mib().round()})
        );
    }

    fn latency(arguments: &Arguments) {
        let (mut engine, load_ms) = load(arguments);
        let started = Instant::now();
        let first = engine
            .intent(BRIEF[0], started, &|| false)
            .expect("the first request decodes");
        println!(
            "first request after load: first token {:.0} ms, total {:.0} ms, {} request tokens, {} passes",
            first.timing.first_token_ms,
            first.timing.total_ms,
            first.timing.request_tokens,
            first.timing.passes
        );
        let mut first_ms = Vec::new();
        let mut total_ms = Vec::new();
        let mut rewind_ms = Vec::new();
        let mut prefill_ms = Vec::new();
        let mut decode_ms = Vec::new();
        let mut prefill_rate = Vec::new();
        let mut pass_ms = Vec::new();
        for run in 0..arguments.runs {
            let query = BRIEF[run % BRIEF.len()];
            let outcome = engine
                .intent(query, Instant::now(), &|| false)
                .expect("a warm request decodes");
            let timing = &outcome.timing;
            first_ms.push(timing.first_token_ms);
            total_ms.push(timing.total_ms);
            rewind_ms.push(timing.rewind_ms);
            prefill_ms.push(timing.prefill_ms);
            decode_ms.push(timing.decode_ms);
            prefill_rate
                .push(f64::from(timing.request_tokens) * 1000.0 / timing.prefill_ms.max(1e-3));
            if timing.passes > 0 {
                pass_ms.push(timing.decode_ms / f64::from(timing.passes));
            }
            if arguments.verbose {
                println!(
                    "{query:<28} {} first {:.0} ms total {:.0} ms (rewind {:.0}, prefill {:.0} for {} tokens, decode {:.0} in {} passes)",
                    outcome.json,
                    timing.first_token_ms,
                    timing.total_ms,
                    timing.rewind_ms,
                    timing.prefill_ms,
                    timing.request_tokens,
                    timing.decode_ms,
                    timing.passes
                );
            }
        }
        println!(
            "RESULT {}",
            serde_json::json!({
                "load_ms": load_ms.round(),
                "prefix_tokens": engine.load_report().prefix_tokens,
                "prefix_restored": engine.load_report().prefix_restored,
                "prefix_ms": engine.load_report().prefix_ms.round(),
                "first_request_first_token_ms": first.timing.first_token_ms.round(),
                "first_request_total_ms": first.timing.total_ms.round(),
                "warm_runs": arguments.runs,
                "warm_first_token_ms_p50": percentile(&first_ms, 0.5).round(),
                "warm_first_token_ms_p90": percentile(&first_ms, 0.9).round(),
                "warm_total_ms_p50": percentile(&total_ms, 0.5).round(),
                "warm_total_ms_p90": percentile(&total_ms, 0.9).round(),
                "warm_rewind_ms_p50": percentile(&rewind_ms, 0.5).round(),
                "warm_prefill_ms_p50": percentile(&prefill_ms, 0.5).round(),
                "warm_prefill_tok_s_p50": percentile(&prefill_rate, 0.5).round(),
                "warm_decode_ms_p50": percentile(&decode_ms, 0.5).round(),
                "warm_ms_per_pass_p50": percentile(&pass_ms, 0.5).round(),
                "peak_rss_mib": peak_rss_mib().round(),
            })
        );
        let calibration = engine.calibrate().expect("calibration runs");
        println!("RESULT {}", serde_json::json!(calibration));
    }

    /// Through the real service on the session bus: what Spotlight sees.
    fn through_service(arguments: &Arguments) {
        let queries: Vec<String> = if arguments.rest.is_empty() {
            BRIEF.iter().map(|query| (*query).to_owned()).collect()
        } else {
            arguments.rest.clone()
        };
        let mut wall = Vec::new();
        for run in 0..arguments.runs.max(1) {
            let query = &queries[run % queries.len()];
            let started = Instant::now();
            match rmac_intelligence::client::intent(query) {
                Ok(reply) => {
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                    println!(
                        "{run:>3} {query:<28} {} client {elapsed:.0} ms, service first {:.0} ms total {:.0} ms{}",
                        reply.intent.to_json(),
                        reply.timing.first_token_ms,
                        reply.timing.total_ms,
                        if reply.timing.cold { " (cold)" } else { "" }
                    );
                    if run > 0 {
                        wall.push(elapsed);
                    }
                }
                Err(error) => println!("{run:>3} {query:<28} error: {error}"),
            }
        }
        println!(
            "RESULT {}",
            serde_json::json!({
                "warm_client_ms_p50": percentile(&wall, 0.5).round(),
                "warm_client_ms_p90": percentile(&wall, 0.9).round(),
                "samples": wall.len(),
            })
        );
    }

    pub fn main() {
        let arguments = arguments();
        match arguments.command.as_str() {
            "eval" => evaluate(&arguments),
            "latency" => latency(&arguments),
            "service" => through_service(&arguments),
            _ => {
                eprintln!("usage: rmac-intelligence-bench eval|latency|service --model M.gguf …");
                std::process::exit(2);
            }
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    bench::main();
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("rmac-intelligence-bench runs on Linux");
        std::process::exit(2);
    }
}
