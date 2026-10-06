//! One process per app on Windows (ADR 0023 phase 2).
//!
//! On Linux a second launch hands its windows to the running process over
//! D-Bus (`rmac_app_menu::open_window_in_running_instance`). Windows has no
//! session bus, so the running app serves a named pipe instead,
//! `\\.\pipe\lulo-<app id>-<user>`, and a later launch (a document opened
//! from Explorer, a second Start-menu click) writes its window requests to
//! it and exits. The pipe takes no remote clients, and its default security
//! lets only the same user (and administrators) write to it.
//!
//! The wire format is one window per line, its arguments separated by NUL.
//! Windows paths cannot contain either character, so nothing needs quoting.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::time::Duration;

use gpui::App;
use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

use crate::app_menu::OpenWindowRequest;

/// A request larger than this is cut off; eight windows of long paths fit.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
/// How long a launch waits for a busy pipe before starting on its own.
const BUSY_RETRIES: u32 = 20;

fn pipe_name(app_id: &str) -> String {
    let user = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect::<String>();
    format!(r"\\.\pipe\lulo-{app_id}-{user}")
}

/// The wire form of `windows`, or `None` when an argument cannot travel
/// (it holds a newline or NUL, which no Windows path does).
fn encode(windows: &[Vec<String>]) -> Option<Vec<u8>> {
    let mut message = String::new();
    for arguments in windows {
        if arguments
            .iter()
            .any(|argument| argument.contains(['\n', '\0']))
        {
            return None;
        }
        message.push_str(&arguments.join("\0"));
        message.push('\n');
    }
    Some(message.into_bytes())
}

/// The windows a request asks for; an unreadable request asks for none.
fn decode(bytes: &[u8]) -> Vec<Vec<String>> {
    let Ok(message) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    message
        .split_terminator('\n')
        .take(32)
        .map(|line| {
            if line.is_empty() {
                Vec::new()
            } else {
                line.split('\0').map(str::to_owned).collect()
            }
        })
        .collect()
}

/// Give `windows` to the running process. True when it took them.
pub(crate) fn hand_off(app_id: &str, windows: &[Vec<String>]) -> bool {
    let Some(message) = encode(windows) else {
        return false;
    };
    let name = pipe_name(app_id);
    for _ in 0..BUSY_RETRIES {
        match std::fs::OpenOptions::new().write(true).open(&name) {
            Ok(mut pipe) => {
                // This launch holds the user's activation; let the running
                // process bring its window forward with it.
                // SAFETY: a plain Win32 call with no pointers.
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                return pipe.write_all(&message).is_ok();
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            // Not running (or not answering): this launch starts the app.
            Err(_) => return false,
        }
    }
    false
}

/// One listening instance of the pipe, as a `File` that owns its handle.
fn create_instance(name: &[u16], first: bool) -> Option<File> {
    let mut mode = PIPE_ACCESS_INBOUND;
    if first {
        // Fail rather than share the name with a process that owns it.
        mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: `name` is NUL-terminated and outlives the call; no security
    // attributes means the default, same-user descriptor.
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            0,
            MAX_REQUEST_BYTES as u32,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        return None;
    }
    // SAFETY: the handle was just created and is owned by nothing else.
    Some(unsafe { File::from_raw_handle(handle.0) })
}

/// Serve later launches: each request's windows go to `open_window` on the
/// UI thread. A background thread blocks on the pipe, so nothing polls.
pub(crate) fn serve(app_id: &'static str, open_window: OpenWindowRequest, cx: &mut App) {
    let name = pipe_name(app_id)
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let Some(first) = create_instance(&name, true) else {
        eprintln!("{app_id}: another process already serves this app's launches");
        return;
    };
    let (sender, receiver) = async_channel::unbounded::<Vec<String>>();
    let spawned = std::thread::Builder::new()
        .name("lulo-instance".into())
        .spawn(move || {
            let mut pipe = first;
            loop {
                // SAFETY: `pipe` owns a live pipe handle.
                let connected = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) };
                if let Err(error) = connected {
                    if error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                        eprintln!("{app_id}: a launch could not reach this process: {error}");
                    }
                }
                // Listen again before reading, so a launch never finds the
                // name missing and starts a second process.
                let next = create_instance(&name, false);
                let mut bytes = Vec::new();
                let _ = (&mut pipe).take(MAX_REQUEST_BYTES).read_to_end(&mut bytes);
                drop(pipe);
                for arguments in decode(&bytes) {
                    if sender.send_blocking(arguments).is_err() {
                        return;
                    }
                }
                match next {
                    Some(next) => pipe = next,
                    None => return,
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("{app_id}: cannot serve later launches: {error}");
        return;
    }
    cx.spawn(async move |cx| {
        while let Ok(arguments) = receiver.recv().await {
            cx.update(|cx| open_window(arguments, cx));
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn requests_round_trip_windows_and_their_arguments() {
        let windows = vec![
            vec![r"C:\Users\Ada\Notes.txt".to_owned()],
            Vec::new(),
            vec!["--new-document".to_owned(), r"D:\a b\c.md".to_owned()],
        ];
        let bytes = encode(&windows).unwrap();
        assert_eq!(decode(&bytes), windows);
        assert!(encode(&[vec!["bad\nname".to_owned()]]).is_none());
        assert!(decode(&[0xff, b'\n']).is_empty());
    }
}
