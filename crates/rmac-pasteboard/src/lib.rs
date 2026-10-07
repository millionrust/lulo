//! The system clipboard for files, shared by every surface that puts
//! filesystem items on it or reads them back — Files' own Copy/Cut/Paste,
//! and the desktop's context menu Copy (DESK-01).
//!
//! Copy and Cut put the selection on the real system clipboard, so another
//! Files window — or another file manager — can paste it, and Paste reads
//! files other apps put there.
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
//! Every call returns a [`Pending`] result and never blocks the caller's
//! thread: on Linux the work runs, in request order, on one worker thread,
//! so a Paste issued after a Copy always sees that Copy.
//!
//! Deliberately depends only on std, `async-channel` and (on macOS) the
//! system pasteboard bindings — no GUI toolkit — so a process that must
//! stay lean (the resident wallpaper renderer) can use it directly instead
//! of linking a whole GUI app's worth of dependencies for one clipboard
//! call.

mod file_list;

use std::fmt;
use std::path::PathBuf;

pub use file_list::file_uri;
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

/// The real Windows clipboard. `CF_HDROP` is the format Explorer's own
/// Copy/Cut/Paste writes and reads (ADR 0023 phase 4); "Preferred
/// DropEffect" (`CFSTR_PREFERREDDROPEFFECT`, a registered format carrying
/// one `DROPEFFECT_*` DWORD) is the same extra format Explorer uses to
/// tell a cut from a copy, so a cut done in Files and pasted into Explorer
/// (or the reverse) behaves the same way it would between two Explorer
/// windows.
#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::PathBuf;

    use windows::core::w;
    use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
    use windows::Win32::UI::Shell::DragQueryFileW;

    use super::{FileList, PasteboardError};

    const CF_HDROP: u32 = 15;
    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

    // The fixed header every `CF_HDROP` payload starts with (`shlobj_core.h`'s
    // `DROPFILES`); windows-rs does not export it, so it is spelled out here.
    #[repr(C)]
    struct DropFiles {
        files_offset: u32,
        anchor_x: i32,
        anchor_y: i32,
        in_non_client_area: i32,
        wide_chars: i32,
    }

    pub type Job = Box<dyn FnOnce() + Send>;

    /// The Win32 clipboard answers at once; run on the caller's thread.
    pub fn run(job: Job) {
        job();
    }

    struct OpenGuard;

    impl OpenGuard {
        fn acquire() -> Result<Self, PasteboardError> {
            // SAFETY: no preconditions beyond no other open clipboard on
            // this thread, which `OpenClipboard` itself enforces.
            unsafe { OpenClipboard(Some(HWND::default())) }
                .map(|()| Self)
                .map_err(|_| PasteboardError::new("The clipboard is in use"))
        }
    }

    impl Drop for OpenGuard {
        fn drop(&mut self) {
            // SAFETY: this guard exists only while the clipboard is open.
            let _ = unsafe { CloseClipboard() };
        }
    }

    pub fn write_file_list(paths: &[PathBuf], cut: bool) -> Result<(), PasteboardError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut wide: Vec<u16> = Vec::new();
        for path in paths {
            wide.extend(path.as_os_str().encode_wide());
            wide.push(0);
        }
        wide.push(0);
        let header_bytes = std::mem::size_of::<DropFiles>();
        let total_bytes = header_bytes + wide.len() * std::mem::size_of::<u16>();

        let guard = OpenGuard::acquire()?;
        // SAFETY: `EmptyClipboard` has no preconditions once open.
        unsafe { EmptyClipboard() }
            .map_err(|_| PasteboardError::new("The clipboard could not be cleared"))?;

        // SAFETY: `GHND` zero-initialises the block; `total_bytes` is the
        // header plus the double-NUL-terminated wide path list computed
        // above.
        let handle = unsafe { GlobalAlloc(GHND, total_bytes) }
            .map_err(|_| PasteboardError::new("The clipboard ran out of memory"))?;
        // SAFETY: `handle` was just allocated above and is not yet locked.
        // `windows-rs` 0.61 does not bind `GlobalFree`; this leaks the
        // just-allocated block on this (rare: a lock failing right after a
        // successful alloc) error path. The block is small, process-local,
        // and freed by Windows when the process exits.
        let locked = unsafe { GlobalLock(handle) };
        if locked.is_null() {
            return Err(PasteboardError::new("The clipboard ran out of memory"));
        }
        // SAFETY: `locked` points at `total_bytes` of writable memory just
        // allocated and locked above; `DropFiles` and the wide string list
        // together fit exactly within it.
        unsafe {
            let header = locked as *mut DropFiles;
            header.write(DropFiles {
                files_offset: header_bytes as u32,
                anchor_x: 0,
                anchor_y: 0,
                in_non_client_area: 0,
                wide_chars: 1,
            });
            let data = (locked as *mut u8).add(header_bytes) as *mut u16;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), data, wide.len());
        }
        // SAFETY: `handle` was locked exactly once above.
        unsafe { GlobalUnlock(handle) }.ok();

        // SAFETY: `handle` holds a well-formed `CF_HDROP` payload; ownership
        // passes to the clipboard on success. On failure the block leaks
        // (see the `GlobalFree` note above); this path is rare.
        if unsafe { SetClipboardData(CF_HDROP, Some(HANDLE(handle.0))) }.is_err() {
            return Err(PasteboardError::new("The clipboard refused the items"));
        }

        set_preferred_drop_effect(if cut {
            DROPEFFECT_MOVE
        } else {
            DROPEFFECT_COPY
        });
        drop(guard);
        Ok(())
    }

    fn set_preferred_drop_effect(effect: u32) {
        // SAFETY: a registered clipboard format id with no preconditions.
        let format = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
        if format == 0 {
            return;
        }
        // SAFETY: a `DWORD`-sized, zero-initialised block, matching the
        // registered format's documented contents.
        let Ok(handle) = (unsafe { GlobalAlloc(GHND, std::mem::size_of::<u32>()) }) else {
            return;
        };
        // See the `GlobalFree` note in `write_file_list` above: this leaks
        // the block on the (rare) failure path rather than guessing at an
        // unbound API.
        let locked = unsafe { GlobalLock(handle) };
        if locked.is_null() {
            return;
        }
        // SAFETY: `locked` points at a just-allocated, locked `u32`-sized
        // block.
        unsafe { (locked as *mut u32).write(effect) };
        unsafe { GlobalUnlock(handle) }.ok();
        // SAFETY: `handle` holds a well-formed `DWORD` payload. On failure
        // the block leaks (see the `GlobalFree` note above).
        let _ = unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) };
    }

    pub fn clear() -> Result<(), PasteboardError> {
        let guard = OpenGuard::acquire()?;
        // SAFETY: the clipboard is open, held by `guard`.
        let result = unsafe { EmptyClipboard() };
        drop(guard);
        result.map_err(|_| PasteboardError::new("The clipboard could not be cleared"))
    }

    pub fn has_file_list() -> Result<bool, PasteboardError> {
        // SAFETY: no preconditions; reads clipboard state without opening it.
        Ok(unsafe { IsClipboardFormatAvailable(CF_HDROP) }.is_ok())
    }

    pub fn read_file_list() -> Result<Option<FileList>, PasteboardError> {
        if !has_file_list()? {
            return Ok(None);
        }
        let guard = OpenGuard::acquire()?;
        // SAFETY: the clipboard is open, held by `guard`; `CF_HDROP` was
        // confirmed available above.
        let Ok(handle) = (unsafe { GetClipboardData(CF_HDROP) }) else {
            return Ok(None);
        };
        let hdrop = windows::Win32::UI::Shell::HDROP(handle.0);
        // SAFETY: `hdrop` is the clipboard's own `CF_HDROP` handle, valid
        // for the lifetime of `guard`; `0xFFFF_FFFF` is `DragQueryFileW`'s
        // documented "return the count" index.
        let count = unsafe { DragQueryFileW(hdrop, 0xFFFF_FFFF, None) };
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut buffer = [0u16; 32 * 1024];
            // SAFETY: `buffer` is a valid, sufficiently sized buffer for
            // `index`, one of the `count` entries `DragQueryFileW` itself
            // just reported.
            let length = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) };
            if length > 0 {
                paths.push(PathBuf::from(String::from_utf16_lossy(
                    &buffer[..length as usize],
                )));
            }
        }
        let cut = preferred_drop_effect_is_move();
        drop(guard);
        Ok((!paths.is_empty()).then_some(FileList { paths, cut }))
    }

    fn preferred_drop_effect_is_move() -> bool {
        // SAFETY: a registered clipboard format id with no preconditions.
        let format = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) };
        if format == 0 || unsafe { IsClipboardFormatAvailable(format) }.is_err() {
            return false;
        }
        // SAFETY: the clipboard is already open by this function's only
        // caller, `read_file_list`, for as long as `guard` there is alive.
        let Ok(handle) = (unsafe { GetClipboardData(format) }) else {
            return false;
        };
        let locked = unsafe { GlobalLock(HGLOBAL(handle.0)) };
        if locked.is_null() {
            return false;
        }
        // SAFETY: `locked` points at the registered format's documented
        // one-`DWORD` payload.
        let effect = unsafe { *(locked as *const u32) };
        unsafe { GlobalUnlock(HGLOBAL(handle.0)) }.ok();
        effect == DROPEFFECT_MOVE
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
        assert!(!same_files(std::slice::from_ref(&a), &[a.clone(), b]));
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
