//! The Lock Screen policy on Windows.
//!
//! Windows locks an idle session through the screen saver: when the screen
//! saver starts and "On resume, display logon screen" is set, waking it
//! shows the sign-in screen. Lulo's "Lock screen when inactive" is therefore
//! the screen saver's timeout, active and secure flags. "Put the computer to
//! sleep when inactive" is the active power scheme's sleep (standby) timeout,
//! written for both mains and battery so one choice holds on either, as the
//! Mac's single setting does. Changes made elsewhere arrive as
//! `WM_SETTINGCHANGE` and power-setting notifications on a hidden window;
//! nothing polls.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use async_channel::Sender;
use windows::core::{w, BOOL, GUID, PCWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_ACCESS_DENIED, ERROR_SUCCESS, HANDLE, HINSTANCE, HLOCAL, HWND, LPARAM,
    LRESULT, WIN32_ERROR, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    GetPwrCapabilities, GetSystemPowerStatus, PowerGetActiveScheme, PowerReadACValueIndex,
    PowerReadDCValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex, PowerWriteDCValueIndex,
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    SYSTEM_POWER_CAPABILITIES, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_ACTIVE_POWERSCHEME, GUID_SLEEP_SUBGROUP, GUID_STANDBY_TIMEOUT,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassW, SystemParametersInfoW, TranslateMessage,
    DEVICE_NOTIFY_WINDOW_HANDLE, MSG, PBT_POWERSETTINGCHANGE, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SPI_GETSCREENSAVEACTIVE, SPI_GETSCREENSAVESECURE, SPI_GETSCREENSAVETIMEOUT,
    SPI_SETSCREENSAVEACTIVE, SPI_SETSCREENSAVESECURE, SPI_SETSCREENSAVETIMEOUT,
    SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_POWERBROADCAST, WM_QUIT,
    WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use crate::lock::IdlePolicy;
use crate::lock_settings::{Error, Snapshot, SuspendCapability};

const DESKTOP_KEY: PCWSTR = w!("Control Panel\\Desktop");
const SCREEN_SAVER_VALUE: PCWSTR = w!("SCRNSAVE.EXE");
const WINDOW_CLASS: PCWSTR = w!("LuloLockSettingsWatcher");

/// Set once Windows refuses a power-scheme write, so later snapshots tell
/// Settings the sleep timer is not this user's to change.
static SUSPEND_WRITE_DENIED: AtomicBool = AtomicBool::new(false);

pub fn lock_workstation() -> windows::core::Result<()> {
    // SAFETY: no arguments; fails only outside an interactive desktop.
    unsafe { LockWorkStation() }
}

pub fn settings() -> Result<Snapshot, Error> {
    Ok(Snapshot {
        lock_after_seconds: read_lock_after()?,
        suspend_after_seconds: read_suspend_after()?,
        suspend_capability: suspend_capability(),
    })
}

pub fn set_lock_after(seconds: Option<u32>) -> Result<Snapshot, Error> {
    match seconds {
        Some(seconds) => {
            IdlePolicy {
                version: 1,
                lock_after_seconds: Some(seconds),
                suspend_after_seconds: None,
            }
            .validate()
            .map_err(|_| Error::Invalid)?;
            // With no screen saver chosen Windows never starts one, so the
            // secure flag would never lock. The blank screen saver is the
            // closest match to the Mac's plain lock.
            if !has_screen_saver() {
                choose_blank_screen_saver()?;
            }
            spi_set(SPI_SETSCREENSAVETIMEOUT, seconds)?;
            spi_set(SPI_SETSCREENSAVESECURE, 1)?;
            spi_set(SPI_SETSCREENSAVEACTIVE, 1)?;
        }
        None => spi_set(SPI_SETSCREENSAVEACTIVE, 0)?,
    }
    settings()
}

pub fn set_suspend_after(seconds: Option<u32>) -> Result<Snapshot, Error> {
    if let Some(seconds) = seconds {
        IdlePolicy {
            version: 1,
            lock_after_seconds: None,
            suspend_after_seconds: Some(seconds),
        }
        .validate()
        .map_err(|_| Error::Invalid)?;
        if suspend_capability() != SuspendCapability::Authorized {
            return Err(Error::AutomaticSuspendUnavailable);
        }
    }
    let value = seconds.unwrap_or(0);
    let scheme = ActiveScheme::get()?;
    // SAFETY: the scheme pointer lives until `scheme` drops; the GUID
    // constants are 'static.
    let (ac, dc) = unsafe {
        let ac = PowerWriteACValueIndex(
            None,
            scheme.0,
            Some(&GUID_SLEEP_SUBGROUP as *const GUID),
            Some(&GUID_STANDBY_TIMEOUT as *const GUID),
            value,
        );
        let dc = WIN32_ERROR(PowerWriteDCValueIndex(
            None,
            scheme.0,
            Some(&GUID_SLEEP_SUBGROUP as *const GUID),
            Some(&GUID_STANDBY_TIMEOUT as *const GUID),
            value,
        ));
        (ac, dc)
    };
    for result in [ac, dc] {
        if result == ERROR_ACCESS_DENIED {
            SUSPEND_WRITE_DENIED.store(true, Ordering::Relaxed);
            return Err(Error::AutomaticSuspendUnavailable);
        }
        if result != ERROR_SUCCESS {
            return Err(Error::Persistence);
        }
    }
    // Writing a value only stores it; re-activating the scheme applies it.
    // SAFETY: as above.
    if unsafe { PowerSetActiveScheme(None, Some(scheme.0 as *const GUID)) } != ERROR_SUCCESS {
        return Err(Error::Persistence);
    }
    drop(scheme);
    settings()
}

pub async fn watch(sender: Sender<Result<Snapshot, String>>) -> Result<(), Error> {
    use futures_util::FutureExt as _;

    let (changed, changes) = async_channel::bounded::<()>(1);
    let listener = match Listener::start(changed) {
        Ok(listener) => listener,
        Err(error) => {
            let _ = sender.send(Err(error.to_string())).await;
            return Err(error);
        }
    };
    let mut last = None;
    loop {
        let next = settings().map_err(|error| error.to_string());
        if last.as_ref() != Some(&next) {
            if sender.send(next.clone()).await.is_err() {
                break;
            }
            last = Some(next);
        }
        let change = changes.recv().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(change, closed);
        let stop = futures_util::select! {
            change = change => change.is_err(),
            _ = closed => true,
        };
        if stop {
            break;
        }
    }
    drop(listener);
    Ok(())
}

fn read_lock_after() -> Result<Option<u32>, Error> {
    Ok(lock_after(
        spi_get(SPI_GETSCREENSAVEACTIVE)? != 0,
        spi_get(SPI_GETSCREENSAVESECURE)? != 0,
        has_screen_saver(),
        spi_get(SPI_GETSCREENSAVETIMEOUT)?,
    ))
}

/// The idle lock is on only when a screen saver will actually start and
/// waking it asks for sign-in.
fn lock_after(active: bool, secure: bool, has_screen_saver: bool, timeout: u32) -> Option<u32> {
    (active && secure && has_screen_saver && timeout != 0).then_some(timeout)
}

fn spi_get(action: SYSTEM_PARAMETERS_INFO_ACTION) -> Result<u32, Error> {
    // Every action read here writes a BOOL or an int, both 32 bits.
    let mut value = BOOL(0);
    // SAFETY: `value` outlives the call and has the size the action writes.
    unsafe {
        SystemParametersInfoW(
            action,
            0,
            Some(&mut value as *mut BOOL as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|_| Error::Call)?;
    Ok(value.0 as u32)
}

fn spi_set(action: SYSTEM_PARAMETERS_INFO_ACTION, value: u32) -> Result<(), Error> {
    // SAFETY: these actions take their value in `uiParam` and no pointer.
    unsafe { SystemParametersInfoW(action, value, None, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) }
        .map_err(|_| Error::Persistence)
}

fn has_screen_saver() -> bool {
    let mut bytes = 0u32;
    // SAFETY: a size query; no data buffer.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            DESKTOP_KEY,
            SCREEN_SAVER_VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    // An empty string is one UTF-16 terminator.
    result == ERROR_SUCCESS && bytes > 2
}

fn choose_blank_screen_saver() -> Result<(), Error> {
    let mut directory = [0u16; 260];
    // SAFETY: the buffer's length is passed with it.
    let length = unsafe { GetSystemDirectoryW(Some(&mut directory)) } as usize;
    if length == 0 || length >= directory.len() {
        return Err(Error::Persistence);
    }
    let mut path: Vec<u16> = directory[..length].to_vec();
    path.extend("\\scrnsave.scr".encode_utf16());
    path.push(0);
    // SAFETY: `path` is NUL-terminated and its byte length is passed.
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            DESKTOP_KEY,
            SCREEN_SAVER_VALUE,
            REG_SZ.0,
            Some(path.as_ptr() as *const core::ffi::c_void),
            (path.len() * 2) as u32,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(Error::Persistence)
    }
}

fn read_suspend_after() -> Result<Option<u32>, Error> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter.
    let on_battery =
        unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 0;
    let scheme = ActiveScheme::get()?;
    let mut seconds = 0u32;
    // SAFETY: the scheme pointer lives until `scheme` drops; the GUID
    // constants are 'static; `seconds` is a plain out-parameter.
    let result = unsafe {
        let scheme = Some(scheme.0 as *const GUID);
        let subgroup = Some(&GUID_SLEEP_SUBGROUP as *const GUID);
        let setting = Some(&GUID_STANDBY_TIMEOUT as *const GUID);
        if on_battery {
            WIN32_ERROR(PowerReadDCValueIndex(
                None,
                scheme,
                subgroup,
                setting,
                &mut seconds,
            ))
        } else {
            PowerReadACValueIndex(None, scheme, subgroup, setting, &mut seconds)
        }
    };
    if result != ERROR_SUCCESS {
        return Err(Error::Call);
    }
    Ok((seconds != 0).then_some(seconds))
}

fn suspend_capability() -> SuspendCapability {
    if SUSPEND_WRITE_DENIED.load(Ordering::Relaxed) {
        return SuspendCapability::Denied;
    }
    let mut capabilities = SYSTEM_POWER_CAPABILITIES::default();
    // SAFETY: plain out-parameter.
    if !unsafe { GetPwrCapabilities(&mut capabilities) } {
        return SuspendCapability::Unavailable;
    }
    // S3 is classic sleep; AoAc is modern standby, which the same idle
    // timer enters on PCs without S3.
    if capabilities.SystemS3 || capabilities.AoAc {
        SuspendCapability::Authorized
    } else {
        SuspendCapability::Unavailable
    }
}

/// The active power scheme's GUID, which Windows allocates and the caller
/// frees with `LocalFree`.
struct ActiveScheme(*mut GUID);

impl ActiveScheme {
    fn get() -> Result<Self, Error> {
        let mut scheme: *mut GUID = std::ptr::null_mut();
        // SAFETY: plain out-parameter; freed in Drop.
        let result = unsafe { PowerGetActiveScheme(None, &mut scheme) };
        if result != ERROR_SUCCESS || scheme.is_null() {
            return Err(Error::Call);
        }
        Ok(Self(scheme))
    }
}

impl Drop for ActiveScheme {
    fn drop(&mut self) {
        // SAFETY: allocated by PowerGetActiveScheme and freed once.
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.cast()))) };
    }
}

thread_local! {
    /// The watcher thread's "changed" sender, for its window procedure.
    static CHANGED: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
}

/// A thread owning a hidden top-level window: broadcast `WM_SETTINGCHANGE`
/// reaches only top-level windows (not message-only ones), and the window
/// also receives the power-setting notifications, so registration, delivery
/// and unregistration all stay on one thread.
struct Listener {
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    fn start(changed: Sender<()>) -> Result<Self, Error> {
        let (ready, started) = std::sync::mpsc::sync_channel::<Result<u32, Error>>(1);
        let thread = std::thread::Builder::new()
            .name("lock-settings-watch".into())
            .spawn(move || run_window(changed, ready))
            .map_err(|_| Error::Subscribe)?;
        match started.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Subscribe)
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // SAFETY: posting to a thread that owns a message queue (its window
        // exists until the loop ends).
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_window(changed: Sender<()>, ready: std::sync::mpsc::SyncSender<Result<u32, Error>>) {
    CHANGED.with(|slot| *slot.borrow_mut() = Some(changed));
    // SAFETY: the class and window use a 'static name and a procedure that
    // lives for the program; the window is destroyed on this thread.
    let window = unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            let _ = ready.send(Err(Error::Subscribe));
            return;
        };
        let instance = HINSTANCE::from(module);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_procedure),
            hInstance: instance,
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };
        // A second watcher finds the class already registered, which is fine.
        let _ = RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            WINDOW_CLASS,
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    };
    let Ok(window) = window else {
        let _ = ready.send(Err(Error::Subscribe));
        return;
    };
    // The power source decides which of the two sleep timeouts applies; the
    // scheme and its standby timeout change the value itself.
    let registrations: Vec<HPOWERNOTIFY> = [
        GUID_ACDC_POWER_SOURCE,
        GUID_ACTIVE_POWERSCHEME,
        GUID_STANDBY_TIMEOUT,
    ]
    .iter()
    .filter_map(|setting| {
        // SAFETY: the window outlives the registration, which is removed
        // below before the window is destroyed.
        unsafe {
            RegisterPowerSettingNotification(
                HANDLE(window.0),
                setting as *const GUID,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        }
        .ok()
    })
    .collect();
    // SAFETY: plain query.
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));

    let mut message = MSG::default();
    // SAFETY: a standard message loop on the thread that owns the window;
    // it ends when the listener posts WM_QUIT (0) or on error (-1).
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        for registration in registrations {
            let _ = UnregisterPowerSettingNotification(registration);
        }
        let _ = DestroyWindow(window);
    }
    CHANGED.with(|slot| slot.borrow_mut().take());
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let relevant = match message {
        WM_SETTINGCHANGE => is_screen_saver_change(wparam.0 as u32),
        WM_POWERBROADCAST => wparam.0 as u32 == PBT_POWERSETTINGCHANGE,
        _ => false,
    };
    if relevant {
        CHANGED.with(|slot| {
            if let Some(changed) = slot.borrow().as_ref() {
                // A pending change already covers this one.
                let _ = changed.try_send(());
            }
        });
        if message == WM_POWERBROADCAST {
            return LRESULT(1);
        }
    }
    // SAFETY: forwarding the message this procedure was given.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn is_screen_saver_change(action: u32) -> bool {
    [
        SPI_SETSCREENSAVETIMEOUT,
        SPI_SETSCREENSAVEACTIVE,
        SPI_SETSCREENSAVESECURE,
    ]
    .iter()
    .any(|setting| setting.0 == action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_lock_needs_an_active_secure_screen_saver() {
        assert_eq!(lock_after(true, true, true, 300), Some(300));
        assert_eq!(lock_after(false, true, true, 300), None);
        assert_eq!(lock_after(true, false, true, 300), None);
        assert_eq!(lock_after(true, true, false, 300), None);
        assert_eq!(lock_after(true, true, true, 0), None);
    }

    #[test]
    fn only_screen_saver_actions_count_as_changes() {
        assert!(is_screen_saver_change(SPI_SETSCREENSAVETIMEOUT.0));
        assert!(is_screen_saver_change(SPI_SETSCREENSAVEACTIVE.0));
        assert!(is_screen_saver_change(SPI_SETSCREENSAVESECURE.0));
        assert!(!is_screen_saver_change(0));
        assert!(!is_screen_saver_change(SPI_GETSCREENSAVETIMEOUT.0));
    }
}
