//! View ▸ Sample Process (MON-10/MON-14, MON-MENU-031): a real stack
//! sample, honestly labelled by whichever source actually produced it.
//! Linux gives no `gdb`-free way to read another process's full call stack
//! as text; this collects what genuinely is available without any kernel
//! debug symbols:
//!
//! - `eu-stack -p <pid>` (elfutils), when installed — the closest match to
//!   the Mac's own sample, a real unwound stack per thread.
//! - Otherwise `/proc/<pid>/task/<tid>/wchan` and `/proc/<pid>/task/<tid>/
//!   stat` for every thread — the kernel function each thread is blocked
//!   in, which is real information even though it is not a full stack.
//!
//! If neither source can be read (process gone, or wchan unreadable under
//! a hardened kernel), the report says so plainly rather than fabricating
//! sample lines.

use std::fmt::Write as _;
use std::fs;
use std::process::Command;

/// One thread's real sampled state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ThreadSample {
    pub(crate) tid: u32,
    /// The thread's `comm` name (`/proc/<pid>/task/<tid>/comm`), when read.
    pub(crate) name: Option<String>,
    /// The kernel function the thread is blocked in ("0" means running).
    pub(crate) wchan: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SampleSource {
    /// `eu-stack`'s raw stdout, one real unwound stack per thread.
    EuStack(String),
    /// Per-thread `wchan` readings, when `eu-stack` is not installed.
    Wchan(Vec<ThreadSample>),
    /// Neither source could be read.
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SampleReport {
    pub(crate) pid: u32,
    pub(crate) process_name: String,
    pub(crate) source: SampleSource,
}

/// Pure formatting of an already-collected sample into the report text the
/// window displays — kept separate from `collect` so it is testable
/// without a real `/proc` or `eu-stack` binary.
pub(crate) fn format_report(report: &SampleReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Sample of {} (PID {})",
        report.process_name, report.pid
    );
    let _ = writeln!(out);
    match &report.source {
        SampleSource::EuStack(text) => {
            let _ = writeln!(out, "Source: eu-stack (elfutils)");
            let _ = writeln!(out);
            out.push_str(text);
        }
        SampleSource::Wchan(threads) => {
            let _ = writeln!(
                out,
                "Source: per-thread kernel wait channel (eu-stack is not installed, so this is not a full unwound call stack)"
            );
            let _ = writeln!(out);
            if threads.is_empty() {
                let _ = writeln!(out, "No threads could be read.");
            }
            for thread in threads {
                let name = thread.name.as_deref().unwrap_or("?");
                let wchan = thread.wchan.as_deref().unwrap_or("unknown");
                let state = if wchan == "0" {
                    "running (not blocked in the kernel)".to_string()
                } else {
                    format!("blocked in {wchan}")
                };
                let _ = writeln!(out, "Thread {} \"{name}\": {state}", thread.tid);
            }
        }
        SampleSource::Unavailable(reason) => {
            let _ = writeln!(out, "Sample unavailable: {reason}");
        }
    }
    out
}

/// Whether `eu-stack` is installed, by actually trying to run it with
/// `--version` rather than guessing from `PATH` — the same technique
/// `escalate`/`pkexec` detection in `signal_escalation.rs` relies on
/// (`NotFound` is unambiguous).
fn eu_stack_available() -> bool {
    Command::new("eu-stack").arg("--version").output().is_ok()
}

fn run_eu_stack(pid: u32) -> Option<String> {
    let output = Command::new("eu-stack")
        .arg("-p")
        .arg(pid.to_string())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            text.push_str("\n(eu-stack reported: ");
            text.push_str(stderr.trim());
            text.push(')');
        }
    }
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn thread_ids(pid: u32) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    let mut tids: Vec<u32> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect();
    tids.sort_unstable();
    tids
}

fn read_wchan_sample(pid: u32, tid: u32) -> ThreadSample {
    let name = fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm"))
        .ok()
        .map(|s| s.trim().to_string());
    let wchan = fs::read_to_string(format!("/proc/{pid}/task/{tid}/wchan"))
        .ok()
        .map(|s| s.trim().to_string());
    ThreadSample { tid, name, wchan }
}

/// Collect a real sample of `pid`. Blocking (spawns a process and/or reads
/// several `/proc` files) — the caller must run this off the UI thread.
pub(crate) fn collect(pid: u32, process_name: String) -> SampleReport {
    let source = if eu_stack_available() {
        match run_eu_stack(pid) {
            Some(text) => SampleSource::EuStack(text),
            None => SampleSource::Unavailable(
                "eu-stack is installed but produced no output for this process (it may have \
                 exited, or may not be ptrace-accessible)."
                    .to_string(),
            ),
        }
    } else {
        let tids = thread_ids(pid);
        if tids.is_empty() {
            SampleSource::Unavailable(format!(
                "eu-stack is not installed, and /proc/{pid}/task could not be read (the \
                 process may have exited, or may not be visible to this account)."
            ))
        } else {
            SampleSource::Wchan(
                tids.into_iter()
                    .map(|tid| read_wchan_sample(pid, tid))
                    .collect(),
            )
        }
    };
    SampleReport {
        pid,
        process_name,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eu_stack_report_includes_its_raw_text() {
        let report = SampleReport {
            pid: 42,
            process_name: "worker".into(),
            source: SampleSource::EuStack("TID 42:\n#0 0x1234 some_fn\n".into()),
        };
        let text = format_report(&report);
        assert!(text.contains("Sample of worker (PID 42)"));
        assert!(text.contains("eu-stack"));
        assert!(text.contains("some_fn"));
    }

    #[test]
    fn wchan_report_distinguishes_running_from_blocked_threads() {
        let report = SampleReport {
            pid: 42,
            process_name: "worker".into(),
            source: SampleSource::Wchan(vec![
                ThreadSample {
                    tid: 42,
                    name: Some("worker".into()),
                    wchan: Some("0".into()),
                },
                ThreadSample {
                    tid: 43,
                    name: Some("worker-io".into()),
                    wchan: Some("pipe_wait".into()),
                },
            ]),
        };
        let text = format_report(&report);
        assert!(text.contains("running (not blocked in the kernel)"));
        assert!(text.contains("blocked in pipe_wait"));
        assert!(text.contains("worker-io"));
    }

    #[test]
    fn unavailable_report_never_fabricates_a_stack() {
        let report = SampleReport {
            pid: 42,
            process_name: "worker".into(),
            source: SampleSource::Unavailable("process exited".into()),
        };
        let text = format_report(&report);
        assert!(text.contains("Sample unavailable: process exited"));
        assert!(!text.contains("#0"));
    }

    #[test]
    fn empty_wchan_sample_set_says_so() {
        let report = SampleReport {
            pid: 1,
            process_name: "init".into(),
            source: SampleSource::Wchan(Vec::new()),
        };
        assert!(format_report(&report).contains("No threads could be read"));
    }
}
