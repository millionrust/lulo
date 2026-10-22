//! The volume OSD on Windows, as on Lulo OS and the Mac (KB-04): the
//! keyboard's volume keys step the default output by Lulo's step (or mute
//! it) and Lulo's Sound panel shows under the bar's right end for a moment,
//! in place of Windows' own flyout. A low-level keyboard hook on a thread of
//! its own takes only the three volume keys and compares one number for any
//! other key. The panel's window exists only while it shows.
//!
//! Brightness keys are the PC's firmware's on most laptops: Windows changes
//! the brightness and shows its own flyout, and they never reach a hook.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use gpui::{
    div, point, prelude::*, px, App, AsyncApp, Bounds, Context, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions,
};
use rmac_shell_layer::layer::*;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT,
    MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

static SENDER: OnceLock<async_channel::Sender<rmac_osd::Command>> = OnceLock::new();

/// The command a volume key asks for, if `key` is one.
fn command_for(key: u32) -> Option<rmac_osd::Command> {
    if key == u32::from(VK_VOLUME_UP.0) {
        Some(rmac_osd::Command::VolumeUp)
    } else if key == u32::from(VK_VOLUME_DOWN.0) {
        Some(rmac_osd::Command::VolumeDown)
    } else if key == u32::from(VK_VOLUME_MUTE.0) {
        Some(rmac_osd::Command::ToggleOutputMute)
    } else {
        None
    }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for HC_ACTION, `lparam` points at the event's
        // KBDLLHOOKSTRUCT for the duration of the call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if let Some(command) = command_for(info.vkCode) {
            // Presses (and their repeats) step; releases are taken too, so
            // Windows sees neither and shows no flyout of its own.
            if matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
                if let Some(sender) = SENDER.get() {
                    let _ = sender.try_send(command);
                }
            }
            return LRESULT(1);
        }
    }
    // SAFETY: passes the event on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Take the volume keys and show the panel for each; false when Windows
/// refused the hook (its own flyout then answers).
pub fn start(cx: &mut App) -> bool {
    let (sender, commands) = async_channel::bounded::<rmac_osd::Command>(16);
    if SENDER.set(sender).is_err() {
        return true;
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("lulo-volume-keys".into())
        .spawn(move || {
            // SAFETY: a low-level hook whose procedure is in this executable;
            // this thread pumps messages, as such a hook needs.
            let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok();
            // SAFETY: as above.
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(on_key),
                    module.map(|module| HINSTANCE(module.0)),
                    0,
                )
            };
            let _ = ready_tx.send(hook.is_ok());
            if hook.is_err() {
                return;
            }
            let mut message = MSG::default();
            // SAFETY: the standard message loop; `message` is a valid MSG.
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                // SAFETY: as above.
                unsafe { DispatchMessageW(&message) };
            }
        });
    if spawned.is_err() || !ready_rx.recv().unwrap_or(false) {
        return false;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(command) = commands.recv().await {
            let shown = blocking::unblock(move || rmac_osd::execute(command)).await;
            match shown {
                Ok(presentation) => {
                    let _ = cx.update(|cx| show(presentation, cx));
                }
                Err(error) => eprintln!("lulo-shell: the volume key did nothing: {error}"),
            }
        }
    })
    .detach();
    true
}

/// Where the OSD's glyphs are installed (`share`): the XDG data home's
/// `rmac/osd`.
fn glyph_path(file: &str) -> PathBuf {
    rmac_apps::windows_apps::data_home()
        .unwrap_or_default()
        .join("rmac")
        .join("osd")
        .join(file)
}

struct OsdPanel {
    presentation: rmac_osd::Presentation,
}

impl Render for OsdPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(rmac_osd::view::panel(
            &self.presentation,
            "system-osd",
            rmac_shell_ui::tokens::is_dark(),
            glyph_path,
        ))
    }
}

thread_local! {
    /// The panel's window while it shows, and the count of what it showed.
    static PANEL: RefCell<Option<(WindowHandle<OsdPanel>, u64)>> = const { RefCell::new(None) };
}

/// Show `presentation` in the panel (opening it if it is not up) and close
/// it once nothing new came for the Mac's 1.6 s.
fn show(presentation: rmac_osd::Presentation, cx: &mut App) {
    let current = PANEL.with(|panel| panel.borrow().clone());
    let generation = current.as_ref().map_or(1, |(_, generation)| generation + 1);
    let updated = current.is_some_and(|(handle, _)| {
        handle
            .update(cx, |panel, _, cx| {
                panel.presentation = presentation.clone();
                cx.notify();
            })
            .is_ok()
    });
    let handle = if updated {
        current.map(|(handle, _)| handle)
    } else {
        open(presentation, cx)
    };
    let Some(handle) = handle else {
        PANEL.with(|panel| *panel.borrow_mut() = None);
        return;
    };
    PANEL.with(|panel| *panel.borrow_mut() = Some((handle, generation)));
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor()
            .timer(rmac_osd::view::HIDE_DELAY)
            .await;
        let _ = cx.update(|cx| {
            let still = PANEL.with(|panel| {
                panel
                    .borrow()
                    .as_ref()
                    .is_some_and(|(_, shown)| *shown == generation)
            });
            if still {
                PANEL.with(|panel| *panel.borrow_mut() = None);
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });
    })
    .detach();
}

fn open(presentation: rmac_osd::Presentation, cx: &mut App) -> Option<WindowHandle<OsdPanel>> {
    let options = WindowOptions {
        titlebar: None,
        focus: false,
        show: true,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: Size::new(
                px(rmac_osd::view::SURFACE_WIDTH),
                px(rmac_osd::view::SURFACE_HEIGHT),
            ),
        })),
        app_id: Some("dev.rmac.Osd".to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let layer = LayerShellOptions {
        namespace: "rmac-osd".to_owned(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::RIGHT,
        margin: Some((
            px(rmac_shell_ui::tokens::menubar_height() + rmac_osd::view::GAP_BELOW_BAR),
            px(rmac_osd::view::RIGHT_MARGIN),
            px(0.0),
            px(0.0),
        )),
        keyboard_interactivity: KeyboardInteractivity::None,
        ..Default::default()
    };
    match rmac_shell_layer::open_layer_window(cx, options, layer, move |_, cx| {
        cx.new(|_| OsdPanel { presentation })
    }) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("lulo-shell: the volume panel did not open: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_volume_keys_are_taken() {
        assert_eq!(
            command_for(u32::from(VK_VOLUME_UP.0)),
            Some(rmac_osd::Command::VolumeUp)
        );
        assert_eq!(
            command_for(u32::from(VK_VOLUME_MUTE.0)),
            Some(rmac_osd::Command::ToggleOutputMute)
        );
        assert_eq!(command_for(0x41), None);
    }
}
