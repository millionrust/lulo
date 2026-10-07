//! `rmac-intelligence-fetch <tier>`: download and verify one pinned model.
//!
//! System Settings starts it and reads one line per event on stdout:
//! `progress <bytes> <total>`, then `done`, or `error <message>`. It keeps
//! going if Settings closes (a broken pipe is ignored), so a download
//! survives the window; the next Settings window sees the finished file or
//! the partial one to resume.

use std::io::Write as _;

fn say(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn main() {
    let Some(tier) = std::env::args()
        .nth(1)
        .as_deref()
        .and_then(rmac_intelligence::manifest::Tier::parse)
    else {
        eprintln!("usage: rmac-intelligence-fetch tiny|standard");
        std::process::exit(2);
    };
    let model = tier.model();
    match rmac_intelligence::fetch::fetch(model, |done, total| {
        say(&format!("progress {done} {total}"));
    }) {
        Ok(_) => say("done"),
        Err(error) => {
            say(&format!("error {error}"));
            std::process::exit(1);
        }
    }
}
