//! Layer surfaces on Windows (ADR 0023, "Phase 3 revised: shared shell
//! views"). GPUI has no layer shell there, so a surface the shell's views
//! describe with [`LayerShellOptions`] becomes a borderless pop-up window:
//!
//! - **Placement.** Its anchors, margins and size are resolved against its
//!   monitor exactly as wlr-layer-shell resolves them, in logical pixels,
//!   then placed in physical pixels. A display or DPI change places every
//!   surface again.
//! - **Layers.** `Overlay` and `Top` are topmost windows, `Overlay` kept
//!   above `Top` (the menu bar's menus over the Dock and the menu
//!   materials). `Background` and `Bottom` sit directly above Explorer's
//!   desktop and below every app window, and refuse any other change of
//!   their place in the z-order.
//! - **Exclusive zones** are AppBars: Windows takes the strip out of the
//!   work area, so maximised windows stop below the bar and above the Dock.
//! - **Keyboard.** `None` never takes the foreground (a click on the Dock
//!   leaves the app in front with the keyboard); `OnDemand` takes it when
//!   clicked; `Exclusive` takes it at once.
//! - **Input regions** (`Window::set_input_region`) are window regions, set
//!   by the vendored `gpui_windows`: outside them clicks reach the windows
//!   below, as on Lulo OS.
//!
//! Nothing here polls: placement happens when a surface opens and when
//! Windows reports a display, work-area or Explorer change.

pub mod appbar;
pub mod desktop_layer;
pub mod layer_types;
pub mod material;
pub mod power;
pub mod surface;
pub mod trace;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{
    point, px, size, AnyWindowHandle, App, Bounds, Entity, Pixels, PlatformDisplay, Render, Size,
    Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use uuid::Uuid;
use windows::Win32::Foundation::{HWND, LRESULT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITORINFOEXW,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, RegisterWindowMessageW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, WINDOWPOS, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEACTIVATE, WM_NCDESTROY,
    WM_SETTINGCHANGE, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST,
};

pub use layer_types::*;

/// A shell surface this process opened, and how it was described.
struct LayerWindow {
    hwnd: isize,
    layer: LayerShellOptions,
    /// The monitor it was opened on (GPUI's display id is the `HMONITOR`).
    display: u64,
    requested: Size<Pixels>,
    /// Asked for `WindowBackgroundAppearance::Blurred` (see `material`).
    blurred: bool,
    /// Asked for keyboard focus when it opens (`WindowOptions::focus` on a
    /// surface that takes the keyboard on demand or exclusively).
    focus: bool,
    styled: bool,
    /// The strip it holds as an AppBar.
    strip: Option<RECT>,
}

#[derive(Clone, Copy, Debug)]
enum Signal {
    /// Place every surface again (a display, DPI or work-area change).
    Replace,
    /// Explorer restarted and forgot every AppBar.
    ExplorerRestarted,
}

/// Callbacks with a surface's window and namespace.
type Hooks = RefCell<Vec<Rc<dyn Fn(isize, &str)>>>;

thread_local! {
    static WINDOWS: RefCell<Vec<LayerWindow>> = const { RefCell::new(Vec::new()) };
    static SIGNALS: RefCell<Option<async_channel::Sender<Signal>>> = const { RefCell::new(None) };
    static BACKGROUND_WATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static STRIP_HOOKS: Hooks = const { RefCell::new(Vec::new()) };
    static PLACED_HOOKS: Hooks = const { RefCell::new(Vec::new()) };
}

/// Run `hook` with a surface's window and namespace when it first holds an
/// AppBar strip (the Windows shell records them, so `lulo-session` can give
/// the work area back after a crash, and watches the bar's window for the
/// session ending).
pub fn on_strip(hook: impl Fn(isize, &str) + 'static) {
    STRIP_HOOKS.with(|hooks| hooks.borrow_mut().push(Rc::new(hook)));
}

/// Run `hook` with a surface's window and namespace each time it is
/// placed (the Windows shell keeps its wallpaper layer under the desktop).
pub fn on_placed(hook: impl Fn(isize, &str) + 'static) {
    PLACED_HOOKS.with(|hooks| hooks.borrow_mut().push(Rc::new(hook)));
}

fn run_hooks(hooks: &'static std::thread::LocalKey<Hooks>, raw: isize, namespace: &str) {
    let hooks = hooks.with(|hooks| hooks.borrow().clone());
    for hook in hooks {
        hook(raw, namespace);
    }
}

fn handle(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

/// Open `build`'s view as the shell surface `layer` describes (see the
/// module documentation). `options.window_bounds`' size is the surface's
/// requested size, as for a layer surface.
pub fn open_layer_window<V: 'static + Render>(
    cx: &mut App,
    mut options: WindowOptions,
    layer: LayerShellOptions,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> gpui::Result<WindowHandle<V>> {
    let signals = ensure_signals(cx);
    let display = options
        .display_id
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display());
    let requested = options
        .window_bounds
        .map(|bounds| bounds.get_bounds().size)
        .unwrap_or(size(px(1.0), px(1.0)));
    if let Some(display) = &display {
        options.window_bounds = Some(WindowBounds::Windowed(logical_bounds(
            &layer,
            requested,
            display.bounds(),
        )));
        options.display_id = Some(display.id());
    }
    options.kind = WindowKind::PopUp;
    let focus = options.focus && layer.keyboard_interactivity != KeyboardInteractivity::None;
    // Shown once placed and styled, so the first frame is where it belongs.
    options.show = false;
    options.focus = false;
    options.is_movable = false;
    options.is_resizable = false;
    options.is_minimizable = false;
    let background = matches!(layer.layer, Layer::Background | Layer::Bottom);
    let blurred = options.window_background == gpui::WindowBackgroundAppearance::Blurred;
    let handle = cx.open_window(options, build)?;
    let any: AnyWindowHandle = handle.into();
    if let Some(hwnd) = any
        .update(cx, |_, window, _| surface::hwnd(window))
        .ok()
        .flatten()
    {
        let raw = hwnd.0 as isize;
        WINDOWS.with(|windows| {
            windows.borrow_mut().push(LayerWindow {
                hwnd: raw,
                layer,
                display: display.map_or(0, |display| u64::from(display.id())),
                requested,
                blurred,
                focus,
                styled: false,
                strip: None,
            })
        });
        hook(hwnd, signals);
        if trace::enabled() {
            let namespace = WINDOWS.with(|windows| {
                windows
                    .borrow()
                    .iter()
                    .find(|window| window.hwnd == raw)
                    .map(|window| window.layer.namespace.clone())
                    .unwrap_or_default()
            });
            let _ = any.update(cx, |_, window, _| {
                window.on_next_frame(move |_, _| trace::memory(&format!("{namespace} drawn")));
            });
        }
        // Win32 calls that send messages to this process's own windows run
        // outside GPUI's update, as `gpui_windows` does for its own.
        cx.spawn(async move |_| place(raw)).detach();
        if background {
            watch_desktop_foreground(cx);
        }
    }
    Ok(handle)
}

/// Give the open surface `handle` a new requested size and place it again,
/// as a layer surface's `set_size` does. Windows can resize a window in
/// place, so a view whose surface only changes size keeps its window (and
/// any strip it holds) instead of opening another.
pub fn resize_layer(cx: &mut App, handle: AnyWindowHandle, requested: Size<Pixels>) {
    let Some(raw) = handle
        .update(cx, |_, window, _| surface::hwnd(window))
        .ok()
        .flatten()
        .map(|hwnd| hwnd.0 as isize)
    else {
        return;
    };
    let changed = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let window = windows.iter_mut().find(|window| window.hwnd == raw)?;
        (window.requested != requested).then(|| window.requested = requested)
    });
    if changed.is_some() {
        cx.spawn(async move |_| place(raw)).detach();
    }
}

/// Where a layer surface goes on a display whose logical bounds are
/// `display`: anchored edges hold it there (both of two opposite edges
/// stretch it), margins push it in, and an axis with no anchor centres it,
/// as wlr-layer-shell does.
pub fn logical_bounds(
    layer: &LayerShellOptions,
    requested: Size<Pixels>,
    display: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let (top, right, bottom, left) = layer.margin.unwrap_or((px(0.0), px(0.0), px(0.0), px(0.0)));
    let anchor = layer.anchor;
    let axis = |start: Pixels,
                extent: Pixels,
                wanted: Pixels,
                low: bool,
                high: bool,
                margin_low: Pixels,
                margin_high: Pixels| {
        match (low, high) {
            (true, true) => (start + margin_low, extent - margin_low - margin_high),
            (true, false) => (start + margin_low, wanted),
            (false, true) => (start + extent - margin_high - wanted, wanted),
            (false, false) => (start + (extent - wanted) / 2.0, wanted),
        }
    };
    let (x, width) = axis(
        display.origin.x,
        display.size.width,
        requested.width,
        anchor.contains(Anchor::LEFT),
        anchor.contains(Anchor::RIGHT),
        left,
        right,
    );
    let (y, height) = axis(
        display.origin.y,
        display.size.height,
        requested.height,
        anchor.contains(Anchor::TOP),
        anchor.contains(Anchor::BOTTOM),
        top,
        bottom,
    );
    Bounds {
        origin: point(x, y),
        size: size(width.max(px(1.0)), height.max(px(1.0))),
    }
}

/// The edge an exclusive zone holds: the one anchored edge (alone, or with
/// both edges across it), or the one asked for.
fn exclusive_edge(layer: &LayerShellOptions) -> Option<appbar::Edge> {
    if let Some(edge) = layer.exclusive_edge {
        return edge_of(edge);
    }
    let anchor = layer.anchor;
    let top = anchor.contains(Anchor::TOP);
    let bottom = anchor.contains(Anchor::BOTTOM);
    let left = anchor.contains(Anchor::LEFT);
    let right = anchor.contains(Anchor::RIGHT);
    match (top, bottom, left, right) {
        (true, false, _, _) if left == right => Some(appbar::Edge::Top),
        (false, true, _, _) if left == right => Some(appbar::Edge::Bottom),
        (_, _, true, false) if top == bottom => Some(appbar::Edge::Left),
        (_, _, false, true) if top == bottom => Some(appbar::Edge::Right),
        _ => None,
    }
}

fn edge_of(anchor: Anchor) -> Option<appbar::Edge> {
    if anchor == Anchor::TOP {
        Some(appbar::Edge::Top)
    } else if anchor == Anchor::BOTTOM {
        Some(appbar::Edge::Bottom)
    } else if anchor == Anchor::LEFT {
        Some(appbar::Edge::Left)
    } else if anchor == Anchor::RIGHT {
        Some(appbar::Edge::Right)
    } else {
        None
    }
}

/// A monitor's whole area (physical pixels) and its scale.
fn monitor_area(monitor: HMONITOR) -> Option<(RECT, f32)> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a MONITORINFO whose size field says so.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: plain out-parameters.
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    Some((info.rcMonitor, dpi_x.max(1) as f32 / 96.0))
}

fn primary_monitor() -> HMONITOR {
    // SAFETY: no pointers.
    unsafe {
        MonitorFromPoint(
            windows::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        )
    }
}

/// The device name of a monitor (`\\.\DISPLAY1`), as
/// `rmac-compositor-system` names its outputs.
pub fn monitor_device_name(monitor: HMONITOR) -> Option<String> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: a MONITORINFOEXW whose size field says so.
    if !unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    }
    .as_bool()
    {
        return None;
    }
    let end = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    Some(String::from_utf16_lossy(&info.szDevice[..end]))
}

/// GPUI's displays by the stable output id the compositor snapshot uses.
pub fn newest_displays(cx: &App) -> BTreeMap<Uuid, Rc<dyn PlatformDisplay>> {
    cx.displays()
        .into_iter()
        .filter_map(|display| {
            let monitor = HMONITOR(u64::from(display.id()) as isize as *mut core::ffi::c_void);
            let name = monitor_device_name(monitor)?;
            Some((
                crate::stable_output_uuid(&rmac_compositor::OutputId(name)),
                display,
            ))
        })
        .collect()
}

fn style(hwnd: HWND, layer: &LayerShellOptions, blurred: bool) {
    surface::make_borderless(hwnd);
    surface::plain_without_shadow(hwnd);
    if blurred {
        material::apply(hwnd);
    }
    trace::trace(|| {
        let kind = match (blurred, material::transparency_effects()) {
            (false, _) => "none",
            (true, true) => "blur",
            (true, false) => "tint only",
        };
        format!("backdrop {}: {kind}", layer.namespace)
    });
    let background = matches!(layer.layer, Layer::Background | Layer::Bottom);
    let never_active = layer.keyboard_interactivity == KeyboardInteractivity::None;
    // SAFETY: style bits on a window this process owns.
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        style |= WS_EX_TOOLWINDOW.0 as isize;
        style &= !(WS_EX_APPWINDOW.0 as isize);
        if background {
            style &= !(WS_EX_TOPMOST.0 as isize);
        } else {
            style |= WS_EX_TOPMOST.0 as isize;
        }
        if never_active {
            style |= WS_EX_NOACTIVATE.0 as isize;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
    }
    if never_active {
        surface::subclass(
            hwnd,
            Box::new(|_, message, _, _| {
                (message == WM_MOUSEACTIVATE).then_some(LRESULT(MA_NOACTIVATE as isize))
            }),
        );
    }
    if background {
        surface::subclass(
            hwnd,
            Box::new(|_, message, _, lparam| {
                if message == WM_WINDOWPOSCHANGING
                    && lparam.0 != 0
                    && !desktop_layer::own_placement()
                {
                    // SAFETY: for WM_WINDOWPOSCHANGING, `lparam` points at the
                    // WINDOWPOS Windows is about to apply; changing its flags
                    // is how a window declines part of the change.
                    let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
                    position.flags |= SWP_NOZORDER;
                }
                None
            }),
        );
    }
}

/// Watch the window for what only a window learns: display, DPI and
/// work-area changes, other AppBars moving, Explorer restarting, and its
/// own end.
fn hook(hwnd: HWND, signals: async_channel::Sender<Signal>) {
    // SAFETY: registers (or looks up) a system-wide message name.
    let taskbar_created = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) };
    surface::subclass(
        hwnd,
        Box::new(move |hwnd, message, wparam, _| {
            if message == appbar::CALLBACK_MESSAGE {
                if wparam.0 as u32 == windows::Win32::UI::Shell::ABN_POSCHANGED {
                    let _ = signals.try_send(Signal::Replace);
                }
                return Some(LRESULT(0));
            }
            if taskbar_created != 0 && message == taskbar_created {
                let _ = signals.try_send(Signal::ExplorerRestarted);
            } else if matches!(message, WM_DISPLAYCHANGE | WM_DPICHANGED)
                // SPI_SETWORKAREA: the work area changed under the surface.
                || (message == WM_SETTINGCHANGE && wparam.0 == 0x002F)
            {
                let _ = signals.try_send(Signal::Replace);
            } else if message == WM_NCDESTROY {
                forget(hwnd.0 as isize);
            }
            None
        }),
    );
}

/// The window is going: give its strip back and stop tracking it.
fn forget(raw: isize) {
    material::forget(raw);
    let removed = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let index = windows.iter().position(|window| window.hwnd == raw)?;
        Some(windows.remove(index))
    });
    if let Some(window) = &removed {
        trace::trace(|| format!("layer {} closed", window.layer.namespace));
    }
    if removed.is_some_and(|window| window.strip.is_some()) {
        appbar::remove(handle(raw));
    }
}

fn ensure_signals(cx: &mut App) -> async_channel::Sender<Signal> {
    if let Some(sender) = SIGNALS.with(|signals| signals.borrow().clone()) {
        return sender;
    }
    let (sender, receiver) = async_channel::unbounded::<Signal>();
    SIGNALS.with(|signals| *signals.borrow_mut() = Some(sender.clone()));
    cx.spawn(async move |_| {
        while let Ok(signal) = receiver.recv().await {
            // Repeats that queued up meanwhile fold into this one.
            let mut restarted = matches!(signal, Signal::ExplorerRestarted);
            while let Ok(more) = receiver.try_recv() {
                restarted |= matches!(more, Signal::ExplorerRestarted);
            }
            if restarted {
                // A new Explorer knows none of the old strips.
                WINDOWS.with(|windows| {
                    for window in windows.borrow_mut().iter_mut() {
                        window.strip = None;
                    }
                });
            }
            replace_all();
        }
    })
    .detach();
    sender
}

/// Lulo's background surfaces go back over Explorer's desktop whenever it
/// comes forward (Show Desktop, Win+D, a click through to it).
fn watch_desktop_foreground(cx: &mut App) {
    if BACKGROUND_WATCH.with(|watching| watching.replace(true)) {
        return;
    }
    let (sender, receiver) = async_channel::bounded::<rmac_compositor::Event>(16);
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = rmac_compositor_system::watch(sender).await {
                eprintln!("the desktop cannot follow Explorer's: {error}");
            }
        })
        .detach();
    cx.spawn(async move |_| {
        while receiver.recv().await.is_ok() {
            while receiver.try_recv().is_ok() {}
            let backgrounds = WINDOWS.with(|windows| {
                windows
                    .borrow()
                    .iter()
                    .filter(|window| {
                        matches!(window.layer.layer, Layer::Background | Layer::Bottom)
                    })
                    .map(|window| window.hwnd)
                    .collect::<Vec<_>>()
            });
            for raw in backgrounds {
                desktop_layer::place_above_desktop_layer(handle(raw));
            }
        }
    })
    .detach();
}

/// Place every surface again.
pub fn replace_all() {
    let windows = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .map(|window| window.hwnd)
            .collect::<Vec<_>>()
    });
    for raw in windows {
        place(raw);
    }
}

/// Open `target` with its default handler, as a double-click in Explorer
/// does (`ShellExecuteW`'s `open`).
pub fn shell_open(target: &std::path::Path) -> std::io::Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = HSTRING::from(target.as_os_str());
    // SAFETY: NUL-terminated strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            &file,
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute's documented success: a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "could not open {} (error {})",
            target.display(),
            result.0 as isize
        )))
    }
}

/// The signed-in user's display name ("Jacob Samas"), else the account
/// name.
pub fn account_display_name() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
    let mut buffer = vec![0u16; 256];
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer and its length; the size is updated in place.
    let read = unsafe { GetUserNameExW(NameDisplay, Some(PWSTR(buffer.as_mut_ptr())), &mut size) };
    let name = if read {
        String::from_utf16_lossy(&buffer[..size as usize])
    } else {
        std::env::var("USERNAME").unwrap_or_default()
    };
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Every AppBar this process holds, for the paths that must give the work
/// area back even if the process cannot finish (`lulo-session`).
pub fn appbar_windows() -> Vec<isize> {
    WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .filter(|window| window.strip.is_some())
            .map(|window| window.hwnd)
            .collect()
    })
}

/// Every shell surface by namespace, for traces and checks.
pub fn layer_windows() -> Vec<(isize, String)> {
    WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .map(|window| (window.hwnd, window.layer.namespace.clone()))
            .collect()
    })
}

/// Give every strip back to the work area now (turning Lulo off).
pub fn release_appbars() {
    let held = WINDOWS.with(|windows| {
        windows
            .borrow_mut()
            .iter_mut()
            .filter_map(|window| window.strip.take().map(|_| window.hwnd))
            .collect::<Vec<_>>()
    });
    for raw in held {
        appbar::remove(handle(raw));
    }
}

fn raise_overlays() {
    let overlays = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .filter(|window| window.layer.layer == Layer::Overlay && window.styled)
            .map(|window| window.hwnd)
            .collect::<Vec<_>>()
    });
    for raw in overlays {
        // SAFETY: raises a window this process owns within the topmost band.
        let _ = unsafe {
            SetWindowPos(
                handle(raw),
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }
}

/// Style (once), size and place one surface, and hold its strip.
/// `display` less the exclusive zones the other surfaces on it hold.
fn usable_area(raw: isize, display: u64, mut area: Bounds<Pixels>) -> Bounds<Pixels> {
    let zones: Vec<(appbar::Edge, Pixels)> = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .filter(|window| window.hwnd != raw && window.display == display)
            .filter_map(|window| {
                let zone = window
                    .layer
                    .exclusive_zone
                    .filter(|zone| zone.as_f32() > 0.0)?;
                Some((exclusive_edge(&window.layer)?, zone))
            })
            .collect()
    });
    for (edge, zone) in zones {
        match edge {
            appbar::Edge::Top => {
                area.origin.y += zone;
                area.size.height -= zone;
            }
            appbar::Edge::Bottom => area.size.height -= zone,
            appbar::Edge::Left => {
                area.origin.x += zone;
                area.size.width -= zone;
            }
            appbar::Edge::Right => area.size.width -= zone,
        }
    }
    area
}

fn place(raw: isize) {
    let Some((layer, display, requested, blurred, focus, styled, strip)) =
        WINDOWS.with(|windows| {
            windows
                .borrow()
                .iter()
                .find(|window| window.hwnd == raw)
                .map(|window| {
                    (
                        window.layer.clone(),
                        window.display,
                        window.requested,
                        window.blurred,
                        window.focus,
                        window.styled,
                        window.strip,
                    )
                })
        })
    else {
        return;
    };
    let hwnd = handle(raw);
    if !styled {
        style(hwnd, &layer, blurred);
        WINDOWS.with(|windows| {
            if let Some(window) = windows
                .borrow_mut()
                .iter_mut()
                .find(|window| window.hwnd == raw)
            {
                window.styled = true;
            }
        });
    }
    let (monitor, scale) = monitor_area(HMONITOR(display as isize as *mut core::ffi::c_void))
        .or_else(|| monitor_area(primary_monitor()))
        .unwrap_or((RECT::default(), 1.0));
    let display_bounds = Bounds {
        origin: point(
            px(monitor.left as f32 / scale),
            px(monitor.top as f32 / scale),
        ),
        size: size(
            px((monitor.right - monitor.left) as f32 / scale),
            px((monitor.bottom - monitor.top) as f32 / scale),
        ),
    };
    // As on wlr-layer-shell, a surface with no exclusive zone of its own
    // (0) keeps out of the others' zones: Spotlight's and Control Centre's
    // margins count from below the menu bar. A negative zone ignores them.
    let display_bounds = if layer.exclusive_zone.is_none_or(|zone| zone.as_f32() == 0.0) {
        usable_area(raw, display, display_bounds)
    } else {
        display_bounds
    };
    let logical = logical_bounds(&layer, requested, display_bounds);
    let physical = |value: Pixels| (value.as_f32() * scale).round() as i32;
    let left = monitor.left + physical(logical.origin.x - display_bounds.origin.x);
    let top = monitor.top + physical(logical.origin.y - display_bounds.origin.y);
    let rect = RECT {
        left,
        top,
        right: left + physical(logical.size.width),
        bottom: top + physical(logical.size.height),
    };
    match layer.layer {
        Layer::Background | Layer::Bottom => {
            {
                let _guard = desktop_layer::OwnPlacement::begin();
                // SAFETY: positions a window this process owns.
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                    )
                };
            }
            desktop_layer::place_above_desktop_layer(hwnd);
        }
        Layer::Top | Layer::Overlay => {
            // The first placement stacks the surface (newest on top, as
            // layer surfaces of one layer stack); placing it again (a
            // display change, a new size) keeps its place.
            let order = if styled { SWP_NOZORDER } else { Default::default() };
            // SAFETY: positions a window this process owns.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | order,
                )
            };
            if layer.layer == Layer::Top && !styled {
                raise_overlays();
            }
        }
    }
    if let (Some(zone), Some(edge)) = (
        layer.exclusive_zone.filter(|zone| zone.as_f32() > 0.0),
        exclusive_edge(&layer),
    ) {
        let current = match strip {
            Some(current) => current,
            None => {
                appbar::register(hwnd);
                RECT::default()
            }
        };
        let thickness = (zone.as_f32() * scale).round() as i32;
        let held = appbar::reserve(hwnd, edge, thickness, monitor, current);
        trace::trace(|| {
            format!(
                "strip {} at {},{},{},{}",
                layer.namespace, held.left, held.top, held.right, held.bottom
            )
        });
        if held != current {
            appbar::moved(hwnd);
        }
        WINDOWS.with(|windows| {
            if let Some(window) = windows
                .borrow_mut()
                .iter_mut()
                .find(|window| window.hwnd == raw)
            {
                window.strip = Some(held);
            }
        });
        if strip.is_none() {
            run_hooks(&STRIP_HOOKS, raw, &layer.namespace);
        }
    }
    trace::trace(|| {
        format!(
            "layer {} at {},{},{},{}",
            layer.namespace, rect.left, rect.top, rect.right, rect.bottom
        )
    });
    if !styled && (focus || layer.keyboard_interactivity == KeyboardInteractivity::Exclusive) {
        surface::take_foreground(hwnd);
    }
    run_hooks(&PLACED_HOOKS, raw, &layer.namespace);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(1366.0), px(768.0)),
        }
    }

    #[test]
    fn anchors_and_margins_place_surfaces_as_layer_shell_does() {
        let bar = LayerShellOptions {
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            ..Default::default()
        };
        let bounds = logical_bounds(&bar, size(px(1366.0), px(768.0)), display());
        assert_eq!(bounds.origin, point(px(0.0), px(0.0)));
        assert_eq!(bounds.size.width, px(1366.0));

        let shelf = LayerShellOptions {
            anchor: Anchor::BOTTOM,
            margin: Some((px(0.0), px(0.0), px(4.0), px(0.0))),
            ..Default::default()
        };
        let bounds = logical_bounds(&shelf, size(px(600.0), px(70.0)), display());
        assert_eq!(bounds.origin, point(px(383.0), px(694.0)));
        assert_eq!(bounds.size, size(px(600.0), px(70.0)));

        let menu = LayerShellOptions {
            anchor: Anchor::TOP | Anchor::LEFT,
            margin: Some((px(30.0), px(0.0), px(0.0), px(12.0))),
            ..Default::default()
        };
        let bounds = logical_bounds(&menu, size(px(200.0), px(300.0)), display());
        assert_eq!(bounds.origin, point(px(12.0), px(30.0)));
    }

    #[test]
    fn exclusive_zones_hold_the_anchored_edge() {
        let zone = |anchor| LayerShellOptions {
            anchor,
            exclusive_zone: Some(px(29.0)),
            ..Default::default()
        };
        assert_eq!(
            exclusive_edge(&zone(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT)),
            Some(appbar::Edge::Top)
        );
        assert_eq!(
            exclusive_edge(&zone(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT)),
            Some(appbar::Edge::Bottom)
        );
        assert_eq!(
            exclusive_edge(&zone(Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM)),
            Some(appbar::Edge::Left)
        );
        assert_eq!(
            exclusive_edge(&zone(
                Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT
            )),
            None
        );
    }
}
