//! The Lulo layer's surfaces: the menu bar, the Dock, Spotlight and the
//! menu panels, all in one GPUI process (one Direct3D device and font
//! collection for the lot, which keeps start-up and memory small).

mod assets;
mod bar;
mod dock;
mod menu;
mod spotlight;

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, RenderImage,
    SharedString, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use rmac_app_menu::pipe::{self, Command};
use rmac_app_menu::Menu;
use windows::Win32::Foundation::{LRESULT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_QUERYENDSESSION,
};

use crate::model::apps;
use crate::model::clock::{self, ClockStyle};
use crate::model::dock::{self as dock_model, Pinned, Tile, TileIcon};
use crate::model::menus;
use crate::model::search::Entry;
use crate::win::appbar::{self, Edge};
use crate::win::events::{self, DesktopEvent, Hooks};
use crate::win::menubar_server::{self, MenuEvent, Server};
use crate::win::status::{Battery, StatusEvent, Volume, Wifi};
use crate::win::windows_list::{self, AppWindow};
use crate::win::{catalog, icons, launch, power, registry, surface, taskbar, trace};

/// The menu bar's height, as the Mac's.
pub(crate) const BAR_HEIGHT: f32 = 24.0;

/// The app in front, whose name and menus the bar shows.
#[derive(Clone, Debug)]
pub(crate) struct Front {
    pub hwnd: isize,
    pub pid: u32,
    pub key: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListState {
    Missing,
    Loading,
    Ready,
    Stale,
}

/// What every surface draws from. Changed only by events.
pub(crate) struct ShellState {
    pub pins: Vec<Pinned>,
    pub windows: Vec<AppWindow>,
    pub tiles: Vec<Tile>,
    /// `None`: the desktop itself is in front.
    pub front: Option<Front>,
    /// Lulo apps' own menus, by process.
    pub lulo_menus: HashMap<u32, (String, Vec<Menu>)>,
    commands: HashMap<u32, Arc<Mutex<File>>>,
    pub battery: Option<Battery>,
    pub volume: Option<Volume>,
    pub wifi: Option<Wifi>,
    pub clock: SharedString,
    clock_style: ClockStyle,
    pub apps: Vec<Entry>,
    pub files: Vec<Entry>,
    pub apps_state: ListState,
    pub files_state: ListState,
    icons: HashMap<String, Option<Arc<RenderImage>>>,
    pub starts_at_sign_in: bool,
    /// The bar title whose menu is open (0 is the Lulo menu).
    pub open_menu: Option<usize>,
    pub spotlight_open: bool,
}

impl ShellState {
    fn new() -> Self {
        Self {
            pins: dock_model::default_pins(),
            windows: Vec::new(),
            tiles: Vec::new(),
            front: None,
            lulo_menus: HashMap::new(),
            commands: HashMap::new(),
            battery: None,
            volume: None,
            wifi: None,
            clock: SharedString::default(),
            clock_style: clock_style(),
            apps: Vec::new(),
            files: Vec::new(),
            apps_state: ListState::Missing,
            files_state: ListState::Missing,
            icons: HashMap::new(),
            starts_at_sign_in: registry::starts_at_sign_in(),
            open_menu: None,
            spotlight_open: false,
        }
    }

    /// Read the window list again and rebuild the Dock's tiles.
    fn refresh_windows(&mut self) {
        events::windows_read();
        self.windows = windows_list::app_windows();
        let running = windows_list::running(&self.windows);
        self.tiles = dock_model::tiles(&self.pins, &running);
        let wanted = self
            .tiles
            .iter()
            .filter_map(|tile| match &tile.icon {
                TileIcon::Shell(source) => Some(source.clone()),
                TileIcon::Asset(_) => None,
            })
            .collect::<Vec<_>>();
        for source in wanted {
            self.want_icon(&source);
        }
        // An app that left takes its front place with it.
        if let Some(front) = &self.front {
            if !self.windows.iter().any(|window| window.pid == front.pid) {
                self.front = None;
            }
        }
    }

    /// The window `hwnd` came to the front.
    fn set_foreground(&mut self, hwnd: isize) {
        if windows_list::is_desktop(hwnd) {
            self.front = None;
            return;
        }
        // Start, the task switcher and other system pop-ups take the
        // foreground for a moment; the bar keeps the app it showed.
        if !windows_list::is_app(hwnd) {
            return;
        }
        let pid = windows_list::process_id(windows_list::handle(hwnd));
        let exe_path = windows_list::process_path(pid);
        let key = apps::exe_key(&exe_path);
        let name = if key == "applicationframehost.exe" {
            windows_list::title(windows_list::handle(hwnd))
        } else {
            windows_list::app_name(&exe_path)
        };
        self.front = Some(Front {
            hwnd,
            pid,
            key,
            name,
        });
    }

    /// What the bar shows of the app in front.
    fn front_key(&self) -> Option<(isize, u32, String)> {
        self.front
            .as_ref()
            .map(|front| (front.hwnd, front.pid, front.name.clone()))
    }

    /// The menus the bar shows after the Lulo menu: the front Lulo app's
    /// own, or the app and Window menus for any other app.
    pub fn front_menus(&self) -> Vec<Menu> {
        match &self.front {
            Some(front) => match self.lulo_menus.get(&front.pid) {
                Some((_, menus)) => menus.clone(),
                None => menus::windows_app_menus(&front.name),
            },
            None => menus::desktop_menus(),
        }
    }

    /// Whether the app in front is a Lulo app whose menus the bar shows.
    pub fn front_is_lulo(&self) -> bool {
        self.front
            .as_ref()
            .is_some_and(|front| self.lulo_menus.contains_key(&front.pid))
    }

    /// Send `command` to the Lulo app in front.
    fn send_to_front(&self, command: Command) {
        let Some(front) = &self.front else {
            return;
        };
        let Some(writer) = self.commands.get(&front.pid).cloned() else {
            return;
        };
        let Ok(line) = pipe::encode_command(&command) else {
            return;
        };
        // A tiny write to a pipe the app reads all the time; off the UI
        // thread all the same.
        let _ = std::thread::Builder::new()
            .name("lulo-menubar-send".into())
            .spawn(move || {
                if let Ok(mut writer) = writer.lock() {
                    let _ = writer.write_all(line.as_bytes());
                }
            });
    }

    /// Ask, once, for the icon Windows shows for `source`.
    pub fn want_icon(&mut self, source: &str) {
        if !self.icons.contains_key(source) {
            self.icons.insert(source.to_owned(), None);
            icons::request(source);
        }
    }

    /// The icon for `source`, once it has been read.
    pub fn cached_icon(&self, source: &str) -> Option<Arc<RenderImage>> {
        self.icons.get(source).cloned().flatten()
    }

    fn tick_clock(&mut self) {
        let now = chrono::Local::now().naive_local();
        self.clock = clock::clock_text(now, self.clock_style).into();
    }
}

fn clock_style() -> ClockStyle {
    use windows::Win32::Globalization::{
        GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_STIMEFORMAT,
    };
    let mut locale = [0u16; 85];
    // SAFETY: writable buffers with their lengths.
    let length = unsafe { GetUserDefaultLocaleName(&mut locale) };
    if length <= 0 {
        return ClockStyle::default();
    }
    let name = crate::win::from_wide(&locale);
    let mut format = [0u16; 80];
    // SAFETY: as above; the locale name is NUL-terminated.
    unsafe {
        GetLocaleInfoEx(
            windows::core::PCWSTR(locale.as_ptr()),
            LOCALE_STIMEFORMAT,
            Some(&mut format),
        )
    };
    ClockStyle::from_locale(&name, &crate::win::from_wide(&format))
}

/// A surface window and its Win32 handle.
#[derive(Clone, Copy)]
pub(crate) struct Surface {
    pub handle: AnyWindowHandle,
    pub hwnd: isize,
}

/// What wndproc hooks hand to the UI.
#[derive(Clone, Copy, Debug)]
enum Signal {
    /// The work area, the display or its scale changed: place the bars
    /// again.
    Reposition,
    /// Explorer restarted: hide its new taskbar and register again.
    ExplorerRestarted,
    /// `lulo-session --stop` asked the Lulo layer to turn off.
    Stop,
}

pub(crate) struct Runtime {
    pub shell: Entity<ShellState>,
    pub bar: Option<Surface>,
    pub dock: Option<Surface>,
    pub spotlight: Option<Surface>,
    pub overlay: Option<Surface>,
    server: Server,
    hooks: Option<Hooks>,
    took_taskbar: bool,
    /// The bar's and the Dock's strips in physical pixels.
    pub bar_rect: RECT,
    pub dock_strip: RECT,
    pub scale: f32,
}

impl Global for Runtime {}

pub(crate) fn runtime(cx: &App) -> &Runtime {
    cx.global::<Runtime>()
}

pub(crate) fn shell(cx: &App) -> Entity<ShellState> {
    runtime(cx).shell.clone()
}

static CLEANED_UP: AtomicBool = AtomicBool::new(false);

/// Give the desktop back: the AppBars' strips, the taskbar and the ready
/// event. Safe to call more than once and from a window procedure.
fn restore_desktop(bar: Option<isize>, dock: Option<isize>, took_taskbar: bool) {
    if CLEANED_UP.swap(true, Ordering::AcqRel) {
        return;
    }
    for hwnd in [bar, dock].into_iter().flatten() {
        appbar::remove(windows_list::handle(hwnd));
    }
    if took_taskbar {
        taskbar::restore();
    }
    registry::delete_value("BarWindow");
    registry::delete_value("DockWindow");
}

/// Run Win32 calls that send messages to this process's own windows (a
/// window moving, showing, hiding or losing the foreground) once GPUI is no
/// longer in the middle of an update: GPUI's window procedure calls back
/// into the app for those messages, as its own Windows backend avoids by
/// deferring the same calls.
pub(crate) fn later(cx: &mut App, f: impl FnOnce() + 'static) {
    cx.spawn(async move |_| f()).detach();
}

/// Turn the Lulo layer off.
pub(crate) fn quit(cx: &mut App) {
    let runtime = runtime(cx);
    trace(|| "quitting".into());
    runtime.server.stop();
    if let Some(hooks) = &runtime.hooks {
        hooks.stop();
    }
    let (bar, dock, took_taskbar) = (
        runtime.bar.map(|bar| bar.hwnd),
        runtime.dock.map(|dock| dock.hwnd),
        runtime.took_taskbar,
    );
    cx.spawn(async move |cx| {
        restore_desktop(bar, dock, took_taskbar);
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("lulo-shell".into()),
        ..Default::default()
    }
}

/// Open a hidden surface window whose root is `build`'s view.
pub(crate) fn open_surface<V: gpui::Render + 'static>(
    bounds: Bounds<gpui::Pixels>,
    activatable: bool,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut gpui::Context<V>) -> V + 'static,
) -> Option<Surface> {
    let handle = cx
        .open_window(options(bounds), move |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|cx| build(window, cx));
            cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
        })
        .ok()?;
    let any: AnyWindowHandle = handle.into();
    let hwnd = any
        .update(cx, |_, window, _| surface::hwnd(window))
        .ok()
        .flatten()?;
    let raw = hwnd.0 as isize;
    later(cx, move || {
        let hwnd = windows_list::handle(raw);
        surface::make_shell_surface(hwnd, activatable);
        if activatable {
            // A panel waits cloaked off screen, painted and parked, until
            // it is first shown (see `surface::hide`).
            surface::hide(hwnd);
        }
    });
    Some(Surface {
        handle: any,
        hwnd: raw,
    })
}

fn logical(rect: RECT, scale: f32) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: point(px(rect.left as f32 / scale), px(rect.top as f32 / scale)),
        size: size(
            px((rect.right - rect.left) as f32 / scale),
            px((rect.bottom - rect.top) as f32 / scale),
        ),
    }
}

/// Hook the bar's window procedure for what only a window learns: AppBar
/// notifications, Explorer restarting, the display changing and the
/// session ending (when the taskbar must be given back at once, since the
/// process may not get another chance).
fn hook_bar(hwnd: isize, signals: async_channel::Sender<Signal>, took_taskbar: bool) {
    let taskbar_created = taskbar::taskbar_created_message();
    surface::subclass(
        windows_list::handle(hwnd),
        Box::new(move |_, message, wparam, _| {
            if message == appbar::CALLBACK_MESSAGE {
                if wparam.0 as u32 == windows::Win32::UI::Shell::ABN_POSCHANGED {
                    let _ = signals.try_send(Signal::Reposition);
                }
                return Some(LRESULT(0));
            }
            if message == taskbar_created && taskbar_created != 0 {
                let _ = signals.try_send(Signal::ExplorerRestarted);
            } else if matches!(message, WM_DISPLAYCHANGE | WM_DPICHANGED) {
                let _ = signals.try_send(Signal::Reposition);
            } else if message == WM_QUERYENDSESSION || (message == WM_ENDSESSION && wparam.0 != 0) {
                // The user's own taskbar setting comes back before Windows
                // ends the session.
                if took_taskbar {
                    taskbar::restore();
                }
            }
            None
        }),
    );
}

/// Place the bar and the Dock in their AppBar strips.
fn place_bars(cx: &mut App) {
    let runtime = runtime(cx);
    let (Some(bar), dock) = (runtime.bar, runtime.dock) else {
        return;
    };
    let (current_bar, current_strip) = (runtime.bar_rect, runtime.dock_strip);
    let took_taskbar = runtime.took_taskbar;
    cx.spawn(async move |cx| {
        if took_taskbar {
            taskbar::hide_windows();
        }
        let (monitor, _) = surface::primary_monitor();
        let scale = surface::scale_factor(windows_list::handle(bar.hwnd));
        let thickness = (BAR_HEIGHT * scale).round() as i32;
        let rect = appbar::reserve(
            windows_list::handle(bar.hwnd),
            Edge::Top,
            thickness,
            monitor,
            current_bar,
        );
        // Moving an AppBar notifies the others, whose answer must not
        // move it again: only a changed strip is applied.
        if rect != current_bar {
            surface::show_at(windows_list::handle(bar.hwnd), rect);
            appbar::moved(windows_list::handle(bar.hwnd));
        }
        let mut strip = RECT::default();
        if let Some(dock) = dock {
            let thickness = (dock::DOCK_HEIGHT * scale).round() as i32;
            strip = appbar::reserve(
                windows_list::handle(dock.hwnd),
                Edge::Bottom,
                thickness,
                monitor,
                current_strip,
            );
            if strip != current_strip {
                appbar::moved(windows_list::handle(dock.hwnd));
            }
        }
        if rect == current_bar && strip == current_strip {
            return;
        }
        trace(|| {
            format!(
                "bar at {},{},{},{} dock strip at {},{},{},{}",
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                strip.left,
                strip.top,
                strip.right,
                strip.bottom
            )
        });
        cx.update(|cx| {
            let runtime = cx.global_mut::<Runtime>();
            runtime.scale = scale;
            runtime.bar_rect = rect;
            runtime.dock_strip = strip;
            dock::place(cx);
        });
    })
    .detach();
}

fn start_clock(cx: &mut App) {
    let shell = shell(cx);
    shell.update(cx, |state, _| state.tick_clock());
    cx.spawn(async move |cx| loop {
        let wait = clock::millis_to_next_minute(chrono::Local::now().naive_local());
        cx.background_executor()
            .timer(Duration::from_millis(wait + 5))
            .await;
        shell.update(cx, |state, cx| {
            state.tick_clock();
            cx.notify();
        });
    })
    .detach();
}

/// Read the app catalogue, or the files, on the catalogue thread.
pub(crate) fn load_catalog(cx: &mut App) {
    let shell = shell(cx);
    let (load_apps, load_files) = shell.update(cx, |state, _| {
        let apps = matches!(state.apps_state, ListState::Missing | ListState::Stale);
        let files = matches!(state.files_state, ListState::Missing | ListState::Stale);
        if apps {
            state.apps_state = ListState::Loading;
        }
        if files {
            state.files_state = ListState::Loading;
        }
        (apps, files)
    });
    if !load_apps && !load_files {
        return;
    }
    let loaded = blocking::unblock(move || {
        catalog::init_com();
        let apps = load_apps.then(catalog::load_apps);
        let files = load_files.then(catalog::load_files);
        (apps, files)
    });
    cx.spawn(async move |cx| {
        let (apps, files) = loaded.await;
        shell.update(cx, |state, cx| {
            if let Some(apps) = apps {
                trace(|| format!("catalog: {} apps", apps.len()));
                state.apps = apps;
                if state.apps_state == ListState::Loading {
                    state.apps_state = ListState::Ready;
                }
            }
            if let Some(files) = files {
                trace(|| format!("catalog: {} files", files.len()));
                state.files = files;
                if state.files_state == ListState::Loading {
                    state.files_state = ListState::Ready;
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// Run a command chosen in the Lulo menu or a Windows app's menus.
pub(crate) fn run_shell_command(action: &str, cx: &mut App) {
    let shell = shell(cx);
    let front = shell.read(cx).front.clone();
    let front_windows = front
        .as_ref()
        .and_then(|front| {
            shell
                .read(cx)
                .tiles
                .iter()
                .find(|tile| tile.key == front.key)
                .map(|tile| tile.windows.clone())
        })
        .unwrap_or_default();
    match action {
        menus::ABOUT => launch::open(launch::Request::Shell("ms-settings:about".into())),
        menus::SYSTEM_SETTINGS => launch::open(launch::Request::Shell("ms-settings:".into())),
        menus::FORCE_QUIT => launch::open(launch::Request::Shell("taskmgr.exe".into())),
        menus::SLEEP => power::run(power::Command::Sleep),
        menus::LOCK_SCREEN => power::run(power::Command::LockScreen),
        menus::RESTART | menus::SHUT_DOWN | menus::LOG_OUT => confirm_power(action, cx),
        menus::START_AT_SIGN_IN => {
            let on = !shell.read(cx).starts_at_sign_in;
            let session = launch::install_dir().join("lulo-session.exe");
            let command = format!("\"{}\"", session.display());
            if registry::set_starts_at_sign_in(on.then_some(command.as_str())) {
                shell.update(cx, |state, cx| {
                    state.starts_at_sign_in = on;
                    cx.notify();
                });
            }
        }
        menus::TURN_OFF => quit(cx),
        menus::HIDE_APP => later(cx, move || {
            front_windows
                .iter()
                .for_each(|&hwnd| windows_list::minimize(hwnd))
        }),
        menus::QUIT_APP => later(cx, move || {
            front_windows
                .iter()
                .for_each(|&hwnd| windows_list::close(hwnd))
        }),
        menus::MINIMIZE | menus::ZOOM | menus::CLOSE_WINDOW => {
            let Some(hwnd) = front.map(|front| front.hwnd) else {
                return;
            };
            let action = action.to_owned();
            later(cx, move || match action.as_str() {
                menus::MINIMIZE => windows_list::minimize(hwnd),
                menus::ZOOM => windows_list::zoom(hwnd),
                _ => windows_list::close(hwnd),
            });
        }
        _ => {}
    }
}

/// Run a command from the front Lulo app's own menus: the app gets the
/// keyboard back first, so the command reaches its focused view.
pub(crate) fn run_app_command(action: &str, cx: &mut App) {
    let front = shell(cx).read(cx).front.as_ref().map(|front| front.hwnd);
    let action = action.to_owned();
    cx.spawn(async move |cx| {
        if let Some(hwnd) = front {
            windows_list::activate(hwnd);
        }
        cx.update(|cx| shell(cx).read(cx).send_to_front(Command::Activate(action)));
    })
    .detach();
}

/// Ask the front Lulo app to validate its menus before one opens.
pub(crate) fn validate_front_menus(cx: &mut App) {
    shell(cx).read(cx).send_to_front(Command::Validate);
}

fn confirm_power(action: &str, cx: &mut App) {
    let Some((message, detail, button)) = menus::confirmation(action) else {
        return;
    };
    let command = match action {
        menus::RESTART => power::Command::Restart,
        menus::SHUT_DOWN => power::Command::ShutDown,
        _ => power::Command::LogOut,
    };
    let Some(bar) = runtime(cx).bar else {
        return;
    };
    let answer = bar.handle.update(cx, |_, window, cx| {
        window.prompt(
            gpui::PromptLevel::Warning,
            message,
            Some(detail),
            &[button, "Cancel"],
            cx,
        )
    });
    let Ok(answer) = answer else {
        return;
    };
    cx.spawn(async move |cx| {
        if answer.await == Ok(0) {
            let (bar, dock, took_taskbar) = cx.update(|cx| {
                let runtime = runtime(cx);
                (
                    runtime.bar.map(|bar| bar.hwnd),
                    runtime.dock.map(|dock| dock.hwnd),
                    runtime.took_taskbar,
                )
            });
            // Windows is about to end the session: give the desktop back
            // first.
            restore_desktop(bar, dock, took_taskbar);
            power::run(command);
        }
    })
    .detach();
}

/// The exit code `lulo-session` sees: 0 when the user turned Lulo off.
pub fn run() -> i32 {
    trace(|| {
        format!(
            "starting pid {} at {:.0} ms",
            std::process::id(),
            crate::win::process_millis()
        )
    });
    let (menu_tx, menu_rx) = async_channel::unbounded::<MenuEvent>();
    let Some(server) = menubar_server::start(menu_tx) else {
        eprintln!("Lulo is already running.");
        return 3;
    };
    let (desktop_tx, desktop_rx) = async_channel::unbounded::<DesktopEvent>();
    let (status_tx, status_rx) = async_channel::unbounded::<StatusEvent>();
    let (icon_tx, icon_rx) = async_channel::unbounded::<(String, Option<Arc<RenderImage>>)>();
    let (catalog_tx, catalog_rx) = async_channel::unbounded::<catalog::List>();
    let (signal_tx, signal_rx) = async_channel::unbounded::<Signal>();
    let mut server = Some(server);

    rmac_ui::application()
        .with_assets(rmac_ui::layered_assets(assets::ShellAssets))
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            let Some(server) = server.take() else {
                cx.quit();
                return;
            };
            let shell = cx.new(|_| ShellState::new());
            let took_taskbar = std::env::var_os("LULO_KEEP_TASKBAR").is_none();
            cx.set_global(Runtime {
                shell: shell.clone(),
                bar: None,
                dock: None,
                spotlight: None,
                overlay: None,
                server,
                hooks: None,
                took_taskbar,
                bar_rect: RECT::default(),
                dock_strip: RECT::default(),
                scale: 1.0,
            });
            // The Dock asks for Windows apps' icons as soon as it lists them.
            icons::start(icon_tx.clone());
            shell.update(cx, |state, _| state.refresh_windows());
            let foreground = windows_list::foreground();
            if foreground != 0 {
                shell.update(cx, |state, _| state.set_foreground(foreground));
            }
            start_clock(cx);

            // The surfaces, hidden until they are placed (close to where
            // they go, so their first frame is the right size).
            let (monitor, _) = surface::primary_monitor();
            let bar_guess = RECT {
                bottom: monitor.top + BAR_HEIGHT as i32,
                ..monitor
            };
            let dock_guess = RECT {
                top: monitor.bottom - dock::DOCK_HEIGHT as i32,
                left: monitor.left + (monitor.right - monitor.left) / 4,
                right: monitor.right - (monitor.right - monitor.left) / 4,
                ..monitor
            };
            let bar = open_surface(logical(bar_guess, 1.0), false, cx, bar::BarView::new);
            let dock = open_surface(logical(dock_guess, 1.0), false, cx, dock::DockView::new);
            let (Some(bar), Some(dock)) = (bar, dock) else {
                eprintln!("lulo-shell: the menu bar or the Dock could not open");
                quit(cx);
                return;
            };
            {
                let runtime = cx.global_mut::<Runtime>();
                runtime.bar = Some(bar);
                runtime.dock = Some(dock);
            }
            // A crash leaves these for `lulo-session` to remove.
            registry::set_dword("BarWindow", bar.hwnd as u32);
            registry::set_dword("DockWindow", dock.hwnd as u32);
            {
                let signals = signal_tx.clone();
                later(cx, move || {
                    appbar::register(windows_list::handle(bar.hwnd));
                    appbar::register(windows_list::handle(dock.hwnd));
                    if took_taskbar {
                        taskbar::take_over();
                    }
                    hook_bar(bar.hwnd, signals, took_taskbar);
                });
            }
            place_bars(cx);
            later(cx, || {
                trace(|| format!("ready at {:.0} ms", crate::win::process_millis()))
            });

            {
                let signals = signal_tx.clone();
                crate::win::session::on_stop_request(move || {
                    let _ = signals.try_send(Signal::Stop);
                });
            }
            let hooks = events::start(desktop_tx.clone());
            trace(|| {
                format!(
                    "spotlight hotkey Alt+Space {}",
                    if hooks.as_ref().is_some_and(|hooks| hooks.hotkey) {
                        "registered"
                    } else {
                        "unavailable"
                    }
                )
            });
            cx.global_mut::<Runtime>().hooks = hooks;
            crate::win::status::start(status_tx.clone());
            {
                let catalog_tx = catalog_tx.clone();
                catalog::watch(move |list| {
                    let _ = catalog_tx.try_send(list);
                });
            }
            load_catalog(cx);
            // Spotlight opens on a hotkey and must be there at once: make
            // its window now, after the bar and the Dock are on screen.
            cx.defer(spotlight::prepare);

            spawn_receivers(
                cx,
                shell,
                ReceiverSet {
                    desktop: desktop_rx.clone(),
                    menus: menu_rx.clone(),
                    status: status_rx.clone(),
                    icons: icon_rx.clone(),
                    catalog: catalog_rx.clone(),
                    signals: signal_rx.clone(),
                },
            );

            // Any other way out (Windows ending the session) gives the
            // desktop back as well; `run` checks again once GPUI stops.
            cx.on_app_quit(|cx| {
                let runtime = runtime(cx);
                let (bar, dock, took_taskbar) = (
                    runtime.bar.map(|bar| bar.hwnd),
                    runtime.dock.map(|dock| dock.hwnd),
                    runtime.took_taskbar,
                );
                async move { restore_desktop(bar, dock, took_taskbar) }
            })
            .detach();
        });
    // Whatever stopped the loop, the desktop is the user's again.
    if !CLEANED_UP.load(Ordering::Acquire) {
        taskbar::restore();
    }
    0
}

struct ReceiverSet {
    desktop: async_channel::Receiver<DesktopEvent>,
    menus: async_channel::Receiver<MenuEvent>,
    status: async_channel::Receiver<StatusEvent>,
    icons: async_channel::Receiver<(String, Option<Arc<RenderImage>>)>,
    catalog: async_channel::Receiver<catalog::List>,
    signals: async_channel::Receiver<Signal>,
}

fn spawn_receivers(cx: &mut App, shell: Entity<ShellState>, receivers: ReceiverSet) {
    let ReceiverSet {
        desktop,
        menus: menu_events,
        status,
        icons: icon_replies,
        catalog: catalog_changes,
        signals,
    } = receivers;
    {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = desktop.recv().await {
                match event {
                    // Surfaces redraw only when what they show changed: other
                    // apps' windows come and go all the time.
                    DesktopEvent::Foreground(hwnd) => shell.update(cx, |state, cx| {
                        let before = state.front_key();
                        state.set_foreground(hwnd);
                        if state.front_key() != before {
                            cx.notify();
                        }
                    }),
                    DesktopEvent::WindowsChanged => {
                        let changed = shell.update(cx, |state, cx| {
                            let before = (state.tiles.clone(), state.front_key());
                            state.refresh_windows();
                            let changed = before.0 != state.tiles;
                            if changed || before.1 != state.front_key() {
                                cx.notify();
                            }
                            changed
                        });
                        if changed {
                            cx.update(dock::place);
                        }
                    }
                    DesktopEvent::Hotkey => cx.update(spotlight::toggle),
                }
            }
        })
        .detach();
    }
    {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = menu_events.recv().await {
                let menus_changed = matches!(event, MenuEvent::Menus { .. });
                shell.update(cx, |state, cx| {
                    match event {
                        MenuEvent::Menus { pid, app_id, menus } => {
                            trace(|| format!("menus from {app_id} ({pid}): {}", menus.len()));
                            state.lulo_menus.insert(pid, (app_id, menus));
                        }
                        MenuEvent::Commands { pid, writer } => {
                            state.commands.insert(pid, Arc::new(Mutex::new(writer)));
                        }
                        MenuEvent::Gone { pid } => {
                            state.lulo_menus.remove(&pid);
                            state.commands.remove(&pid);
                        }
                    }
                    cx.notify();
                });
                if menus_changed {
                    cx.update(menu::refresh);
                }
            }
        })
        .detach();
    }
    {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = status.recv().await {
                shell.update(cx, |state, cx| {
                    match event {
                        StatusEvent::Battery(battery) => state.battery = battery,
                        StatusEvent::Volume(volume) => state.volume = volume,
                        StatusEvent::Wifi(wifi) => state.wifi = wifi,
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
    {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            while let Ok((source, icon)) = icon_replies.recv().await {
                shell.update(cx, |state, cx| {
                    state.icons.insert(source, icon);
                    cx.notify();
                });
            }
        })
        .detach();
    }
    {
        let shell = shell.clone();
        cx.spawn(async move |cx| {
            while let Ok(list) = catalog_changes.recv().await {
                shell.update(cx, |state, _| match list {
                    catalog::List::Apps if state.apps_state == ListState::Ready => {
                        state.apps_state = ListState::Stale
                    }
                    catalog::List::Files if state.files_state == ListState::Ready => {
                        state.files_state = ListState::Stale
                    }
                    _ => {}
                });
            }
        })
        .detach();
    }
    cx.spawn(async move |cx| {
        while let Ok(signal) = signals.recv().await {
            if let Signal::Stop = signal {
                cx.update(quit);
                continue;
            }
            if let Signal::ExplorerRestarted = signal {
                trace(|| "Explorer restarted".into());
                let (surfaces, took_taskbar) = cx.update(|cx| {
                    let runtime = runtime(cx);
                    ([runtime.bar, runtime.dock], runtime.took_taskbar)
                });
                for surface in surfaces.into_iter().flatten() {
                    appbar::register(windows_list::handle(surface.hwnd));
                }
                if took_taskbar {
                    taskbar::take_over();
                }
                // A new Explorer knows none of the old strips.
                cx.update(|cx| {
                    let runtime = cx.global_mut::<Runtime>();
                    runtime.bar_rect = RECT::default();
                    runtime.dock_strip = RECT::default();
                });
            }
            cx.update(place_bars);
        }
    })
    .detach();
}
