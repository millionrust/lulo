//! The menu bar's side of the Lulo menu pipe on Windows (ADR 0023 phase 3;
//! "Phase 3 revised: shared shell views").
//!
//! On Lulo OS the menu bar reads each app's menus over D-Bus with
//! [`fetch_layout`], follows them with [`watch_layout_changes`] and
//! [`watch_menu_owners`], and runs a command with [`activate`]. On Windows
//! the same functions are answered here, from the menus Lulo apps send over
//! `\\.\pipe\lulo-menubar-<user>` ([`crate::pipe`]), so the one menu bar
//! view runs unchanged on both.
//!
//! [`start`] serves the pipe and sets the ready event once it exists, so
//! Lulo apps that are already running connect at once. Each connection gets
//! a thread that blocks reading it; which process is on the other end comes
//! from the pipe itself. The first pipe instance is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE`, which also keeps a second Lulo layer
//! from starting beside the first. Nothing polls.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write as _};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::{Arc, Mutex};

use windows::core::{HRESULT, HSTRING};
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent};

use crate::pipe::{self, Command, Message};
use crate::{Error, Layout, Menu};

/// One running Lulo app's link.
struct Linked {
    app_id: &'static str,
    menus: Option<Vec<Menu>>,
    revision: u32,
    commands: Option<Commands>,
}

#[derive(Default)]
struct Host {
    /// By process id.
    apps: BTreeMap<u32, Linked>,
    layout_watchers: Vec<async_channel::Sender<()>>,
    owner_watchers: Vec<async_channel::Sender<(&'static str, bool)>>,
}

static HOST: Mutex<Option<Host>> = Mutex::new(None);

fn with_host<T>(f: impl FnOnce(&mut Host) -> T) -> T {
    let mut guard = HOST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(guard.get_or_insert_with(Host::default))
}

/// The app id a Lulo app gave over the pipe, as the static identity the
/// D-Bus side names apps by; an app this shell does not know is ignored.
fn known_app(app_id: &str) -> Option<&'static str> {
    crate::bus_name(app_id).and_then(crate::app_for_bus_name)
}

/// The pipe while this process serves it.
pub struct PipeHost {
    ready: HANDLE,
}

// The event handle is a kernel object usable from any thread.
unsafe impl Send for PipeHost {}
unsafe impl Sync for PipeHost {}

impl PipeHost {
    /// Tell apps the bar is gone, so none waits on a pipe nobody serves.
    pub fn stop(&self) {
        // SAFETY: the event handle this host created.
        let _ = unsafe { ResetEvent(self.ready) };
    }
}

fn user_name() -> String {
    std::env::var("USERNAME").unwrap_or_default()
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
pub fn start() -> Option<PipeHost> {
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
                let _ = std::thread::Builder::new()
                    .name("lulo-menubar-app".into())
                    .spawn(move || serve(pipe));
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
    Some(PipeHost { ready })
}

fn serve(pipe: File) {
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
            let Some(app_id) = known_app(&app_id) else {
                return;
            };
            let writer = Arc::new(Mutex::new(reader.into_inner()));
            with_host(|host| {
                host.apps
                    .entry(pid)
                    .or_insert_with(|| Linked {
                        app_id,
                        menus: None,
                        revision: 0,
                        commands: None,
                    })
                    .commands = Some(writer);
            });
        }
        Ok(Message::Hello { app_id }) => {
            let Some(app_id) = known_app(&app_id) else {
                return;
            };
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > pipe::MAX_LINE_BYTES => break,
                    Ok(_) => {
                        if let Ok(Message::Menus(menus)) =
                            pipe::decode_message(line.trim_end_matches('\n'))
                        {
                            published(pid, app_id, menus);
                        }
                    }
                }
            }
            gone(pid);
        }
        _ => {}
    }
}

/// `pid` sent its menus: the first time it is published, as its bus name
/// appearing is on Lulo OS; after that only a change is announced.
fn published(pid: u32, app_id: &'static str, menus: Vec<Menu>) {
    let (first, changed, layout, owners) = with_host(|host| {
        let linked = host.apps.entry(pid).or_insert_with(|| Linked {
            app_id,
            menus: None,
            revision: 0,
            commands: None,
        });
        let first = linked.menus.is_none();
        let changed = linked.menus.as_ref() != Some(&menus);
        if changed {
            linked.revision = linked.revision.wrapping_add(1);
            linked.menus = Some(menus);
        }
        host.layout_watchers.retain(|watcher| !watcher.is_closed());
        host.owner_watchers.retain(|watcher| !watcher.is_closed());
        (
            first,
            changed,
            host.layout_watchers.clone(),
            host.owner_watchers.clone(),
        )
    });
    if first {
        rmac_apps::windows_apps::register_process_app(pid, app_id);
        // For the Windows CI checks (`scripts/windows/shell_smoke.py`).
        if std::env::var_os("LULO_SHELL_TRACE").is_some_and(|value| value == "1") {
            eprintln!("lulo-shell: menus from {app_id}");
        }
        for watcher in owners {
            let _ = watcher.try_send((app_id, true));
        }
    } else if changed {
        for watcher in layout {
            let _ = watcher.try_send(());
        }
    }
}

fn gone(pid: u32) {
    let (removed, owners) = with_host(|host| {
        let removed = host.apps.remove(&pid).map(|linked| linked.app_id);
        host.owner_watchers.retain(|watcher| !watcher.is_closed());
        (removed, host.owner_watchers.clone())
    });
    rmac_apps::windows_apps::forget_process_app(pid);
    if let Some(app_id) = removed {
        for watcher in owners {
            let _ = watcher.try_send((app_id, false));
        }
    }
}

/// Where a linked app's commands go.
type Commands = Arc<Mutex<File>>;

/// The most recently linked process of `app_id` that has sent its menus.
fn linked(app_id: &str) -> Option<(Vec<Menu>, u32, Option<Commands>)> {
    with_host(|host| {
        host.apps
            .iter()
            .rev()
            .filter(|(_, linked)| linked.app_id == app_id)
            .find_map(|(_, linked)| {
                linked
                    .menus
                    .clone()
                    .map(|menus| (menus, linked.revision, linked.commands.clone()))
            })
    })
}

fn send(writer: &Commands, command: &Command) -> Result<(), Error> {
    let line = pipe::encode_command(command)?;
    let mut writer = writer.lock().map_err(|_| Error::Protocol)?;
    writer
        .write_all(line.as_bytes())
        .map_err(|error| Error::Bus(format!("the app's menu link closed: {error}")))
}

pub async fn fetch(app_id: &str) -> Result<Vec<Menu>, Error> {
    fetch_layout(app_id).await.map(|layout| layout.menus)
}

/// The app's menus as it last sent them. As `Layout` does on Lulo OS, the
/// app is asked to validate them again; a reply that changes them arrives
/// as a layout change.
pub async fn fetch_layout(app_id: &str) -> Result<Layout, Error> {
    known_app(app_id).ok_or(Error::Unsupported)?;
    let (menus, revision, commands) = linked(app_id).ok_or(Error::NotPublished)?;
    if let Some(commands) = commands {
        let _ = send(&commands, &Command::Validate);
    }
    Ok(Layout {
        revision: Some(revision),
        menus,
    })
}

/// Changes to any linked app's menus.
pub struct LayoutChanges {
    receiver: async_channel::Receiver<()>,
}

pub async fn watch_layout_changes() -> Result<LayoutChanges, Error> {
    let (sender, receiver) = async_channel::bounded(1);
    with_host(|host| host.layout_watchers.push(sender));
    Ok(LayoutChanges { receiver })
}

impl LayoutChanges {
    /// Waits for the next change; `false` once the host is gone.
    pub async fn next(&mut self) -> bool {
        self.receiver.recv().await.is_ok()
    }
}

pub async fn activate(app_id: &str, action: &str) -> Result<(), Error> {
    known_app(app_id).ok_or(Error::Unsupported)?;
    if !crate::valid_action(action) {
        return Err(Error::Protocol);
    }
    let (_, _, commands) = linked(app_id).ok_or(Error::NotPublished)?;
    let commands = commands.ok_or(Error::NotPublished)?;
    send(&commands, &Command::Activate(action.to_owned()))
}

/// Lulo apps linking to the bar (`true`, once their menus arrive) and
/// leaving it (`false`).
pub struct MenuOwners {
    receiver: async_channel::Receiver<(&'static str, bool)>,
}

pub async fn watch_menu_owners() -> Result<MenuOwners, Error> {
    let (sender, receiver) = async_channel::bounded(64);
    with_host(|host| host.owner_watchers.push(sender));
    Ok(MenuOwners { receiver })
}

impl MenuOwners {
    pub async fn next(&mut self) -> Option<(&'static str, bool)> {
        self.receiver.recv().await.ok()
    }
}

/// The Lulo app each linked process says it is, for the compositor
/// backend to name windows of renamed executables.
pub fn linked_apps() -> Vec<(u32, &'static str)> {
    with_host(|host| {
        host.apps
            .iter()
            .map(|(pid, linked)| (*pid, linked.app_id))
            .collect()
    })
}
