//! The menu bar's end of the Lulo app menu pipe (`rmac_app_menu::pipe`).
//!
//! The bar serves `\\.\pipe\lulo-menubar-<user>` and sets the ready event
//! once the pipe exists, so Lulo apps that are already running connect at
//! once. Each connection gets a thread that blocks reading it; which
//! process is on the other end comes from the pipe itself. The first pipe
//! instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, which also
//! keeps a second Lulo layer from starting beside the first.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use rmac_app_menu::pipe::{self, Message};
use rmac_app_menu::Menu;
use windows::core::{HRESULT, HSTRING};
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent};

use super::{trace, user_name};

pub enum MenuEvent {
    /// A Lulo app's menus, as it wants them shown (the bold app menu
    /// first).
    Menus {
        pid: u32,
        app_id: String,
        menus: Vec<Menu>,
    },
    /// Where to send that app's commands.
    Commands { pid: u32, writer: File },
    /// The app quit or dropped its link.
    Gone { pid: u32 },
}

pub struct Server {
    ready: HANDLE,
}

impl Server {
    /// Tell apps the bar is gone, so none waits on a pipe nobody serves.
    pub fn stop(&self) {
        // SAFETY: the event handle this server created.
        let _ = unsafe { ResetEvent(self.ready) };
    }
}

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
            64 * 1024,
            64 * 1024,
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

/// Serve the menu pipe; `None` when another Lulo layer already does.
pub fn start(events: async_channel::Sender<MenuEvent>) -> Option<Server> {
    let user = user_name();
    let name = HSTRING::from(pipe::pipe_name(&user));
    let first = create_instance(&name, true)?;
    // SAFETY: a named manual-reset event, shared with the apps.
    let ready = unsafe {
        CreateEventW(
            None,
            true,
            false,
            &HSTRING::from(pipe::ready_event_name(&user)),
        )
    }
    .ok()?;
    let spawned = std::thread::Builder::new()
        .name("lulo-menubar-server".into())
        .spawn(move || {
            let mut pipe = first;
            loop {
                // SAFETY: `pipe` owns a listening instance.
                if let Err(error) = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) }
                {
                    if error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                        // The client left before it was served: start over
                        // with a fresh instance.
                        match create_instance(&name, false) {
                            Some(fresh) => pipe = fresh,
                            None => return,
                        }
                        continue;
                    }
                }
                // Listen again before serving, so the next app finds the
                // name.
                let next = create_instance(&name, false);
                let events = events.clone();
                let _ = std::thread::Builder::new()
                    .name("lulo-menubar-app".into())
                    .spawn(move || serve(pipe, events));
                match next {
                    Some(next) => pipe = next,
                    None => return,
                }
            }
        });
    if spawned.is_err() {
        return None;
    }
    // SAFETY: as above; the pipe exists now.
    let _ = unsafe { SetEvent(ready) };
    Some(Server { ready })
}

fn serve(pipe: File, events: async_channel::Sender<MenuEvent>) {
    let mut pid = 0u32;
    // SAFETY: a connected pipe handle and an out-parameter.
    if unsafe { GetNamedPipeClientProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.is_err() {
        return;
    }
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    match pipe::decode_message(line.trim_end_matches('\n')) {
        Ok(Message::Commands { app_id }) => {
            trace(|| format!("menu link from {app_id} ({pid})"));
            let _ = events.send_blocking(MenuEvent::Commands {
                pid,
                writer: reader.into_inner(),
            });
        }
        Ok(Message::Hello { app_id }) => loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = events.send_blocking(MenuEvent::Gone { pid });
                    return;
                }
                Ok(_) if line.len() > pipe::MAX_LINE_BYTES => {
                    let _ = events.send_blocking(MenuEvent::Gone { pid });
                    return;
                }
                Ok(_) => {
                    if let Ok(Message::Menus(menus)) =
                        pipe::decode_message(line.trim_end_matches('\n'))
                    {
                        let sent = events.send_blocking(MenuEvent::Menus {
                            pid,
                            app_id: app_id.clone(),
                            menus,
                        });
                        if sent.is_err() {
                            return;
                        }
                    }
                }
            }
        },
        _ => {}
    }
}
