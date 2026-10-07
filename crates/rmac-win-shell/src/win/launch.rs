//! Opening apps and files. `ShellExecute` can take a moment (it may load
//! shell extensions or start a Store app), so it runs on its own thread
//! with COM initialised, never on the UI thread.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Com::{
    CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL};

use super::trace;

#[derive(Clone, Debug)]
pub enum Request {
    /// A Lulo app's executable name, beside `lulo-shell.exe`.
    Lulo(String),
    /// Anything `ShellExecute` opens.
    Shell(String),
}

static WORKER: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();

/// The folder `lulo-shell.exe` runs from, where the Lulo apps are.
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_default()
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

fn run(request: &Request) {
    match request {
        Request::Lulo(exe) => {
            let path = install_dir().join(exe);
            match std::process::Command::new(&path).spawn() {
                Ok(_) => trace(|| format!("launched {exe}")),
                Err(error) => eprintln!("lulo-shell: could not open {}: {error}", path.display()),
            }
        }
        Request::Shell(target) => {
            // SAFETY: NUL-terminated strings that outlive the call.
            let result = unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    &HSTRING::from(target.as_str()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                )
            };
            // Values above 32 mean success.
            if result.0 as usize > 32 {
                trace(|| format!("launched {target}"));
            } else {
                eprintln!("lulo-shell: Windows could not open {target}");
            }
        }
    }
}
