//! What Lulo does at sign-in when it is the user's shell instead of
//! Explorer (ADR 0023, "Lulo as the shell"): the decisions only, unit-tested
//! on every platform; `win::startup` and `win::session` carry them out.

use std::time::{Duration, Instant};

/// Whether a startup entry is turned on, by its `StartupApproved` value
/// (what Task Manager's Startup apps page writes): no value means on; the
/// first byte's low bit set (03, 07) means turned off.
pub fn approved(value: Option<&[u8]>) -> bool {
    value
        .and_then(|bytes| bytes.first())
        .is_none_or(|first| first & 1 == 0)
}

/// Lulo's own entry (an older "Start Lulo at Sign-In" in the `Run` key):
/// never started again by Lulo itself.
pub fn is_lulo(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("lulo-session.exe") || lower.contains("lulo-shell.exe")
}

/// The `Winlogon\Shell` value that makes Lulo the user's shell: the
/// session, quoted, in shell mode.
pub fn shell_command(session: &str) -> String {
    format!("\"{session}\" {}", AS_SHELL_SWITCH)
}

/// `lulo-session`'s switch when Windows starts it as the shell.
pub const AS_SHELL_SWITCH: &str = "--as-shell";

/// Whether a `Winlogon\Shell` value is Lulo's (so turning it off may
/// remove it; anyone else's is left alone).
pub fn is_lulo_shell(value: &str) -> bool {
    is_lulo(value) && value.contains(AS_SHELL_SWITCH)
}

/// How many times the shell may stop by itself before the session gives
/// the desktop to Explorer.
pub const FAILURES_BEFORE_EXPLORER: usize = 2;
/// Failures further apart than this are not counted together.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(120);

/// The watchdog's record of `lulo-shell` stopping on its own.
#[derive(Debug, Default)]
pub struct Watchdog {
    failures: Vec<Instant>,
}

impl Watchdog {
    /// `lulo-shell` stopped at `at` without being asked to; true when the
    /// session should now start Explorer instead of trying again.
    pub fn failed(&mut self, at: Instant) -> bool {
        self.failures
            .retain(|failure| at.duration_since(*failure) < FAILURE_WINDOW);
        self.failures.push(at);
        self.failures.len() >= FAILURES_BEFORE_EXPLORER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_manager_s_switch_decides_what_starts() {
        assert!(approved(None));
        assert!(approved(Some(&[0x02, 0, 0, 0])));
        assert!(approved(Some(&[0x06, 0, 0, 0])));
        assert!(!approved(Some(&[0x03, 0, 0, 0])));
        assert!(!approved(Some(&[0x07])));
        assert!(approved(Some(&[])));
    }

    #[test]
    fn lulo_never_starts_itself_twice() {
        assert!(is_lulo(
            r#""C:\Users\a\AppData\Local\Programs\Lulo\lulo-session.exe""#
        ));
        assert!(!is_lulo(
            r#""C:\Program Files\Microsoft OneDrive\OneDrive.exe" /background"#
        ));
    }

    #[test]
    fn the_shell_value_is_recognised_as_lulo_s() {
        let command = shell_command(r"C:\Lulo\lulo-session.exe");
        assert_eq!(command, r#""C:\Lulo\lulo-session.exe" --as-shell"#);
        assert!(is_lulo_shell(&command));
        assert!(!is_lulo_shell("explorer.exe"));
        assert!(!is_lulo_shell(r#""C:\Lulo\lulo-session.exe""#));
    }

    #[test]
    fn two_failures_close_together_give_the_desktop_to_explorer() {
        let start = Instant::now();
        let mut watchdog = Watchdog::default();
        assert!(!watchdog.failed(start));
        assert!(watchdog.failed(start + Duration::from_secs(10)));
        let mut apart = Watchdog::default();
        assert!(!apart.failed(start));
        assert!(!apart.failed(start + FAILURE_WINDOW + Duration::from_secs(1)));
    }
}
