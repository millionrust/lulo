//! Accuracy and latency measurements for ADR 0024 phase 1, on the real
//! model. Not packaged; run on the reference laptop:
//!
//! ```text
//! rmac-intelligence-bench eval    --model M.gguf [--set dev|heldout|all] [--style compact|list|chat]
//!                                 [--decoder schema|gbnf (JSON styles)] [--threads N] [--state-cache DIR|none]
//!                                 [--no-guard]   # the model alone, without guard::check
//! rmac-intelligence-bench latency --model M.gguf [--runs N] [--threads N] [--state-cache DIR|none]
//! rmac-intelligence-bench service [--runs N] [QUERY ...]   # through the real service
//!
//! # Writing Tools (ADR 0024 "Phase 2 (Writing Tools) results")
//! rmac-intelligence-bench writing-eval --model M.gguf [--tier tiny|standard] [--tasks proofread,summarise,…]
//!                                      [--passages N] [--verbose]   # the 30-passage set
//! rmac-intelligence-bench write-latency --model M.gguf [--tier …] [--task rewrite] [--runs N]
//!                                       # the 150-word rewrite: first token, tok/s
//! rmac-intelligence-bench write-service [--task rewrite] [--runs N]   # through the real service
//! rmac-intelligence-bench writing-rescore LOG [--verbose]   # score a writing-eval log again
//! ```

#[cfg(target_os = "linux")]
mod bench {
    use std::path::PathBuf;
    use std::time::Instant;

    use rmac_intelligence::eval::{self, Score};
    use rmac_intelligence::manifest::Tier;
    use rmac_intelligence::prompt::PromptStyle;
    use rmac_intelligence::{writing, writing_eval, Task};
    use rmac_intelligence_service::engine::{Engine as _, WriteOutcome, WriteRequest};
    use rmac_intelligence_service::llama::{LlamaEngine, Options};

    const PASSAGES: &str = include_str!("../../../../tests/intelligence/writing-passages.jsonl");
    /// The 150-word passage the speed numbers use.
    const TIMING_PASSAGE: &str = "p30-rewrite-150";
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
        tasks: Vec<rmac_intelligence::Task>,
        passages: usize,
        skip: usize,
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
            tasks: rmac_intelligence::Task::WRITING.to_vec(),
            passages: 30,
            skip: 0,
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
                "--tasks" | "--task" => {
                    parsed.tasks = value()
                        .split(',')
                        .filter_map(rmac_intelligence::Task::parse)
                        .filter(|task| task.is_writing())
                        .collect()
                }
                "--passages" => parsed.passages = value().parse().unwrap_or(30),
                "--skip" => parsed.skip = value().parse().unwrap_or(0),
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

    fn passages() -> Vec<writing_eval::Passage> {
        writing_eval::parse_passages(PASSAGES).expect("the passage set parses")
    }

    fn write_once(
        engine: &mut LlamaEngine,
        task: Task,
        text: &str,
        attempt: u32,
    ) -> Result<WriteOutcome, rmac_intelligence_service::engine::EngineError> {
        let cleaned = writing::clean_input(text);
        let request = WriteRequest {
            task,
            text: &cleaned,
            attempt,
            received: Instant::now(),
        };
        engine.write(&request, &mut |_| {}, &|| false)
    }

    /// The 30-passage Writing Tools evaluation, every task or `--tasks`.
    fn writing_evaluation(arguments: &Arguments) {
        let (mut engine, load_ms) = load(arguments);
        let passages = passages();
        let start = arguments.skip.min(passages.len());
        let passages = &passages[start..(start + arguments.passages).min(passages.len())];
        let mut summary = Vec::new();
        for task in &arguments.tasks {
            let task = *task;
            let mut first = Vec::new();
            let mut rates = Vec::new();
            let mut passed = 0usize;
            let mut capped = 0usize;
            let (mut injected, mut fixed, mut unwanted, mut exact) = (0u32, 0u32, 0u32, 0usize);
            let mut words = Vec::new();
            for passage in passages {
                let (input, errors) = if task == Task::Proofread {
                    writing_eval::inject_errors(passage, 4)
                } else {
                    (passage.text.clone(), Vec::new())
                };
                let outcome = match write_once(&mut engine, task, &input, 0) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        println!("ERROR {} {} {error}", task.as_str(), passage.id);
                        continue;
                    }
                };
                first.push(outcome.timing.first_token_ms);
                rates.push(outcome.timing.decode_tok_s);
                capped += usize::from(outcome.capped);
                words.push(f64::from(writing::word_count(&outcome.text)));
                let verdict = if task == Task::Proofread {
                    let score =
                        writing_eval::score_proofread(&passage.text, &errors, &outcome.text);
                    injected += score.injected;
                    fixed += score.fixed;
                    unwanted += score.unwanted_changes;
                    exact += usize::from(score.exact);
                    // A passage passes when at least three of its four
                    // errors are fixed and at most one right word changed.
                    let pass = score.fixed + 1 >= score.injected && score.unwanted_changes <= 1;
                    passed += usize::from(pass);
                    serde_json::json!({
                        "pass": pass, "fixed": score.fixed, "injected": score.injected,
                        "unwanted": score.unwanted_changes, "exact": score.exact,
                        "errors": errors.iter().map(|error| format!("{} -> {}", error.right, error.wrong)).collect::<Vec<_>>(),
                    })
                } else {
                    let check = writing_eval::check(task, passage, &outcome.text);
                    passed += usize::from(check.pass);
                    serde_json::json!({
                        "pass": check.pass, "words": check.words, "ratio": (check.ratio * 100.0).round() / 100.0,
                        "terms": (check.terms * 100.0).round() / 100.0, "numbers_kept": check.numbers_kept,
                        "bullets": check.bullets, "reason": check.reason,
                    })
                };
                println!(
                    "SAMPLE {}",
                    serde_json::json!({
                        "task": task.as_str(), "id": passage.id, "input": input,
                        "answer": outcome.text, "raw": outcome.raw, "capped": outcome.capped,
                        "verdict": verdict,
                        "first_token_ms": outcome.timing.first_token_ms.round(),
                        "input_tokens": outcome.timing.input_tokens,
                        "output_tokens": outcome.timing.output_tokens,
                        "decode_tok_s": (outcome.timing.decode_tok_s * 10.0).round() / 10.0,
                    })
                );
            }
            let mut line = serde_json::json!({
                "task": task.as_str(),
                "tier": arguments.tier.as_str(),
                "passages": passages.len(),
                "passed": passed,
                "capped": capped,
                "first_token_ms_p50": percentile(&first, 0.5).round(),
                "first_token_ms_p90": percentile(&first, 0.9).round(),
                "decode_tok_s_p50": (percentile(&rates, 0.5) * 10.0).round() / 10.0,
                "answer_words_p50": percentile(&words, 0.5),
                "answer_words_max": percentile(&words, 1.0),
            });
            if task == Task::Proofread {
                line["errors_injected"] = injected.into();
                line["errors_fixed"] = fixed.into();
                line["unwanted_changes"] = unwanted.into();
                line["exact"] = exact.into();
            }
            println!("RESULT {line}");
            summary.push(line);
        }
        println!(
            "RESULT {}",
            serde_json::json!({"load_ms": load_ms.round(), "peak_rss_mib": peak_rss_mib().round()})
        );
    }

    /// Score the answers a `writing-eval` run logged (its `SAMPLE` lines)
    /// with the current checks, no model needed, split into the five
    /// passages the prompts were tuned on (p01–p05) and the 25 held out.
    fn writing_rescore(arguments: &Arguments) {
        let log = arguments.rest.first().cloned().unwrap_or_else(|| {
            eprintln!("usage: rmac-intelligence-bench writing-rescore LOG");
            std::process::exit(2);
        });
        let text = std::fs::read_to_string(&log).expect("the log reads");
        let passages = passages();
        // task -> split -> (passed, total, invented answers, fixed, injected, unwanted)
        let mut totals: std::collections::BTreeMap<(String, &str), [u32; 6]> = Default::default();
        for line in text.lines() {
            let Some(json) = line.strip_prefix("SAMPLE ") else {
                continue;
            };
            let Ok(sample) = serde_json::from_str::<serde_json::Value>(json) else {
                continue;
            };
            let task_name = sample["task"].as_str().unwrap_or_default().to_owned();
            let Some(task) = Task::parse(&task_name) else {
                continue;
            };
            let id = sample["id"].as_str().unwrap_or_default();
            let answer = sample["answer"].as_str().unwrap_or_default();
            let Some(passage) = passages.iter().find(|passage| passage.id == id) else {
                continue;
            };
            let split = if id < "p06" { "dev" } else { "heldout" };
            let entry = totals.entry((task_name, split)).or_default();
            entry[1] += 1;
            let invented = writing_eval::invented_numbers(&passage.text, answer);
            if !invented.is_empty() {
                entry[2] += 1;
            }
            let pass = if task == Task::Proofread {
                let (_, errors) = writing_eval::inject_errors(passage, 4);
                let score = writing_eval::score_proofread(&passage.text, &errors, answer);
                entry[3] += score.fixed;
                entry[4] += score.injected;
                entry[5] += score.unwanted_changes;
                score.fixed + 1 >= score.injected
                    && score.unwanted_changes <= 1
                    && invented.is_empty()
            } else {
                let check = writing_eval::check(task, passage, answer);
                if arguments.verbose && !check.pass {
                    println!(
                        "FAIL {} {id}: {} {:?}",
                        task.as_str(),
                        check.reason,
                        check.invented
                    );
                }
                check.pass
            };
            entry[0] += u32::from(pass);
        }
        for ((task, split), [passed, total, invented, fixed, injected, unwanted]) in totals {
            println!(
                "RESULT {}",
                serde_json::json!({
                    "task": task, "split": split, "passed": passed, "passages": total,
                    "answers_with_invented_numbers": invented,
                    "errors_fixed": fixed, "errors_injected": injected, "unwanted_changes": unwanted,
                })
            );
        }
    }

    /// The 150-word rewrite, in process: first token and decode rate.
    fn write_latency(arguments: &Arguments) {
        let (mut engine, load_ms) = load(arguments);
        let passage = passages()
            .into_iter()
            .find(|passage| passage.id == TIMING_PASSAGE)
            .expect("the timing passage");
        for task in &arguments.tasks {
            let mut first = Vec::new();
            let mut rates = Vec::new();
            let mut totals = Vec::new();
            let mut prefill = Vec::new();
            for run in 0..arguments.runs.max(1) {
                let outcome = write_once(&mut engine, *task, &passage.text, run as u32)
                    .expect("the timing passage is answered");
                let timing = &outcome.timing;
                println!(
                    "{} run {run}: first token {:.0} ms (prefix {:.0} ms{}, {} input tokens in {:.0} ms), {} output tokens at {:.1} tok/s, total {:.0} ms",
                    task.as_str(),
                    timing.first_token_ms,
                    timing.prefix_ms,
                    if timing.prefix_evaluated { " evaluated" } else { "" },
                    timing.input_tokens,
                    timing.prefill_ms,
                    timing.output_tokens,
                    timing.decode_tok_s,
                    timing.total_ms
                );
                if arguments.verbose {
                    println!("{}", outcome.text);
                }
                // The first run may evaluate the prefix: report it apart.
                if run > 0 || !timing.prefix_evaluated {
                    first.push(timing.first_token_ms);
                    rates.push(timing.decode_tok_s);
                    totals.push(timing.total_ms);
                    prefill.push(
                        f64::from(timing.input_tokens) * 1000.0 / timing.prefill_ms.max(1e-3),
                    );
                }
            }
            println!(
                "RESULT {}",
                serde_json::json!({
                    "task": task.as_str(),
                    "tier": arguments.tier.as_str(),
                    "load_ms": load_ms.round(),
                    "words": writing::word_count(&passage.text),
                    "runs": first.len(),
                    "first_token_ms_p50": percentile(&first, 0.5).round(),
                    "first_token_ms_max": percentile(&first, 1.0).round(),
                    "prefill_tok_s_p50": percentile(&prefill, 0.5).round(),
                    "decode_tok_s_p50": (percentile(&rates, 0.5) * 10.0).round() / 10.0,
                    "total_ms_p50": percentile(&totals, 0.5).round(),
                    "peak_rss_mib": peak_rss_mib().round(),
                })
            );
        }
    }

    /// The 150-word rewrite through the real service, as Notes asks: the
    /// time to the first streamed delta on the client's clock.
    fn write_through_service(arguments: &Arguments) {
        let passage = passages()
            .into_iter()
            .find(|passage| passage.id == TIMING_PASSAGE)
            .expect("the timing passage");
        let task = arguments.tasks.first().copied().unwrap_or(Task::Rewrite);
        let mut first = Vec::new();
        for run in 0..arguments.runs.max(1) {
            let started = Instant::now();
            let mut first_delta = None;
            let mut deltas = 0usize;
            let (_cancel, cancelled) = async_channel::bounded::<()>(1);
            let result = futures_lite::future::block_on(rmac_intelligence::client::write(
                task,
                &passage.text,
                run as u32,
                cancelled,
                |_| {
                    deltas += 1;
                    first_delta.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
                },
            ));
            match result {
                Ok(done) => {
                    let first_ms = first_delta.unwrap_or_default();
                    println!(
                        "{run:>3} {}: first delta {first_ms:.0} ms (service {:.0} ms{}), {deltas} deltas, {} tokens at {:.1} tok/s, client total {:.0} ms",
                        task.as_str(),
                        done.timing.first_token_ms,
                        if done.timing.cold { ", cold" } else { "" },
                        done.timing.output_tokens,
                        done.timing.decode_tok_s,
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                    if run > 0 {
                        first.push(first_ms);
                    }
                }
                Err(error) => println!("{run:>3} error: {error}"),
            }
        }
        println!(
            "RESULT {}",
            serde_json::json!({
                "task": task.as_str(),
                "warm_first_delta_ms_p50": percentile(&first, 0.5).round(),
                "warm_first_delta_ms_max": percentile(&first, 1.0).round(),
                "samples": first.len(),
            })
        );
    }

    pub fn main() {
        let arguments = arguments();
        match arguments.command.as_str() {
            "eval" => evaluate(&arguments),
            "latency" => latency(&arguments),
            "service" => through_service(&arguments),
            "writing-eval" => writing_evaluation(&arguments),
            "writing-rescore" => writing_rescore(&arguments),
            "write-latency" => write_latency(&arguments),
            "write-service" => write_through_service(&arguments),
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
