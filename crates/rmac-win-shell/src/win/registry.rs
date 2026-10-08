//! The Lulo layer's few per-user settings under `HKCU\Software\Lulo\Shell`,
//! and its opt-in sign-in entry under the user's `Run` key. Lulo never
//! writes anywhere else in the registry.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub const SHELL_KEY: &str = r"Software\Lulo\Shell";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Lulo";

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
    with_key(SHELL_KEY, |key| {
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
    let path = HSTRING::from(SHELL_KEY);
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

/// Whether Lulo starts when the user signs in.
pub fn starts_at_sign_in() -> bool {
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

/// Start `command` when the user signs in, or stop (`None`). Opt-in only:
/// Lulo's menu has the switch; nothing turns it on by itself.
pub fn set_starts_at_sign_in(command: Option<&str>) -> bool {
    match command {
        Some(command) => with_key(RUN_KEY, |key| {
            let data = command
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>();
            // SAFETY: a NUL-terminated UTF-16 string value.
            let status = unsafe {
                RegSetValueExW(key, &HSTRING::from(RUN_VALUE), None, REG_SZ, Some(&data))
            };
            status == ERROR_SUCCESS
        })
        .unwrap_or(false),
        None => {
            delete_value_in(RUN_KEY, RUN_VALUE);
            true
        }
    }
}
