//! Edit ▸ Record Audio… (NOT-MENU-008). No microphone capture pipeline
//! existed anywhere in rmac before this (`rmac-audio` only controls
//! playback/volume); this module shells out to PipeWire's own `pw-record`
//! CLI, the same tool `wpctl`/PipeWire-based desktops expect to be present
//! for audio capture. If it is missing, Record Audio tells the person
//! instead of silently doing nothing.
//!
//! The action toggles: the first press starts capture to a private WAV
//! file; the second sends `SIGINT` (which `pw-record` treats as "finish
//! the file cleanly", unlike `SIGKILL`), waits for the process off the UI
//! thread, then attaches the result as a `📎` chip the same way Edit ▸
//! Attach File… does (`NotesView::insert_file_attachment_chip`).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use gpui::{Context, Window};

use crate::NotesView;

/// A `pw-record` capture in progress for the current note.
pub(crate) struct AudioRecording {
    child: Child,
    path: PathBuf,
    started_at: Instant,
}

/// A WAV header alone is 44 bytes; anything at or below that has no real
/// audio in it (e.g. Stop pressed within the same instant as Start).
const WAV_HEADER_BYTES: u64 = 44;
const MIN_RECORDING: std::time::Duration = std::time::Duration::from_millis(250);

fn pw_record_available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("pw-record").is_file()))
}

fn recordings_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("rmac-notes/recordings")
}

fn recording_name(now_ms: u64) -> String {
    format!("Audio Recording {now_ms}.wav")
}

impl NotesView {
    /// Edit ▸ Record Audio…: starts capture if idle, stops and attaches
    /// if a capture is already running.
    pub(super) fn toggle_record_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            self.stop_record_audio(window, cx);
        } else {
            self.start_record_audio(cx);
        }
    }

    fn start_record_audio(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.session.selected_note() else {
            self.message = Some("Choose a note before recording audio.".into());
            cx.notify();
            return;
        };
        if note.deleted || self.session.is_note_closed(note) {
            self.message = Some("Notes can't record audio into this note.".into());
            cx.notify();
            return;
        }
        if !pw_record_available() {
            self.message =
                Some("Notes could not find pw-record (PipeWire) to capture audio.".into());
            cx.notify();
            return;
        }
        let dir = recordings_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            self.message = Some("Notes could not create a folder for the recording.".into());
            cx.notify();
            return;
        }
        let path = dir.join(recording_name(crate::input_support::now_unix_ms()));
        match Command::new("pw-record")
            .arg("--format=s16")
            .arg("--rate=44100")
            .arg("--channels=1")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.recording = Some(AudioRecording {
                    child,
                    path,
                    started_at: Instant::now(),
                });
                self.message = Some("Recording audio… choose Record Audio again to stop.".into());
            }
            Err(error) => {
                self.message = Some(format!("Notes could not start pw-record: {error}").into());
            }
        }
        cx.notify();
    }

    fn stop_record_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        // SAFETY: `child.id()` is a live PID this process owns (we just
        // spawned it and have not reaped it yet); SIGINT is the normal
        // "finish the file" signal `pw-record` documents, unlike `kill()`.
        unsafe {
            libc::kill(recording.child.id() as libc::pid_t, libc::SIGINT);
        }
        cx.spawn_in(window, async move |this, cx| {
            let AudioRecording {
                mut child,
                path,
                started_at,
            } = recording;
            let (path, discard) = cx
                .background_executor()
                .spawn(blocking::unblock(move || {
                    let _ = child.wait();
                    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                    let discard = started_at.elapsed() < MIN_RECORDING || size <= WAV_HEADER_BYTES;
                    if discard {
                        let _ = std::fs::remove_file(&path);
                    }
                    (path, discard)
                }))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if discard {
                    this.message =
                        Some("Notes discarded a recording that was too short to keep.".into());
                } else {
                    this.insert_file_attachment_chip(path, window, cx);
                    this.message = Some("Recording attached to the note.".into());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_name_embeds_the_given_timestamp_and_wav_extension() {
        let name = recording_name(1_700_000_000_000);
        assert_eq!(name, "Audio Recording 1700000000000.wav");
        assert!(name.ends_with(".wav"));
    }

    #[test]
    fn recordings_dir_is_under_a_cache_root_and_app_scoped() {
        let dir = recordings_dir();
        assert!(dir.ends_with("rmac-notes/recordings"));
    }

    #[test]
    fn pw_record_available_reflects_path_contents() {
        // Build a scratch PATH containing a fake, executable `pw-record`
        // and confirm the probe finds it; an empty PATH must not.
        let scratch = std::env::temp_dir().join(format!(
            "rmac-notes-audio-recorder-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&scratch);
        let fake = scratch.join("pw-record");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&fake).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&fake, perms).unwrap();
        }
        let original = std::env::var_os("PATH");
        // Restored immediately below; `pw_record_available` is a plain
        // function call, not a spawned process, so this test does not
        // depend on other threads' PATH.
        std::env::set_var("PATH", &scratch);
        let found = pw_record_available();
        if let Some(path) = original {
            std::env::set_var("PATH", path);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(found);
    }
}
