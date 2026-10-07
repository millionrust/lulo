//! A Lulo app's link to the Lulo menu bar on Windows (ADR 0023 phase 3).
//!
//! On Lulo OS the menu bar reads an app's menus over D-Bus (`app_menu`).
//! On Windows the Lulo layer's menu bar (`lulo-shell`) serves a named pipe
//! instead (`rmac_app_menu::pipe`). When it runs, the app sends it the same
//! menus its in-window strip would show, runs the commands the bar sends
//! back, and hides the strip: there is one menu bar, as on the Mac. When
//! the bar quits, the strip comes back.
//!
//! Nothing polls. The app connects at start when the bar is already there;
//! otherwise a thread blocks on the bar's ready event
//! (`rmac_app_menu::pipe::ready_event_name`) and connects when the bar
//! starts. While connected, the same thread blocks reading the bar's
//! commands; end of file means the bar has gone.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, Global};
use rmac_app_menu::pipe::{self, Command, Message};
use windows::core::HSTRING;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject, INFINITE};

/// How many times a launch retries a pipe every instance of which is busy
/// (several apps connecting at once) before it waits for the next start.
const BUSY_RETRIES: u32 = 40;

enum LinkEvent {
    Connected(File),
    Command(Command),
    Disconnected,
}

#[derive(Default)]
struct MenuBarLink {
    writer: Option<Arc<Mutex<File>>>,
}

impl Global for MenuBarLink {}

/// Whether the Lulo menu bar shows this app's menus now.
pub(crate) fn connected(cx: &App) -> bool {
    cx.try_global::<MenuBarLink>()
        .is_some_and(|link| link.writer.is_some())
}

fn user() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

/// Why a connection attempt failed: the bar is not there at all, or it is
/// there but did not take the connection.
enum ConnectError {
    Missing,
    Other,
}

fn connect(app_id: &str) -> Result<(File, File), ConnectError> {
    let name = pipe::pipe_name(&user());
    let open = |read: bool| {
        for _ in 0..BUSY_RETRIES {
            match std::fs::OpenOptions::new()
                .read(read)
                .write(true)
                .open(&name)
            {
                Ok(file) => return Ok(file),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND.0 as i32) => {
                    return Err(ConnectError::Missing);
                }
                Err(_) => return Err(ConnectError::Other),
            }
        }
        Err(ConnectError::Other)
    };
    let line = |message: Message| pipe::encode_message(&message).map_err(|_| ConnectError::Other);
    let mut menus = open(false)?;
    menus
        .write_all(
            line(Message::Hello {
                app_id: app_id.to_owned(),
            })?
            .as_bytes(),
        )
        .map_err(|_| ConnectError::Other)?;
    let mut commands = open(true)?;
    commands
        .write_all(
            line(Message::Commands {
                app_id: app_id.to_owned(),
            })?
            .as_bytes(),
        )
        .map_err(|_| ConnectError::Other)?;
    Ok((menus, commands))
}

/// Link `app_id` to the menu bar when it runs, now or later.
pub(crate) fn install(app_id: &'static str, cx: &mut App) {
    cx.set_global(MenuBarLink::default());
    let (sender, receiver) = async_channel::unbounded::<LinkEvent>();
    // Connect before the first window opens, so a window opened while the
    // bar runs never makes room for a strip it will not show.
    let first = connect(app_id).ok().and_then(|(menus, commands)| {
        let recorded = menus.try_clone().ok()?;
        set_writer(Some(recorded), app_id, cx);
        Some((menus, commands))
    });
    let spawned = std::thread::Builder::new()
        .name("lulo-menubar-link".into())
        .spawn(move || link_thread(app_id, first, sender));
    if let Err(error) = spawned {
        eprintln!("{app_id}: cannot link to the Lulo menu bar: {error}");
        return;
    }
    cx.spawn(async move |cx| {
        while let Ok(event) = receiver.recv().await {
            cx.update(|cx| match event {
                LinkEvent::Connected(menus) => set_writer(Some(menus), app_id, cx),
                LinkEvent::Disconnected => set_writer(None, app_id, cx),
                LinkEvent::Command(Command::Validate) => send_menus(app_id, cx),
                LinkEvent::Command(Command::Activate(name)) => match cx.build_action(&name, None) {
                    Ok(action) => crate::menu_target::dispatch_menu_action(action, cx),
                    Err(error) => eprintln!("ignored unavailable {app_id} menu action: {error}"),
                },
            });
        }
    })
    .detach();
}

fn set_writer(writer: Option<File>, app_id: &'static str, cx: &mut App) {
    let connected = writer.is_some();
    cx.global_mut::<MenuBarLink>().writer = writer.map(|writer| Arc::new(Mutex::new(writer)));
    crate::menu_strip::refresh_all(cx);
    if connected {
        send_menus(app_id, cx);
    }
}

/// Give the bar this app's menus, validated now, in the strip's shape.
fn send_menus(app_id: &'static str, cx: &mut App) {
    let Some(writer) = cx
        .try_global::<MenuBarLink>()
        .and_then(|link| link.writer.clone())
    else {
        return;
    };
    let app_name = rmac_apps::identity::window_title(app_id).unwrap_or(app_id);
    let menus = crate::menu_strip::strip_menus(app_name, crate::app_menu::current_menus(cx));
    let line = match pipe::encode_message(&Message::Menus(menus)) {
        Ok(line) => line,
        Err(error) => {
            eprintln!("{app_id}: the menu bar cannot show these menus: {error}");
            return;
        }
    };
    cx.background_executor()
        .spawn(async move {
            if let Ok(mut writer) = writer.lock() {
                // A bar that quit is reported by the reading thread.
                let _ = writer.write_all(line.as_bytes());
            }
        })
        .detach();
}

fn link_thread(
    app_id: &'static str,
    mut pending: Option<(File, File)>,
    events: async_channel::Sender<LinkEvent>,
) {
    let event_name = HSTRING::from(pipe::ready_event_name(&user()));
    // SAFETY: a named manual-reset event; the name outlives the call.
    let ready = match unsafe { CreateEventW(None, true, false, &event_name) } {
        Ok(event) => event,
        Err(error) => {
            eprintln!("{app_id}: cannot wait for the Lulo menu bar: {error}");
            return;
        }
    };
    // A connection made before GPUI's windows opened is already recorded;
    // every later one is announced.
    let mut recorded = pending.is_some();
    loop {
        let (menus, commands) = match pending.take() {
            Some(connection) => connection,
            None => {
                // SAFETY: `ready` is a live event handle owned by this thread.
                unsafe { WaitForSingleObject(ready, INFINITE) };
                match connect(app_id) {
                    Ok(connection) => connection,
                    Err(ConnectError::Missing) => {
                        // The event outlived a bar that did not reset it (it
                        // crashed): reset it, unless a new bar has just set
                        // it again with its pipe in place.
                        // SAFETY: as above.
                        let _ = unsafe { ResetEvent(ready) };
                        match connect(app_id) {
                            Ok(connection) => connection,
                            Err(_) => continue,
                        }
                    }
                    Err(ConnectError::Other) => {
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
            }
        };
        let announced = if std::mem::take(&mut recorded) {
            Ok(())
        } else {
            events.send_blocking(LinkEvent::Connected(menus))
        };
        if announced.is_err() {
            return;
        }
        let mut reader = BufReader::new(commands);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > pipe::MAX_LINE_BYTES {
                        break;
                    }
                    if let Ok(command) = pipe::decode_command(line.trim_end_matches('\n')) {
                        if events.send_blocking(LinkEvent::Command(command)).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        if events.send_blocking(LinkEvent::Disconnected).is_err() {
            return;
        }
    }
}
