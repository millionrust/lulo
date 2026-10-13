//! `rmac-intelligence-fetch <tier>`: download and verify one pinned model.
//! `rmac-intelligence-fetch voice-<tier>` downloads that tier's voice model
//! (ADR 0024 §8), file by file, each checked against its own SHA-256, and
//! ONNX Runtime with it (Microsoft's release, pinned the same way).
//!
//! System Settings starts it and reads one line per event on stdout:
//! `progress <bytes> <total>`, then `done`, or `error <message>`. It keeps
//! going if Settings closes (a broken pipe is ignored), so a download
//! survives the window; the next Settings window sees the finished file or
//! the partial one to resume.

use std::io::Write as _;

use rmac_intelligence::manifest::{Pinned as _, Tier};
use rmac_intelligence::voice;

fn say(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn main() {
    let argument = std::env::args().nth(1).unwrap_or_default();
    let result = if let Some(tier) = argument.strip_prefix("voice-").and_then(Tier::parse) {
        let model = tier.voice_model();
        let total = model.download_size();
        let mut before = 0u64;
        let mut outcome = Ok(());
        for file in &model.files {
            let base = before;
            if let Err(error) = rmac_intelligence::fetch::fetch(file, |done, _| {
                say(&format!("progress {} {total}", base + done));
            }) {
                outcome = Err(error);
                break;
            }
            before += file.size();
        }
        // ONNX Runtime, which the voice service loads (see
        // `rmac_intelligence::voice::runtime`).
        if let (Ok(()), Some(library)) = (&outcome, voice::runtime::for_this_cpu()) {
            let base = before;
            outcome = match rmac_intelligence::paths::models_dir() {
                Some(models) => voice::runtime::install(&models, library, |done, _| {
                    say(&format!("progress {} {total}", base + done));
                })
                .map(|_| ()),
                None => Err(rmac_intelligence::fetch::FetchError::NoDataDirectory),
            };
        }
        outcome
    } else if let Some(tier) = Tier::parse(&argument) {
        rmac_intelligence::fetch::fetch(tier.model(), |done, total| {
            say(&format!("progress {done} {total}"));
        })
        .map(|_| ())
    } else {
        eprintln!("usage: rmac-intelligence-fetch tiny|standard|voice-tiny|voice-standard");
        std::process::exit(2);
    };
    match result {
        Ok(()) => say("done"),
        Err(error) => {
            say(&format!("error {error}"));
            std::process::exit(1);
        }
    }
}
