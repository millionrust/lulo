//! Lulo voice measurements on recorded clips (ADR 0024 "Phase 3 (Voice)
//! results"). Not packaged; run on the reference laptop:
//!
//! ```text
//! rmac-voice-bench clips CLIPS.jsonl [--model tiny|base] [--threads N] [--intent M.gguf]
//!     # in process: transcribe each clip ({"file": "x.wav", "text": "reference"} per
//!     # line), word error rate, release-to-transcript time, peak memory; with
//!     # --intent, the transcript also goes through the Tiny language model's intent
//!     # task (Spotlight's "Lulo can do this" pipeline) and that time is added.
//! rmac-voice-bench service CLIP.wav [--runs N]
//!     # through the real org.rmac.Voice1 on the session bus: Start, hold for the
//!     # clip's length, Stop; the service must run with RMAC_VOICE_CAPTURE_FILE=CLIP.wav.
//! ```

#[cfg(target_os = "linux")]
mod bench {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use rmac_intelligence::manifest::Tier;
    use rmac_intelligence::voice::{self, audio, wer};
    use rmac_intelligence_service::engine::Engine as _;
    use rmac_intelligence_service::voice::engine::Transcriber as _;

    struct Arguments {
        command: String,
        target: Option<PathBuf>,
        model: &'static voice::VoiceModel,
        threads: usize,
        intent_model: Option<PathBuf>,
        runs: usize,
    }

    fn arguments() -> Arguments {
        let mut iter = std::env::args().skip(1);
        let mut parsed = Arguments {
            command: iter.next().unwrap_or_default(),
            target: None,
            model: &voice::TINY,
            threads: rmac_intelligence_service::voice::engine::threads(),
            intent_model: None,
            runs: 5,
        };
        while let Some(argument) = iter.next() {
            match argument.as_str() {
                "--model" => {
                    parsed.model = match iter.next().as_deref() {
                        Some("base") => &voice::BASE,
                        _ => &voice::TINY,
                    }
                }
                "--threads" => {
                    parsed.threads = iter.next().and_then(|n| n.parse().ok()).unwrap_or(2)
                }
                "--intent" => parsed.intent_model = iter.next().map(PathBuf::from),
                "--runs" => parsed.runs = iter.next().and_then(|n| n.parse().ok()).unwrap_or(5),
                other => parsed.target = Some(PathBuf::from(other)),
            }
        }
        parsed
    }

    fn peak_rss_mib() -> f64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix("VmHWM:"))
                    .and_then(|value| {
                        value
                            .trim()
                            .trim_end_matches("kB")
                            .trim()
                            .parse::<f64>()
                            .ok()
                    })
            })
            .map(|kib| kib / 1024.0)
            .unwrap_or(0.0)
    }

    fn percentile(values: &[f64], fraction: f64) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    fn load_clip(path: &Path) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap_or_else(|error| {
            eprintln!("{}: {error}", path.display());
            std::process::exit(1);
        });
        let wav = audio::read_wav(&bytes).unwrap_or_else(|error| {
            eprintln!("{}: {error}", path.display());
            std::process::exit(1);
        });
        audio::resample_to_16k(&wav)
    }

    fn clips(arguments: &Arguments) {
        let Some(list) = arguments.target.as_deref() else {
            eprintln!("usage: rmac-voice-bench clips CLIPS.jsonl");
            std::process::exit(2);
        };
        let base = list.parent().unwrap_or(Path::new("."));
        let text = std::fs::read_to_string(list).unwrap_or_default();
        let entries: Vec<serde_json::Value> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let rss_before = peak_rss_mib();
        let models = rmac_intelligence::paths::models_dir().expect("a data folder");
        let paths = arguments.model.verified(&models).unwrap_or_else(|error| {
            eprintln!("the voice model is not ready: {error}");
            std::process::exit(1);
        });
        let started = Instant::now();
        let mut engine =
            rmac_intelligence_service::voice::moonshine::Moonshine::load(&paths, arguments.threads)
                .unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                });
        let load_ms = started.elapsed().as_secs_f64() * 1000.0;
        let rss_loaded = peak_rss_mib();
        let mut intent = arguments.intent_model.as_ref().map(|path| {
            rmac_intelligence_service::llama::LlamaEngine::load(
                path,
                Tier::Tiny,
                rmac_intelligence_service::llama::Options {
                    threads: arguments.threads as i32,
                    style: rmac_intelligence::prompt::PromptStyle::DEFAULT,
                    state_cache: rmac_intelligence::paths::cache_dir(),
                    guard: true,
                },
            )
            .unwrap_or_else(|error| {
                eprintln!("the language model did not load: {error}");
                std::process::exit(1);
            })
        });
        println!(
            "{}",
            serde_json::json!({"event": "loaded", "model": arguments.model.display_name,
                "threads": arguments.threads, "load_ms": load_ms,
                "rss_before_mib": rss_before, "rss_loaded_mib": rss_loaded})
        );
        let (mut transcribe_times, mut intent_times, mut totals) =
            (Vec::new(), Vec::new(), Vec::new());
        let mut pairs: Vec<(String, String)> = Vec::new();
        for entry in &entries {
            let file = base.join(entry["file"].as_str().unwrap_or_default());
            let reference = entry["text"].as_str().unwrap_or_default().to_owned();
            let samples = load_clip(&file);
            let started = Instant::now();
            let (raw, steps) = engine.transcribe(&samples).unwrap_or_else(|error| {
                eprintln!("{}: {error}", file.display());
                (String::new(), Default::default())
            });
            let transcribe_ms = started.elapsed().as_secs_f64() * 1000.0;
            let hypothesis = voice::clean_transcript(&raw);
            transcribe_times.push(transcribe_ms);
            let mut row = serde_json::json!({
                "event": "clip",
                "file": entry["file"],
                "accent": entry["accent"],
                "reference": reference,
                "hypothesis": hypothesis,
                "audio_ms": samples.len() as f64 / 16.0,
                "transcribe_ms": transcribe_ms,
                "preprocess_ms": steps.preprocess_ms,
                "encode_ms": steps.encode_ms,
                "decode_ms": steps.decode_ms,
                "tokens": steps.tokens,
                "errors": wer::errors(&wer::normalise(&reference), &wer::normalise(&hypothesis)),
                "words": wer::normalise(&reference).len(),
            });
            if let Some(engine) = intent.as_mut() {
                let started = Instant::now();
                let answer = if rmac_intelligence::prompt::worth_asking(&hypothesis) {
                    engine
                        .intent(&hypothesis, started, &|| false)
                        .map(|outcome| outcome.intent.to_json())
                        .unwrap_or_else(|error| format!("error: {error}"))
                } else {
                    "(not asked: under two words)".into()
                };
                let intent_ms = started.elapsed().as_secs_f64() * 1000.0;
                intent_times.push(intent_ms);
                totals.push(transcribe_ms + intent_ms);
                row["intent"] = answer.into();
                row["intent_ms"] = intent_ms.into();
                row["expected_intent"] = entry["intent"].clone();
            }
            println!("{row}");
            pairs.push((reference, hypothesis));
        }
        let rate = wer::word_error_rate(
            pairs
                .iter()
                .map(|(reference, hypothesis)| (reference.as_str(), hypothesis.as_str())),
        );
        println!(
            "{}",
            serde_json::json!({"event": "summary", "clips": entries.len(), "wer": rate,
                "transcribe_p50_ms": percentile(&transcribe_times, 0.5),
                "transcribe_p90_ms": percentile(&transcribe_times, 0.9),
                "transcribe_max_ms": percentile(&transcribe_times, 1.0),
                "intent_p50_ms": percentile(&intent_times, 0.5),
                "intent_p90_ms": percentile(&intent_times, 0.9),
                "release_to_intent_p50_ms": percentile(&totals, 0.5),
                "release_to_intent_p90_ms": percentile(&totals, 0.9),
                "peak_rss_mib": peak_rss_mib()})
        );
    }

    /// Through the real service: Start, hold for the clip, Stop.
    fn through_service(arguments: &Arguments) {
        let Some(clip) = arguments.target.as_deref() else {
            eprintln!("usage: rmac-voice-bench service CLIP.wav");
            std::process::exit(2);
        };
        let hold = Duration::from_secs_f64(load_clip(clip).len() as f64 / 16_000.0 + 0.3);
        for run in 0..arguments.runs {
            let (controls, receiver) = async_channel::bounded(2);
            let levels = std::cell::Cell::new(0u32);
            let started = Instant::now();
            let stopper = std::thread::spawn(move || {
                std::thread::sleep(hold);
                let released = Instant::now();
                let _ = controls.send_blocking(voice::client::Control::Stop);
                released
            });
            let result = async_io::block_on(voice::client::listen(false, receiver, |event| {
                if let voice::client::Event::Level(_) = event {
                    levels.set(levels.get() + 1);
                }
            }));
            let released = stopper.join().unwrap_or_else(|_| Instant::now());
            let client_ms = released.elapsed().as_secs_f64() * 1000.0;
            match result {
                Ok(transcript) => println!(
                    "{}",
                    serde_json::json!({"event": "service", "run": run, "text": transcript.text,
                    "release_to_text_client_ms": client_ms,
                    "levels": levels.get(),
                    "held_ms": hold.as_secs_f64() * 1000.0,
                    "since_start_ms": started.elapsed().as_secs_f64() * 1000.0,
                    "timing": {
                        "audio_ms": transcript.timing.audio_ms,
                        "stop_to_text_ms": transcript.timing.stop_to_text_ms,
                        "waited_for_model_ms": transcript.timing.waited_for_model_ms,
                        "encode_ms": transcript.timing.encode_ms,
                        "decode_ms": transcript.timing.decode_ms,
                        "model_load_ms": transcript.timing.model_load_ms,
                        "start_to_audio_ms": transcript.timing.start_to_audio_ms,
                    }})
                ),
                Err(error) => println!(
                    "{}",
                    serde_json::json!({"event": "service", "run": run, "error": error.to_string()})
                ),
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    pub fn main() {
        let arguments = arguments();
        match arguments.command.as_str() {
            "clips" => clips(&arguments),
            "service" => through_service(&arguments),
            _ => {
                eprintln!("usage: rmac-voice-bench clips CLIPS.jsonl | service CLIP.wav");
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
        eprintln!("rmac-voice-bench runs on Linux");
        std::process::exit(2);
    }
}
