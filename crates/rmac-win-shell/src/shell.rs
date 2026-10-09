//! `lulo-shell`: Lulo OS's own menu bar, Dock and desktop views
//! (`shell/bins`), running on Windows in one GPUI process (one Direct3D
//! device and font collection for them all, which keeps start-up and memory
//! small on 8 GB PCs). ADR 0023, "Phase 3 revised: shared shell views".
//!
//! The views are the same code as on Lulo OS; this module only gives them
//! Windows underneath:
//!
//! - their surfaces are Win32 windows placed and layered as layer surfaces
//!   (`rmac-shell-layer`), the bar and the Dock holding AppBar strips;
//! - the window list, focus and window actions come from
//!   `rmac-compositor-system`'s Win32 backend, the status items from
//!   `rmac-shell-runtime`'s Windows sources, Lulo apps' menus from
//!   `rmac-app-menu`'s pipe host;
//! - the Lulo apps' desktop entries and artwork are laid out as on Lulo OS
//!   (`share`), and Lulo's font is Inter (`rmac-ui`).
//!
//! What stays Windows' own: the taskbar and Explorer's desktop icons are
//! put away while Lulo runs and given back on every way out (Turn Off, the
//! session ending, a crash through `lulo-session`), Spotlight's hotkey and
//! its fallbacks, Use Files for Folders, Start Lulo at Sign-In, and the
//! memory trims.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{App, AssetSource, Global, SharedString};
use rmac_shell_layer::system::{extra_menu, requests};
use windows::Win32::Foundation::{HWND, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION};

use crate::model::hotkey as model_hotkey;
use crate::share;
use crate::win::events::{self, DesktopEvent, Hooks};
use crate::win::{desktop as explorer_desktop, folders, memory, registry, surface, taskbar, trace};

/// The registry value recording the Spotlight hotkey the user was last
/// told about (`model::hotkey::Hotkey::code`).
const HOTKEY_NOTICE: &str = "SpotlightHotkeyNotice";

/// The Lulo menu rows only Windows has.
const SIGN_IN_ACTION: &str = "lulo::start-at-sign-in";
const FILES_FOR_FOLDERS_ACTION: &str = "lulo::files-for-folders";
const TURN_OFF_ACTION: &str = "lulo::turn-off";

static CLEANED_UP: AtomicBool = AtomicBool::new(false);

/// The surfaces that make the layer ready once each is placed.
#[derive(Clone, Copy)]
enum Placed {
    Bar = 1,
    Dock = 2,
    Desktop = 4,
}

thread_local! {
    /// The bar windows already watched for the session ending.
    static WATCHED: std::cell::RefCell<std::collections::HashSet<isize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static PLACED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// Who waits for the layer to be ready.
    static READY: std::cell::RefCell<Vec<async_channel::Sender<()>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The Dock's own surface (`rmac-dock-<display>`), not its material or
/// keyboard surfaces.
fn is_dock(namespace: &str) -> bool {
    namespace
        .strip_prefix("rmac-dock-")
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// Say the layer is ready (the CI checks and `lulo-session` wait for it)
/// the first time the bar, the Dock and the desktop have all been placed.
fn placed_for_ready(surface: Placed) {
    let before = PLACED.with(|placed| placed.get());
    let after = before | surface as u8;
    PLACED.with(|placed| placed.set(after));
    if before != 7 && after == 7 {
        trace(|| format!("ready at {:.0} ms", crate::win::process_millis()));
        memory::report("ready");
        // Windows' own apps for Spotlight and the Dock, read by a helper
        // process off the UI thread (not in the fixed scene: the runner's
        // apps are not Lulo OS's).
        if std::env::var_os("RMAC_SHELL_SCENE").is_none()
            && std::env::var_os("LULO_NO_WINDOWS_APPS").is_none()
        {
            crate::win::app_entries::start();
        }
        for ready in READY.with(|ready| std::mem::take(&mut *ready.borrow_mut())) {
            let _ = ready.try_send(());
        }
    }
}

/// What the shell keeps for its life.
struct Runtime {
    took_taskbar: bool,
    menu_host: rmac_app_menu::pipe_host::PipeHost,
    hooks: Option<Hooks>,
}

impl Global for Runtime {}

/// Every surface's artwork in one source: the bar's, the desktop's, and
/// rmac-ui's shared icons under them.
struct ShellAssets;

impl AssetSource for ShellAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(rmac_shell_menubar::asset(path)
            .or_else(|| rmac_shell_wallpaper::asset(path))
            .or_else(|| rmac_launcher_app::asset(path))
            .or_else(|| rmac_quick_settings_app::asset(path)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = rmac_shell_menubar::asset_names()
            .chain(rmac_shell_wallpaper::asset_names())
            .filter(|name| name.starts_with(path))
            .map(SharedString::from)
            .collect();
        names.extend(
            rmac_launcher_app::asset_names(path)
                .into_iter()
                .chain(rmac_quick_settings_app::asset_names(path))
                .map(SharedString::from),
        );
        Ok(names)
    }
}

/// Give the desktop back: the AppBars' strips, the taskbar and Explorer's
/// desktop icons. Safe to call more than once and from a window procedure.
fn restore_desktop(took_taskbar: bool) {
    if CLEANED_UP.swap(true, Ordering::AcqRel) {
        return;
    }
    rmac_shell_layer::windows::release_appbars();
    if took_taskbar {
        taskbar::restore();
    }
    explorer_desktop::restore();
    registry::delete_value("BarWindow");
    registry::delete_value("DockWindow");
}

/// Turn the Lulo layer off.
fn quit(cx: &mut App) {
    trace(|| "quitting".into());
    let runtime = cx.global::<Runtime>();
    runtime.menu_host.stop();
    if let Some(hooks) = &runtime.hooks {
        hooks.stop();
    }
    let took_taskbar = runtime.took_taskbar;
    cx.spawn(async move |cx| {
        restore_desktop(took_taskbar);
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// The bar's window learns of the session ending: the user's taskbar and
/// desktop come back before Windows ends it, since the process may not get
/// another chance.
fn watch_session_end(hwnd: HWND, took_taskbar: bool) {
    surface::subclass(
        hwnd,
        Box::new(move |_, message, wparam, _| {
            if message == WM_QUERYENDSESSION || (message == WM_ENDSESSION && wparam.0 != 0) {
                if took_taskbar {
                    taskbar::restore();
                }
                explorer_desktop::restore();
            }
            None::<LRESULT>
        }),
    );
}

/// The Lulo menu's Windows rows.
fn add_menu_rows() {
    extra_menu::add(extra_menu::Row {
        label: "Start Lulo at Sign-In",
        action: SIGN_IN_ACTION,
        separated: true,
        checked: Some(std::rc::Rc::new(registry::starts_at_sign_in)),
    });
    extra_menu::add(extra_menu::Row {
        label: "Use Files for Folders",
        action: FILES_FOR_FOLDERS_ACTION,
        separated: false,
        checked: Some(std::rc::Rc::new(folders::is_on)),
    });
    extra_menu::add(extra_menu::Row {
        label: "Turn Off Lulo",
        action: TURN_OFF_ACTION,
        separated: true,
        checked: None,
    });
    requests::register(SIGN_IN_ACTION, |_| {
        let on = !registry::starts_at_sign_in();
        let session = share::install_dir().join("lulo-session.exe");
        let command = format!("\"{}\"", session.display());
        registry::set_starts_at_sign_in(on.then_some(command.as_str()));
        trace(|| format!("start at sign-in {}", if on { "on" } else { "off" }));
    });
    requests::register(FILES_FOR_FOLDERS_ACTION, |_| {
        let on = !folders::is_on();
        let files = share::install_dir().join("rmac-files.exe");
        if on {
            if files.is_file() {
                folders::turn_on(&files);
            }
        } else {
            folders::turn_off();
        }
        trace(|| format!("files for folders {}", if on { "on" } else { "off" }));
    });
    requests::register(TURN_OFF_ACTION, quit);
}

/// Spotlight's hotkey (and its fallbacks), and the one-time word on which
/// key it is when it is not Alt+Space.
fn start_hotkey(cx: &mut App) -> Option<Hooks> {
    let (sender, receiver) = async_channel::unbounded::<DesktopEvent>();
    let hooks = events::start(sender);
    let hotkey = hooks.as_ref().and_then(|hooks| hooks.hotkey);
    trace(|| match hotkey {
        Some(hotkey) if hotkey.is_fallback() => {
            format!("spotlight hotkey {} (fallback)", hotkey.label())
        }
        Some(hotkey) => format!("spotlight hotkey {} (first choice)", hotkey.label()),
        None => "spotlight hotkey none (the menu bar's icon opens Spotlight)".into(),
    });
    if let Some(hotkey) = hotkey {
        if model_hotkey::should_notice(hotkey, registry::get_dword(HOTKEY_NOTICE)) {
            registry::set_dword(HOTKEY_NOTICE, hotkey.code());
            let (title, detail) = model_hotkey::notice(hotkey);
            trace(|| format!("notice shown: {title}: {detail}"));
        }
    }
    cx.spawn(async move |cx| {
        while let Ok(event) = receiver.recv().await {
            if let DesktopEvent::Hotkey = event {
                cx.update(|cx| {
                    if !requests::run("launcher", cx) {
                        trace(|| "spotlight is not available".into());
                    }
                });
            }
        }
    })
    .detach();
    hooks
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
    let Some(menu_host) = rmac_app_menu::pipe_host::start() else {
        eprintln!("Lulo is already running.");
        return 3;
    };
    share::install(&share::install_dir());
    let took_taskbar = std::env::var_os("LULO_KEEP_TASKBAR").is_none();
    let mut menu_host = Some(menu_host);

    rmac_ui::application()
        .with_assets(rmac_ui::layered_assets(ShellAssets))
        .run(move |cx: &mut App| {
            memory::report("GPUI started");
            rmac_ui::init_application(cx);
            rmac_shell_ui::tokens::install_appearance_watch(cx);
            let Some(menu_host) = menu_host.take() else {
                cx.quit();
                return;
            };
            cx.set_global(Runtime {
                took_taskbar,
                menu_host,
                hooks: None,
            });

            // The taskbar and Explorer's desktop icons out of the way, each
            // recorded first so every way out gives them back.
            if took_taskbar {
                taskbar::take_over();
            }
            explorer_desktop::hide_icons();
            rmac_shell_layer::system::before_session_end(move || restore_desktop(took_taskbar));
            rmac_shell_layer::windows::on_strip(move |hwnd, namespace| {
                // Explorer shows the taskbar again as AppBars come and go
                // (and a restarted Explorer forgets the setting): put it
                // away each time the bar takes its strip.
                if namespace.starts_with("rmac-top-bar") && took_taskbar {
                    taskbar::take_over();
                } else if is_dock(namespace) {
                    // A crash leaves this for `lulo-session` to remove.
                    registry::set_dword("DockWindow", hwnd as u32);
                    trace(|| format!("dock window {hwnd}"));
                }
            });
            // A closed panel (Spotlight, Control Centre, a menu) leaves its
            // frame buffers and search state in freed heap pages: give them
            // back once it is gone.
            {
                let executor = cx.background_executor().clone();
                rmac_shell_layer::windows::on_closed(move |_, namespace| {
                    let panel = ["rmac-launcher", "rmac-quick-settings", "rmac-menu-material"]
                        .iter()
                        .any(|prefix| namespace.starts_with(prefix));
                    if panel {
                        let timer = executor.timer(Duration::from_secs(2));
                        executor
                            .spawn(async move {
                                timer.await;
                                memory::trim("panel closed");
                            })
                            .detach();
                    }
                });
            }
            rmac_shell_layer::windows::on_placed(move |hwnd, namespace| {
                if namespace.starts_with("rmac-top-bar") {
                    if took_taskbar {
                        taskbar::hide_windows();
                    }
                    if WATCHED.with(|watched| watched.borrow_mut().insert(hwnd)) {
                        // A crash leaves this for `lulo-session` to remove;
                        // the session ending reaches the bar's window.
                        registry::set_dword("BarWindow", hwnd as u32);
                        watch_session_end(HWND(hwnd as *mut core::ffi::c_void), took_taskbar);
                        trace(|| format!("bar window {hwnd}"));
                    }
                    placed_for_ready(Placed::Bar);
                } else if namespace.starts_with("rmac-wallpaper") {
                    trace(|| format!("desktop window {hwnd}"));
                    placed_for_ready(Placed::Desktop);
                } else if is_dock(namespace) {
                    placed_for_ready(Placed::Dock);
                }
            });
            add_menu_rows();

            // Lulo OS's views, as on Lulo OS: the desktop under every app
            // window, the menu bar, the Dock.
            rmac_shell_wallpaper::start(cx);
            memory::report("desktop started");
            rmac_shell_menubar::start(cx);
            memory::report("menu bar started");
            let _dock = rmac_shell_dock::start(cx);
            memory::report("Dock started");
            // Spotlight and Control Centre open on demand: the hotkey and
            // the bar's buttons ask for them by name.
            rmac_launcher_app::start(cx);
            memory::report("Spotlight started");
            // A desktop shortcut shows its target's icon, and a running
            // Windows app its program's, read out of process.
            rmac_shell_layer::system::file_icons::provide(crate::win::shell_icons::icon);
            if std::env::var_os("RMAC_SHELL_SCENE").is_none()
                && std::env::var_os("LULO_NO_WINDOWS_APPS").is_none()
            {
                crate::win::shell_icons::watch_running(cx);
            }
            rmac_quick_settings_app::start(cx);
            requests::register("launcher", rmac_launcher_app::toggle);
            requests::register("quick-settings", rmac_quick_settings_app::toggle);
            memory::report("surfaces started");

            {
                cx.spawn(async move |cx| {
                    let (sender, receiver) = async_channel::bounded::<()>(1);
                    crate::win::session::on_stop_request(move || {
                        let _ = sender.try_send(());
                    });
                    if receiver.recv().await.is_ok() {
                        cx.update(quit);
                    }
                })
                .detach();
            }
            // Start-up memory every 250 ms for three seconds, for the CI
            // memory gate's breakdown.
            if std::env::var_os("LULO_SHELL_TRACE").is_some() {
                let executor = cx.background_executor().clone();
                cx.background_executor()
                    .spawn(async move {
                        for step in 1..=12u32 {
                            executor.timer(Duration::from_millis(250)).await;
                            memory::report(&format!("at +{} ms", step * 250));
                        }
                    })
                    .detach();
            }
            let hooks = start_hotkey(cx);
            cx.global_mut::<Runtime>().hooks = hooks;

            // Once the bar, the Dock and the desktop are placed the layer
            // is ready; once it is idle, what setup touched is given back.
            {
                let executor = cx.background_executor().clone();
                let (ready_tx, ready_rx) = async_channel::bounded::<()>(1);
                READY.with(|ready| ready.borrow_mut().push(ready_tx));
                cx.background_executor()
                    .spawn(async move {
                        if ready_rx.recv().await.is_ok() {
                            executor.timer(Duration::from_secs(2)).await;
                            memory::trim("idle after start-up");
                            // The catalogues, icons and status readers
                            // settle a few seconds later.
                            executor.timer(Duration::from_secs(6)).await;
                            memory::trim("settled after start-up");
                        }
                    })
                    .detach();
            }

            // The cross-platform shell scene's Spotlight or Control
            // Centre (`RMAC_SHELL_SCENE_OPEN`), once the layer is ready.
            if std::env::var_os("RMAC_SHELL_SCENE_OPEN").is_some() {
                let (scene_tx, scene_rx) = async_channel::bounded::<()>(1);
                READY.with(|ready| ready.borrow_mut().push(scene_tx));
                cx.spawn(async move |cx| {
                    if scene_rx.recv().await.is_ok() {
                        cx.update(|cx| {
                            rmac_launcher_app::open_scene(cx);
                            rmac_quick_settings_app::open_scene(cx);
                        });
                    }
                })
                .detach();
            }

            // Any other way out (Windows ending the session) gives the
            // desktop back as well; `run` checks again once GPUI stops.
            cx.on_app_quit(move |_| async move { restore_desktop(took_taskbar) })
                .detach();
        });
    // Whatever stopped the loop, the desktop is the user's again.
    if !CLEANED_UP.load(Ordering::Acquire) {
        taskbar::restore();
        explorer_desktop::restore();
    }
    0
}
