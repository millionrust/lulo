//! The system clipboard for files.
//!
//! Copy and Cut in Files put the selection on the real system clipboard, so
//! another Files window — or another file manager — can paste it, and Paste
//! reads files other apps put there.
//!
//! * **Linux**: Wayland selection through wl-clipboard (ADR 0011). Copy
//!   offers `text/uri-list` (wl-copy adds the plain-text types, so a text
//!   field pastes the URIs); Cut offers `x-special/gnome-copied-files`
//!   with `cut`, which Nautilus and Thunar move on paste. wl-copy keeps
//!   serving the selection after this window closes, like the Mac
//!   pasteboard. Paste reads GNOME's list, or a `text/uri-list` with KDE's
//!   `application/x-kde-cutselection` marker.
//! * **macOS**: `NSPasteboard` file URLs.
//!
//! Every call returns a [`Pending`] result and never blocks the UI thread:
//! on Linux the work runs, in request order, on one worker thread, so a
//! Paste issued after a Copy always sees that Copy.

mod file_list;

use std::fmt;
use std::path::PathBuf;

pub use file_list::FileList;

/// A clipboard failure, worded for the Files error banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteboardError(String);

impl PasteboardError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PasteboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The outcome of a clipboard request that is still running.
pub struct Pending<T>(async_channel::Receiver<Result<T, PasteboardError>>);

impl<T> Pending<T> {
    pub async fn wait(self) -> Result<T, PasteboardError> {
        self.0
            .recv()
            .await
            .unwrap_or_else(|_| Err(PasteboardError::new("The clipboard stopped responding")))
    }
}

fn submit<T: Send + 'static>(
    request: impl FnOnce() -> Result<T, PasteboardError> + Send + 'static,
) -> Pending<T> {
    let (reply, pending) = async_channel::bounded(1);
    let job: imp::Job = Box::new(move || {
        // The window may have closed while this ran; nobody is left to tell.
        let _ = reply.send_blocking(request());
    });
    imp::run(job);
    Pending(pending)
}

/// Put `paths` on the clipboard; `cut` makes a paste move them.
pub fn write_file_list(paths: Vec<PathBuf>, cut: bool) -> Pending<()> {
    submit(move || imp::write_file_list(&paths, cut))
}

/// The files on the clipboard, if it holds any.
pub fn read_file_list() -> Pending<Option<FileList>> {
    submit(imp::read_file_list)
}

/// Whether the clipboard offers files, without reading them.
pub fn has_file_list() -> Pending<bool> {
    submit(imp::has_file_list)
}

/// Empty the clipboard after a cut of `paths` was pasted. Anything else on
/// it (a later copy, or the same files copied rather than cut) stays.
pub fn clear_file_list_if(paths: Vec<PathBuf>) -> Pending<()> {
    submit(move || {
        let cut_of_these = |list: FileList| list.cut && same_files(&list.paths, &paths);
        if imp::read_file_list()?.is_some_and(cut_of_these) {
            imp::clear()
        } else {
            Ok(())
        }
    })
}

/// The same files regardless of order and duplicates.
pub fn same_files(left: &[PathBuf], right: &[PathBuf]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    left.dedup();
    right.sort();
    right.dedup();
    left == right
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{ErrorKind, Read as _, Write as _};
    use std::path::PathBuf;
    use std::process::{Command, ExitStatus, Stdio};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};

    use super::file_list::{self, FileList, GNOME_COPIED_FILES, KDE_CUT_SELECTION, URI_LIST};
    use super::PasteboardError;

    pub type Job = Box<dyn FnOnce() + Send>;

    const WL_COPY: &str = "wl-copy";
    const WL_PASTE: &str = "wl-paste";
    /// Every read is bounded in time: an app that offers files and never
    /// writes them must not hang Paste.
    const TIMEOUT: &str = "timeout";
    const READ_SECONDS: &str = "5";
    /// `timeout`'s exit status when time ran out, and when the command is
    /// missing.
    const TIMED_OUT: i32 = 124;
    const NOT_FOUND: i32 = 127;
    /// File lists above this are refused rather than truncated (ADR 0011's
    /// file-list bound).
    const READ_LIMIT: u64 = 1024 * 1024;

    const MISSING: &str =
        "Copying files needs wl-clipboard (wl-copy and wl-paste); install the wl-clipboard package";

    /// One worker, so requests run in the order Files made them. It sleeps
    /// in `recv` between requests.
    pub fn run(job: Job) {
        static WORKER: OnceLock<Mutex<Option<mpsc::Sender<Job>>>> = OnceLock::new();
        let worker = WORKER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new()
                .name("files-clipboard".into())
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        job();
                    }
                });
            Mutex::new(spawned.ok().map(|_| sender))
        });
        let job = match worker.lock() {
            Ok(sender) => match sender.as_ref() {
                Some(sender) => match sender.send(job) {
                    Ok(()) => return,
                    Err(mpsc::SendError(job)) => job,
                },
                None => job,
            },
            Err(_) => job,
        };
        // No worker thread: answer here rather than never.
        job();
    }

    fn spawn_error(program: &str, error: std::io::Error) -> PasteboardError {
        if error.kind() == ErrorKind::NotFound {
            PasteboardError::new(MISSING)
        } else {
            PasteboardError::new(format!("Could not start {program}: {error}"))
        }
    }

    fn check_timeout(status: ExitStatus) -> Result<ExitStatus, PasteboardError> {
        match status.code() {
            Some(NOT_FOUND) => Err(PasteboardError::new(MISSING)),
            Some(TIMED_OUT) => Err(PasteboardError::new(
                "The app that copied these items did not hand them over in time",
            )),
            _ => Ok(status),
        }
    }

    fn copy(mime: &str, bytes: &[u8]) -> Result<(), PasteboardError> {
        let mut child = Command::new(WL_COPY)
            .args(["--type", mime])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| spawn_error(WL_COPY, error))?;
        let written = match child.stdin.take() {
            Some(mut stdin) => stdin.write_all(bytes),
            None => Err(ErrorKind::BrokenPipe.into()),
        };
        // wl-copy returns once the compositor has made it the selection
        // owner; a forked copy keeps serving it after that.
        let status = child
            .wait()
            .map_err(|error| PasteboardError::new(format!("wl-copy failed: {error}")))?;
        match (written, status.success()) {
            (Ok(()), true) => Ok(()),
            (Err(error), _) => Err(PasteboardError::new(format!(
                "Could not hand the items to the clipboard: {error}"
            ))),
            (Ok(()), false) => Err(PasteboardError::new(
                "The clipboard refused the items (is this a Wayland session?)",
            )),
        }
    }

    pub fn clear() -> Result<(), PasteboardError> {
        let status = Command::new(WL_COPY)
            .arg("--clear")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| spawn_error(WL_COPY, error))?;
        if status.success() {
            Ok(())
        } else {
            Err(PasteboardError::new("Could not empty the clipboard"))
        }
    }

    /// MIME types the current selection offers, one per line from
    /// `wl-paste --list-types`; empty when nothing is copied.
    fn offered_types() -> Result<Vec<String>, PasteboardError> {
        let output = Command::new(TIMEOUT)
            .args([READ_SECONDS, WL_PASTE, "--list-types"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| spawn_error(TIMEOUT, error))?;
        // wl-paste exits non-zero when nothing is copied.
        if !check_timeout(output.status)?.success() {
            return Ok(Vec::new());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// The selection as exactly `mime`, bounded in time and size.
    fn paste(mime: &str) -> Result<Vec<u8>, PasteboardError> {
        let mut child = Command::new(TIMEOUT)
            .args([READ_SECONDS, WL_PASTE, "--no-newline", "--type", mime])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| spawn_error(TIMEOUT, error))?;
        let mut bytes = Vec::new();
        let read = match child.stdout.take() {
            Some(stdout) => stdout.take(READ_LIMIT + 1).read_to_end(&mut bytes),
            None => Err(ErrorKind::BrokenPipe.into()),
        };
        if read.is_err() || bytes.len() as u64 > READ_LIMIT {
            // Stop a writer that keeps going; its exit status no longer
            // matters once the read is abandoned.
            let _ = child.kill();
            let _ = child.wait();
            return Err(match read {
                Err(error) => {
                    PasteboardError::new(format!("Could not read the clipboard: {error}"))
                }
                Ok(_) => PasteboardError::new("The clipboard holds too many items to paste"),
            });
        }
        let status = child
            .wait()
            .map_err(|error| PasteboardError::new(format!("wl-paste failed: {error}")))?;
        if check_timeout(status)?.success() {
            Ok(bytes)
        } else {
            Err(PasteboardError::new(
                "The app that copied these items stopped offering them",
            ))
        }
    }

    fn parse_error(error: file_list::ParseError) -> PasteboardError {
        PasteboardError::new(format!("Can’t paste: {error}"))
    }

    pub fn write_file_list(paths: &[PathBuf], cut: bool) -> Result<(), PasteboardError> {
        if paths.is_empty() {
            return Ok(());
        }
        // wl-copy offers one type per selection. A copy is the widely read
        // `text/uri-list`; a cut needs GNOME's list to say "cut", which
        // Nautilus and Thunar honour (Dolphin reads only the URI list, so
        // it cannot paste a cut made here).
        if cut {
            copy(
                GNOME_COPIED_FILES,
                file_list::format_gnome_copied_files(paths, true).as_bytes(),
            )
        } else {
            copy(URI_LIST, file_list::format_uri_list(paths).as_bytes())
        }
    }

    pub fn has_file_list() -> Result<bool, PasteboardError> {
        let types = offered_types()?;
        Ok(types
            .iter()
            .any(|mime| mime == GNOME_COPIED_FILES || mime == URI_LIST))
    }

    pub fn read_file_list() -> Result<Option<FileList>, PasteboardError> {
        let types = offered_types()?;
        let offers = |wanted: &str| types.iter().any(|mime| mime == wanted);
        let list = if offers(GNOME_COPIED_FILES) {
            file_list::parse_gnome_copied_files(&paste(GNOME_COPIED_FILES)?).map_err(parse_error)?
        } else if offers(URI_LIST) {
            let paths = file_list::parse_uri_list(&paste(URI_LIST)?).map_err(parse_error)?;
            let cut = offers(KDE_CUT_SELECTION)
                && file_list::parse_kde_cut_selection(&paste(KDE_CUT_SELECTION)?);
            FileList { paths, cut }
        } else {
            return Ok(None);
        };
        Ok((!list.paths.is_empty()).then_some(list))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::ClassType;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    use super::{FileList, PasteboardError};

    pub type Job = Box<dyn FnOnce() + Send>;

    /// `NSPasteboard` answers at once; run on the caller's thread.
    pub fn run(job: Job) {
        job();
    }

    /// Write the given file paths to the general pasteboard as `file://`
    /// URLs. The pasteboard has no notion of cut: Files remembers that
    /// itself.
    pub fn write_file_list(paths: &[PathBuf], _cut: bool) -> Result<(), PasteboardError> {
        if paths.is_empty() {
            return Ok(());
        }
        let objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
            .iter()
            .map(|p| {
                let s = NSString::from_str(&p.to_string_lossy());
                let url = NSURL::fileURLWithPath(&s);
                ProtocolObject::from_retained(url)
            })
            .collect();
        let array = NSArray::from_retained_slice(&objs);
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        if pb.writeObjects(&array) {
            Ok(())
        } else {
            Err(PasteboardError::new("The pasteboard refused the items"))
        }
    }

    pub fn clear() -> Result<(), PasteboardError> {
        NSPasteboard::generalPasteboard().clearContents();
        Ok(())
    }

    pub fn has_file_list() -> Result<bool, PasteboardError> {
        Ok(read_file_list()?.is_some())
    }

    /// Read any `file://` URLs currently on the general pasteboard.
    pub fn read_file_list() -> Result<Option<FileList>, PasteboardError> {
        let pb = NSPasteboard::generalPasteboard();
        let classes = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: `classes` holds the `NSURL` class and no read options are
        // passed, matching the documented contract.
        let Some(objs) = (unsafe { pb.readObjectsForClasses_options(&classes, None) }) else {
            return Ok(None);
        };
        let mut paths = Vec::new();
        for obj in objs.iter() {
            if let Ok(url) = obj.downcast::<NSURL>() {
                if let Some(path) = url.path() {
                    paths.push(PathBuf::from(path.to_string()));
                }
            }
        }
        Ok((!paths.is_empty()).then_some(FileList { paths, cut: false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_files_ignores_order_and_duplicates() {
        let a = PathBuf::from("/tmp/a");
        let b = PathBuf::from("/tmp/b");
        assert!(same_files(
            &[a.clone(), b.clone()],
            &[b.clone(), a.clone(), a.clone()]
        ));
        assert!(!same_files(std::slice::from_ref(&a), &[a, b]));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn round_trips_file_urls_through_the_system_pasteboard() {
        // Two real paths that exist on every macOS box.
        let want = vec![PathBuf::from("/tmp"), PathBuf::from("/usr/bin/true")];
        imp::write_file_list(&want, false).unwrap();
        let got = imp::read_file_list().unwrap().unwrap().paths;
        // The pasteboard normalises /tmp → /private/tmp; compare by file name +
        // existence rather than exact string.
        assert_eq!(got.len(), want.len(), "got {got:?}");
        assert!(
            got.iter().all(|p| p.exists()),
            "all read paths exist: {got:?}"
        );
        assert!(got.iter().any(|p| p.ends_with("true")));
        imp::clear().unwrap();
        assert!(imp::read_file_list().unwrap().is_none());
    }
}
