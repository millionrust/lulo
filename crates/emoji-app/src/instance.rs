//! One panel at a time, toggled by the shortcut: the running panel's pid
//! lives in `$XDG_RUNTIME_DIR/rmac/emoji.pid`; a second launch signals it to
//! close and exits instead of opening another.

use std::path::PathBuf;

fn pid_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())?;
    Some(runtime.join("rmac/emoji.pid"))
}

fn is_running_panel(pid: i32) -> bool {
    let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
        return false;
    };
    name.trim() == "rmac-emoji"
}

/// Removes the pid file when the panel ends.
pub struct Claim {
    path: PathBuf,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub enum Launch {
    /// This process is the panel.
    Open(Claim),
    /// A panel was open and has been closed; `restore` is the window that
    /// had the keyboard before it opened, which the caller refocuses.
    Closed { restore: Option<u64> },
}

/// Open the panel, or close the one that is open (the shortcut toggles).
/// The pid file holds the panel's pid and the window to refocus.
pub fn claim(previous_window: Option<u64>) -> Launch {
    let Some(path) = pid_path() else {
        // No runtime directory: no toggling, just open.
        return Launch::Open(Claim {
            path: PathBuf::from("/nonexistent/rmac-emoji.pid"),
        });
    };
    let recorded = std::fs::read_to_string(&path).ok().and_then(|text| {
        let mut fields = text.split_whitespace();
        let pid = fields.next()?.parse::<i32>().ok()?;
        let window = fields.next().and_then(|field| field.parse::<u64>().ok());
        Some((pid, window))
    });
    if let Some((pid, window)) = recorded {
        if pid != std::process::id() as i32 && is_running_panel(pid) {
            // SAFETY: SIGTERM to a process just checked to be a panel of ours.
            let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
            let _ = std::fs::remove_file(&path);
            return Launch::Closed { restore: window };
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let window = previous_window.map_or(String::new(), |id| format!(" {id}"));
    let _ = std::fs::write(&path, format!("{}{window}", std::process::id()));
    Launch::Open(Claim { path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_process_named_rmac_emoji_counts_as_the_panel() {
        assert!(!is_running_panel(i32::MAX));
        assert!(!is_running_panel(1));
    }
}
