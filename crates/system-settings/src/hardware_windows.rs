//! The hardware scan on Windows: the same [`Capabilities`] the Linux sysfs
//! scan fills, from Lulo's hardware profile (`rmac-hardware`, ADR 0027),
//! which finds devices by their presence in Windows' device tree. A
//! switched-off Bluetooth radio or Wi-Fi adapter is still present, so its
//! pane stays (with its switch off), while a PC without a battery,
//! Bluetooth adapter, Wi-Fi adapter, touchpad, mouse or backlight gets no
//! pane or control for it. Called on a blocking-pool thread, never the UI
//! thread.

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, MSG, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY,
    WM_DEVICECHANGE, WM_DISPLAYCHANGE, WNDCLASSW, WS_OVERLAPPED,
};

use super::Capabilities;

pub(crate) fn current() -> Capabilities {
    // Detected afresh: `current` runs at start-up and after Windows reports
    // a device or display change.
    capabilities(&rmac_hardware::detect())
}

fn capabilities(profile: &rmac_hardware::HardwareProfile) -> Capabilities {
    let adaptations = profile.adaptations();
    Capabilities {
        has_battery: adaptations.battery,
        has_wifi: adaptations.wifi,
        has_bluetooth: adaptations.bluetooth,
        has_backlight: adaptations.display_brightness,
        has_keyboard_backlight: adaptations.keyboard_brightness,
        has_touchpad: profile.touchpad || profile.precision_touchpad,
        has_pointing_stick: profile.pointing_stick,
        has_external_mouse: profile.mouse,
        has_touchscreen: profile.touchscreen,
        has_pen: profile.pen,
        has_fingerprint: profile.fingerprint.present,
        fprintd_supported: false,
        has_accelerometer: profile.accelerometer,
        ..Capabilities::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switched_off_radio_keeps_its_pane() {
        // rmac-hardware reports the adapter by presence; the radio's on/off
        // state is the Bluetooth pane's own switch.
        let profile = rmac_hardware::HardwareProfile {
            bluetooth: true,
            ..Default::default()
        };
        let capabilities = capabilities(&profile);
        assert!(capabilities.has_bluetooth);
        assert!(capabilities.allows_pane("Bluetooth"));
        assert!(!capabilities.allows_pane("Battery"));
    }
}

/// Wait for Windows to say devices or displays changed: it broadcasts
/// `WM_DEVICECHANGE` (`DBT_DEVNODES_CHANGED`) and `WM_DISPLAYCHANGE` to
/// top-level windows, so a hidden one on its own thread listens. Nothing
/// polls; the window closes when Settings drops the receiver.
pub(crate) async fn watch(sender: async_channel::Sender<()>) -> std::io::Result<()> {
    let (ready_tx, ready_rx) = async_channel::bounded::<Option<isize>>(1);
    let events = sender.clone();
    let thread = std::thread::Builder::new()
        .name("lulo-hardware-watch".into())
        .spawn(move || listen(events, ready_tx))?;
    let window = ready_rx.recv().await.ok().flatten();
    if let Some(window) = window {
        sender.closed().await;
        // SAFETY: posting to the window this module created; it may already
        // be gone, which the call reports and nothing else depends on.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(window as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
    let _ = blocking::unblock(move || thread.join()).await;
    if window.is_none() {
        return Err(std::io::Error::other("could not watch Windows' devices"));
    }
    Ok(())
}

thread_local! {
    static EVENTS: std::cell::RefCell<Option<async_channel::Sender<()>>> =
        const { std::cell::RefCell::new(None) };
}

fn listen(events: async_channel::Sender<()>, ready: async_channel::Sender<Option<isize>>) {
    EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
    // SAFETY: a class and window owned by this thread, with a static name
    // and procedure; the message loop below runs until WM_QUIT.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(|module| module.into());
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.unwrap_or_default(),
            lpszClassName: w!("LuloSettingsHardwareWatch"),
            ..Default::default()
        };
        // Registering twice (a second watch) fails harmlessly: the class
        // already exists with the same procedure.
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("LuloSettingsHardwareWatch"),
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            None,
        );
        let Ok(window) = window else {
            let _ = ready.send_blocking(None);
            return;
        };
        let _ = ready.send_blocking(Some(window.0 as isize));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DEVICECHANGE | WM_DISPLAYCHANGE => {
            EVENTS.with(|slot| {
                if let Some(events) = slot.borrow().as_ref() {
                    let _ = events.try_send(());
                }
            });
            LRESULT(1)
        }
        WM_CLOSE => {
            // SAFETY: this thread's own window.
            let _ = unsafe { DestroyWindow(window) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the default handling for everything else.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
