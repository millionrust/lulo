//! Have Windows check an app before it is opened (WIN-OS-99).
//!
//! Microsoft Defender scans an exe it has not cached before Windows lets its
//! process start: 0.5 to 0.7 s for a 20 MB Lulo app on the reference laptop,
//! more than the app then takes to draw its first frame. Its verdict is kept
//! for a while. A Mac opens an app from the Dock with no such wait, so the
//! Dock, Spotlight and Apps ask for the scan the moment an app is about to
//! be opened (the pointer reaches its Dock tile, it becomes Spotlight's
//! selection) by creating the app's process suspended, where none of its
//! code runs, and ending it at once. The open that follows a moment later
//! starts in milliseconds.
//!
//! Nothing runs at idle: each call is one user action, an app is warmed at
//! most once per [`REWARM_AFTER`], and only an app that is not already
//! running. Elsewhere this does nothing.

use rmac_apps::LaunchSpec;

/// Defender keeps its verdict longer than this on the reference laptop.
#[cfg(windows)]
const REWARM_AFTER: std::time::Duration = std::time::Duration::from_secs(120);

/// Warm `spec`'s program in the background (see the module docs).
pub fn prewarm(spec: &LaunchSpec) {
    #[cfg(windows)]
    {
        let LaunchSpec::Command { program, .. } = spec else {
            return;
        };
        let program = std::path::PathBuf::from(program);
        if !due(&program, std::time::Instant::now()) {
            return;
        }
        blocking::unblock(move || warm(&program)).detach();
    }
    #[cfg(not(windows))]
    let _ = spec;
}

/// Whether `program` was not warmed within [`REWARM_AFTER`] of `now`;
/// records `now` when it was not.
#[cfg(windows)]
fn due(program: &std::path::Path, now: std::time::Instant) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static WARMED: OnceLock<Mutex<HashMap<std::path::PathBuf, std::time::Instant>>> =
        OnceLock::new();
    let Ok(mut warmed) = WARMED.get_or_init(Default::default).lock() else {
        return false;
    };
    match warmed.get(program) {
        Some(at) if now.duration_since(*at) < REWARM_AFTER => false,
        _ => {
            warmed.insert(program.to_owned(), now);
            true
        }
    }
}

#[cfg(windows)]
fn warm(program: &std::path::Path) {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    // CREATE_SUSPENDED: the process's first thread never starts, so not
    // one instruction of the app runs; creating it is what makes Windows
    // scan the exe.
    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    // A bare exe name is a Lulo app beside the shell's own exe.
    let program = if program.is_absolute() {
        program.to_owned()
    } else {
        match std::env::current_exe() {
            Ok(exe) => match exe.parent() {
                Some(folder) => folder.join(program),
                None => return,
            },
            Err(_) => return,
        }
    };
    if !program.is_file() {
        return;
    }
    let Ok(mut child) = Command::new(&program)
        .creation_flags(CREATE_SUSPENDED)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_warmed_once_until_the_verdict_may_have_lapsed() {
        let program = std::path::Path::new(r"C:\Lulo\rmac-test-prewarm.exe");
        let start = std::time::Instant::now();
        assert!(due(program, start));
        assert!(!due(program, start + std::time::Duration::from_secs(5)));
        assert!(due(program, start + REWARM_AFTER));
    }
}
