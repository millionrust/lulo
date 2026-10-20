//! The screen recording the menu bar's stop button follows (SHOT-01).
//!
//! `rmac-screenshot` writes `$XDG_RUNTIME_DIR/rmac/screen-recording` (the
//! start time in Unix milliseconds on its first line) while it records and
//! removes it when the file is finished. An inotify watch on that folder
//! wakes this when it changes; nothing polls.

use std::path::{Path, PathBuf};

use async_channel::Sender;
use notify::{RecursiveMode, Watcher as _};
use rmac_shell_status::ScreenRecordingIndicator;

use crate::Error;

const STATE_FILE: &str = "screen-recording";

fn state_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|runtime| runtime.join("rmac"))
}

pub(crate) fn parse(text: &str) -> Option<ScreenRecordingIndicator> {
    let started_unix_ms = text.lines().next()?.trim().parse().ok()?;
    Some(ScreenRecordingIndicator { started_unix_ms })
}

fn read(directory: &Path) -> Option<ScreenRecordingIndicator> {
    parse(&std::fs::read_to_string(directory.join(STATE_FILE)).ok()?)
}

/// Publish the recording state whenever it changes. A machine that cannot
/// watch the folder simply never shows the button: the shell keeps running.
pub async fn watch(sender: Sender<Option<ScreenRecordingIndicator>>) -> Result<(), Error> {
    let Some(directory) = state_directory() else {
        futures_util::future::pending::<()>().await;
        return Ok(());
    };
    let (wake, woken) = async_channel::bounded::<()>(1);
    let setup = {
        let directory = directory.clone();
        blocking::unblock(move || {
            std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.is_ok() {
                        let _ = wake.try_send(());
                    }
                })
                .map_err(|error| error.to_string())?;
            watcher
                .watch(&directory, RecursiveMode::NonRecursive)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(watcher)
        })
        .await
    };
    let _watcher = match setup {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("the screen recording button cannot watch for recordings: {error}");
            futures_util::future::pending::<()>().await;
            return Ok(());
        }
    };
    let mut last = {
        let directory = directory.clone();
        blocking::unblock(move || read(&directory)).await
    };
    if last.is_some() && sender.send(last).await.is_err() {
        return Ok(());
    }
    loop {
        if woken.recv().await.is_err() {
            return Ok(());
        }
        let now = {
            let directory = directory.clone();
            blocking::unblock(move || read(&directory)).await
        };
        if now != last {
            last = now;
            if sender.send(now).await.is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_line_is_the_start_time() {
        assert_eq!(
            parse("1700000000000\n/home/u/Desktop/Screen Recording.mov\n"),
            Some(ScreenRecordingIndicator {
                started_unix_ms: 1_700_000_000_000
            })
        );
        assert_eq!(parse(""), None);
        assert_eq!(parse("soon\n"), None);
    }
}
