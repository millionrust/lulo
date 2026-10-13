//! `rmac-voice-hud`: Lulo's talk key (ADR 0024 §8).
//!
//! ⌥Space starts this process through the shortcut dispatcher. It opens the
//! microphone through `org.rmac.Voice1` at once, shows a small listening
//! pill with a live level meter while the key is held, and when the key is
//! let go it hands the transcript to Spotlight, which treats it exactly
//! like typed text: the same search, the same "Lulo can do this" row, the
//! same second Return before a Settings change. Then it exits; nothing runs
//! between presses.

#[cfg(target_os = "linux")]
mod hud;
#[cfg(any(target_os = "linux", test))]
mod model;

fn main() {
    #[cfg(target_os = "linux")]
    hud::run();
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("rmac-voice-hud runs on Linux");
        std::process::exit(2);
    }
}
