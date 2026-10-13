//! Opening apps and files. `ShellExecute` can take a moment (it may load
//! shell extensions or start a Store app), so it runs on its own thread
//! with COM initialised, never on the UI thread.
//!
//! An app the user opens from the Dock, Spotlight or the desktop must come
//! to the front, even with another app maximised in front (WIN-OS-45).
//! Windows lets the process that received the click or key hand the
//! foreground on, so the shell allows it to every process before the
//! launch, and reports each launch (`Launched`) so the UI can bring the
//! app's new window forward itself once it appears.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Com::{
    CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL};

use super::trace;

#[derive(Clone, Debug)]
pub enum Request {
    /// A Lulo app's executable name, beside `lulo-shell.exe`.
    Lulo(String),
    /// A Lulo app with arguments (Files opening a folder).
    LuloWith(String, Vec<String>),
    /// Anything `ShellExecute` opens.
    Shell(String),
}

/// A launch that went through: the executable's lower-case file name, as
/// the Dock keys apps, when it is known.
#[derive(Clone, Debug)]
pub struct Launched {
    pub key: String,
}

static WORKER: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();
static LAUNCHED: OnceLock<async_channel::Sender<Launched>> = OnceLock::new();

/// Report each launch on `sender` (the UI brings the app forward).
pub fn report_launches(sender: async_channel::Sender<Launched>) {
    let _ = LAUNCHED.set(sender);
}

/// Lulo's Files, beside `lulo-shell.exe`.
pub const FILES_EXE: &str = "rmac-files.exe";

/// The folder `lulo-shell.exe` runs from, where the Lulo apps are.
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_default()
}

/// Open the folder `path` in Lulo's Files, as the Finder opens folders on
/// the Mac (Explorer when Files is not installed beside the layer).
pub fn folder_request(path: &str) -> Request {
    if install_dir().join(FILES_EXE).is_file() {
        Request::LuloWith(FILES_EXE.into(), vec!["--path".into(), path.to_owned()])
    } else {
        Request::Shell(path.to_owned())
    }
}

fn worker() -> Option<Sender<Request>> {
    let sender = WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Request>();
        let spawned = std::thread::Builder::new()
            .name("lulo-launcher".into())
            .spawn(move || {
                // SAFETY: initialises COM for this thread, once.
                let _ = unsafe {
                    CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
                };
                while let Ok(request) = receiver.recv() {
                    run(&request);
                }
            });
        if let Err(error) = spawned {
            eprintln!("lulo-shell: cannot start the launcher thread: {error}");
        }
        Mutex::new(sender)
    });
    sender.lock().ok().map(|sender| sender.clone())
}

/// Open `request` on the launcher thread.
pub fn open(request: Request) {
    // The user's click or key just reached this process: let the app it
    // opens take the foreground.
    // SAFETY: no pointers.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    if let Some(worker) = worker() {
        let _ = worker.send(request);
    }
}

fn report(key: String) {
    if key.is_empty() {
        return;
    }
    if let Some(sender) = LAUNCHED.get() {
        let _ = sender.try_send(Launched { key });
    }
}

fn spawn(exe: &str, arguments: &[String]) {
    let path = install_dir().join(exe);
    match std::process::Command::new(&path).args(arguments).spawn() {
        Ok(child) => {
            // SAFETY: no pointers; the new process may take the foreground.
            let _ = unsafe { AllowSetForegroundWindow(child.id()) };
            trace(|| format!("launched {exe}"));
            report(exe.to_ascii_lowercase());
        }
        Err(error) => eprintln!("lulo-shell: could not open {}: {error}", path.display()),
    }
}

fn run(request: &Request) {
    match request {
        Request::Lulo(exe) => spawn(exe, &[]),
        Request::LuloWith(exe, arguments) => spawn(exe, arguments),
        Request::Shell(target) => {
            let file = HSTRING::from(target.as_str());
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
                lpVerb: w!("open"),
                lpFile: PCWSTR(file.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            // SAFETY: `info` and the strings it points at outlive the call.
            if unsafe { ShellExecuteExW(&mut info) }.is_err() {
                eprintln!("lulo-shell: Windows could not open {target}");
                return;
            }
            trace(|| format!("launched {target}"));
            if !info.hProcess.is_invalid() {
                // SAFETY: the process handle ShellExecuteExW returned, closed
                // here once its id is read.
                let pid = unsafe { GetProcessId(info.hProcess) };
                // SAFETY: as above.
                let _ = unsafe { CloseHandle(info.hProcess) };
                if pid != 0 {
                    // SAFETY: no pointers.
                    let _ = unsafe { AllowSetForegroundWindow(pid) };
                    report(crate::model::apps::exe_key(&super::process_path(pid)));
                }
            }
        }
    }
}
