//! The notification pipe on Windows (`rmac_notifications::pipe`; ADR 0023,
//! "Notification Centre and the app switcher on Windows").
//!
//! Lulo apps post their notifications on `\\.\pipe\lulo-notifications-<user>`
//! instead of the session bus Lulo OS has. One thread waits for
//! connections; each connection gets a thread that reads its one post,
//! gives it to the bus-less service, answers its id and then writes its
//! action and closing as they happen. Who posted comes from the pipe's
//! client process (its executable), never from what it wrote. Nothing
//! polls: every thread blocks in a pipe call or on its notification's
//! answers.

use std::fs::File;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use rmac_notifications::pipe::{self, Answer};
use rmac_notifications_linux::service::local::Poster;
use rmac_notifications_linux::service::ServiceHandle;
use windows::core::{HRESULT, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

fn create_instance(name: &HSTRING, first: bool) -> Option<File> {
    let mut mode = PIPE_ACCESS_DUPLEX;
    if first {
        mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: a NUL-terminated name; default (same-user) security.
    let handle = unsafe {
        CreateNamedPipeW(
            name,
            mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            16 * 1024,
            pipe::MAX_LINE_BYTES as u32,
            0,
            None,
        )
    };
    if handle.is_invalid() {
        return None;
    }
    // SAFETY: a new handle nothing else owns.
    Some(unsafe { File::from_raw_handle(handle.0) })
}

/// Serve the pipe for `service` (once per process).
pub(crate) fn start(service: ServiceHandle) {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let name = HSTRING::from(pipe::pipe_name(&user));
    let Some(first) = create_instance(&name, true) else {
        eprintln!("notifications: another Notification Centre serves the pipe");
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("lulo-notifications-server".into())
        .spawn(move || {
            let mut instance = first;
            loop {
                // SAFETY: `instance` owns a listening pipe instance.
                if let Err(error) =
                    unsafe { ConnectNamedPipe(HANDLE(instance.as_raw_handle()), None) }
                {
                    if error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                        // The client left before it was served.
                        match create_instance(&name, false) {
                            Some(fresh) => instance = fresh,
                            None => return,
                        }
                        continue;
                    }
                }
                // Listen again before serving, so the next app finds the name.
                let next = create_instance(&name, false);
                let service = service.clone();
                let _ = std::thread::Builder::new()
                    .name("lulo-notification".into())
                    .spawn(move || serve(instance, service));
                match next {
                    Some(next) => instance = next,
                    None => return,
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("notifications: could not serve the pipe: {error}");
    }
}

/// The full path of process `pid`'s executable.
fn process_path(pid: u32) -> Option<String> {
    // SAFETY: opens the process for a limited query only, closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: `buffer` holds `size` UTF-16 units.
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    };
    // SAFETY: the handle opened above.
    let _ = unsafe { CloseHandle(process) };
    read.ok()?;
    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

/// Who process `pid` is: a Lulo app by its executable, else the Windows
/// app the window list names it as.
fn poster(pid: u32) -> Poster {
    let executable = process_path(pid);
    let path = executable.clone().unwrap_or_default();
    let file = path.rsplit(['\\', '/']).next().unwrap_or_default();
    match rmac_apps::windows_apps::app_for_exe(file) {
        Some(app) => Poster {
            app_id: app.app_id.to_owned(),
            desktop_id: Some(app.app_id.to_owned()),
            executable,
            app_name: Some(app.name.to_owned()),
        },
        None => Poster {
            app_id: rmac_apps::windows_apps::window_app_id(&path, None),
            desktop_id: None,
            app_name: Some(rmac_apps::windows_apps::name_from_exe(&path)),
            executable,
        },
    }
}

fn serve(connection: File, service: ServiceHandle) {
    let mut pid = 0u32;
    // SAFETY: a connected pipe handle and an out-parameter.
    if unsafe { GetNamedPipeClientProcessId(HANDLE(connection.as_raw_handle()), &mut pid) }.is_err()
    {
        return;
    }
    let mut reader = BufReader::new(connection).take(pipe::MAX_LINE_BYTES as u64 + 1);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }
    let Ok(post) = pipe::decode_post(line.trim_end_matches(['\r', '\n'])) else {
        return;
    };
    let (answers_tx, answers) = std::sync::mpsc::channel();
    let posted = futures_lite::future::block_on(service.post_local(poster(pid), post, answers_tx));
    let mut connection = reader.into_inner().into_inner();
    let Ok(id) = posted else {
        return;
    };
    rmac_shell_layer::windows::trace::trace(|| format!("notification {} posted", id.get()));
    if connection
        .write_all(pipe::encode_answer(&Answer::Posted(id.get())).as_bytes())
        .is_err()
    {
        return;
    }
    // From here on this side only writes and the app only reads.
    for answer in answers {
        let closed = matches!(answer, Answer::Closed(_));
        if connection
            .write_all(pipe::encode_answer(&answer).as_bytes())
            .is_err()
            || closed
        {
            return;
        }
    }
}
