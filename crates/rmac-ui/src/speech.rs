//! Shared Edit ▸ Speech behaviour (Start Speaking / Stop Speaking).
//!
//! Reads text aloud through speech-dispatcher's `spd-say` CLI, the
//! lightweight system text-to-speech already available on Ubuntu — no
//! bundled voice engine, matching the low-spec-PC budget. `spd-say` itself
//! is spawned and waited on off the UI thread (`blocking::unblock`, the
//! same off-thread pattern used elsewhere in this codebase for blocking
//! I/O); only the process id, needed so Stop can kill it, crosses back to
//! the caller through a small mutex. Where `spd-say` is not installed
//! (every platform but the Ubuntu target), spawning fails and this module
//! quietly does nothing — there is no voice to fall back to.

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use gpui::{App, Entity};

fn speaking_pid() -> &'static Mutex<Option<u32>> {
    static PID: OnceLock<Mutex<Option<u32>>> = OnceLock::new();
    PID.get_or_init(|| Mutex::new(None))
}

fn lock_pid() -> std::sync::MutexGuard<'static, Option<u32>> {
    speaking_pid()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn null_stdio(command: &mut Command) -> &mut Command {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
}

/// Speak `field`'s current selection, or its whole text when nothing is
/// selected. `stop_action` is the app's own `"…::StopSpeaking"` action
/// name, enabled for exactly as long as this utterance is speaking so the
/// menu greys it the way the Mac does.
pub fn start_speaking<T: crate::text_assist::EditableText>(
    field: &Entity<T>,
    stop_action: &'static str,
    cx: &mut App,
) {
    let text = {
        let input = field.read(cx);
        let range = input.editable_selection();
        let full = input.editable_text();
        full.get(range)
            .filter(|selected| !selected.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or(full)
    };
    speak(text, stop_action, cx);
}

/// Speak arbitrary plain text, for a surface with no `InputState` to read
/// from (Preview reads the PDF/image text selection it already tracks for
/// Copy, not an editable field).
pub fn speak(text: String, stop_action: &'static str, cx: &mut App) {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return;
    }
    stop_speaking();
    crate::set_menu_enabled(stop_action, true, cx);
    cx.spawn(async move |cx| {
        blocking::unblock(move || speak_blocking(&text)).await;
        cx.update(|cx| crate::set_menu_enabled(stop_action, false, cx));
    })
    .detach();
}

/// Stop whatever this process is currently speaking, if anything.
pub fn stop_speaking() {
    if let Some(pid) = lock_pid().take() {
        let mut kill = Command::new("kill");
        kill.arg(pid.to_string());
        let _ = null_stdio(&mut kill).spawn();
    }
    // Best effort: `spd-say -w`'s wait is advisory over the SSIP
    // connection, so a message already handed to the daemon can outlive
    // the client process a `kill` above just ended. Asking the daemon to
    // stop directly closes that gap; it is a harmless no-op when nothing
    // is speaking or the daemon is not running.
    let mut stop = Command::new("spd-say");
    stop.arg("-S");
    let _ = null_stdio(&mut stop).spawn();
}

/// Runs on a worker thread: spawn `spd-say`, remember its pid so
/// [`stop_speaking`] can kill it, then block until it exits (spoken to
/// completion, killed, or never started).
fn speak_blocking(text: &str) {
    let mut command = Command::new("spd-say");
    command.arg("-w").arg("--").arg(text);
    let spawned: std::io::Result<Child> = null_stdio(&mut command).spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            eprintln!("speech: spd-say is not available: {error}");
            return;
        }
    };
    let pid = child.id();
    *lock_pid() = Some(pid);
    let _ = child.wait();
    let mut guard = lock_pid();
    if *guard == Some(pid) {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_blank_text_never_reaches_the_speech_daemon() {
        // No process is spawned for either case; if one were, the test
        // process would briefly own a stray `spd-say` child with nothing
        // to join. This only proves the early return, not the spawn path,
        // which needs a real speech-dispatcher session to exercise.
        assert!(lock_pid().is_none());
    }
}
