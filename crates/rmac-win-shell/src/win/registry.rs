//! The Lulo layer's few per-user settings under `HKCU\Software\Lulo\Shell`,
//! whether Lulo mode is on under `HKCU\Software\Lulo\Session`, and its
//! opt-in sign-in switch: the user's own `Winlogon\Shell` value, which makes
//! Lulo their shell instead of Explorer (an older Lulo used the `Run` key,
//! which is cleared). Lulo never writes anywhere else in the registry.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub const SHELL_KEY: &str = r"Software\Lulo\Shell";
/// Whether Lulo mode is on, kept apart from `SHELL_KEY`: an upgrade from an
/// installer older than op/win-upgrade-keep still removes that key with the
/// old product, and this must outlive the upgrade it decides about (see
/// `session.rs`, `--after-install`).
const SESSION_KEY: &str = r"Software\Lulo\Session";
const LULO_MODE: &str = "LuloMode";
const LAST_STARTED: &str = "LastStarted";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Lulo";
/// The user's own shell (Windows reads it before the machine's).
const WINLOGON_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon";
const SHELL_VALUE: &str = "Shell";

pub fn get_dword(name: &str) -> Option<u32> {
    get_user_dword(SHELL_KEY, name)
}

/// A DWORD value under `HKEY_CURRENT_USER\<path>`, read only (Windows'
/// own settings Lulo follows, such as transparency effects).
pub fn get_user_dword(path: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `value` and `size` are valid out-parameters for a DWORD.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

fn with_key<T>(path: &str, f: impl FnOnce(HKEY) -> T) -> Option<T> {
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under the user's own hive.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let result = f(key);
    // SAFETY: closes the key opened above.
    let _ = unsafe { RegCloseKey(key) };
    Some(result)
}

pub fn set_dword(name: &str, value: u32) -> bool {
    set_user_dword(SHELL_KEY, name, value)
}

/// Whether the user last left Lulo mode on (`Some(true)`), turned it off
/// themselves (`Some(false)`), or never ran Lulo here (`None`).
pub fn lulo_mode() -> Option<bool> {
    get_user_dword(SESSION_KEY, LULO_MODE).map(|value| value != 0)
}

/// Record that Lulo mode is on or off. Only the user's own switches turn
/// it off (Turn Off Lulo, Restore Windows Desktop); `lulo-session --stop`
/// from a script or the installer leaves it as it was.
pub fn set_lulo_mode(on: bool) -> bool {
    set_user_dword(SESSION_KEY, LULO_MODE, u32::from(on))
}

/// Record when the session last started (Unix seconds), for the upgrade
/// check (`scripts/windows/upgrade_smoke.py`) and for diagnosis.
pub fn record_session_start() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let _ = set_user_dword(
        SESSION_KEY,
        LAST_STARTED,
        u32::try_from(now).unwrap_or(u32::MAX),
    );
}

/// Forget Lulo mode (uninstall).
pub fn forget_lulo_mode() {
    delete_value_in(SESSION_KEY, LULO_MODE);
    delete_value_in(SESSION_KEY, LAST_STARTED);
}

/// Forget the Lulo layer's settings (`SHELL_KEY` and everything under it).
/// Uninstall only: the installer used to remove the key itself, which also
/// happened while an upgrade removed the old product, so every upgrade
/// lost them.
pub fn forget_shell_settings() {
    // SAFETY: deletes one subtree of the user's own hive; a missing key is
    // nothing to delete.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(SHELL_KEY)) };
}

fn set_user_dword(path: &str, name: &str, value: u32) -> bool {
    with_key(path, |key| {
        // SAFETY: four bytes of DWORD data.
        let status = unsafe {
            RegSetValueExW(
                key,
                &HSTRING::from(name),
                None,
                REG_DWORD,
                Some(&value.to_le_bytes()),
            )
        };
        status == ERROR_SUCCESS
    })
    .unwrap_or(false)
}

/// A string value under `HKCU\Software\Lulo\Shell`.
pub fn get_string(name: &str) -> Option<String> {
    get_user_string(SHELL_KEY, name)
}

/// A string value under `HKEY_CURRENT_USER\<path>`.
fn get_user_string(path: &str, name: &str) -> Option<String> {
    let path = HSTRING::from(path);
    let name = HSTRING::from(name);
    let mut size = 0u32;
    // SAFETY: asks for the size first, then reads into a buffer that size.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            &path,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buffer.len() * 2) as u32;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            &path,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        Some(super::from_wide(&buffer))
    }
}

/// Write a string value under `HKCU\Software\Lulo\Shell`.
pub fn set_string(name: &str, value: &str) -> bool {
    with_key(SHELL_KEY, |key| {
        let data = value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>();
        // SAFETY: a NUL-terminated UTF-16 string value.
        let status =
            unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&data)) };
        status == ERROR_SUCCESS
    })
    .unwrap_or(false)
}

fn delete_value_in(path: &str, name: &str) {
    let mut key = HKEY::default();
    // SAFETY: opens an existing key; a missing key means nothing to delete.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return;
    }
    // SAFETY: deletes one value of the key opened above, then closes it.
    unsafe {
        let _ = RegDeleteValueW(key, &HSTRING::from(name));
        let _ = RegCloseKey(key);
    }
}

pub fn delete_value(name: &str) {
    delete_value_in(SHELL_KEY, name);
}

/// The user's own shell, when they have one (`Winlogon\Shell`).
pub fn user_shell() -> Option<String> {
    get_user_string(WINLOGON_KEY, SHELL_VALUE)
}

/// Whether Lulo is the user's shell at sign-in.
pub fn starts_at_sign_in() -> bool {
    user_shell().is_some_and(|shell| crate::model::startup::is_lulo_shell(&shell))
        || runs_at_sign_in()
}

/// An older Lulo's `Run` entry.
fn runs_at_sign_in() -> bool {
    let mut size = 0u32;
    // SAFETY: asks only for the size of a string value.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(RUN_VALUE),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && size > 2
}

/// Make Lulo (`session`, `lulo-session.exe`) the user's shell at sign-in,
/// instead of Explorer, or give the sign-in back to Explorer (`None`).
/// Opt-in only: Lulo's menu has the switch; nothing turns it on by itself.
/// A shell value that is not Lulo's is never removed.
pub fn set_starts_at_sign_in(session: Option<&std::path::Path>) -> bool {
    // An older Lulo's Run entry is replaced either way.
    remove_run_entry();
    match session {
        Some(session) => {
            let command = crate::model::startup::shell_command(&session.display().to_string());
            with_key(WINLOGON_KEY, |key| {
                let data = command
                    .encode_utf16()
                    .chain(Some(0))
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<u8>>();
                // SAFETY: a NUL-terminated UTF-16 string value.
                let status = unsafe {
                    RegSetValueExW(key, &HSTRING::from(SHELL_VALUE), None, REG_SZ, Some(&data))
                };
                status == ERROR_SUCCESS
            })
            .unwrap_or(false)
        }
        None => {
            if user_shell().is_some_and(|shell| crate::model::startup::is_lulo_shell(&shell)) {
                delete_value_in(WINLOGON_KEY, SHELL_VALUE);
            }
            true
        }
    }
}

/// Remove an older Lulo's `Run` entry.
fn remove_run_entry() {
    delete_value_in(RUN_KEY, RUN_VALUE);
}
