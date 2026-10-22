//! Options ▸ Open at Login on Windows: the app's value in the user's own
//! `Run` key, named by its desktop entry, holding its program (what
//! Explorer, or Lulo as the shell, starts at sign-in). Lulo OS uses XDG
//! autostart instead (`rmac-login-items-linux`).

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The `Run` value for an app: quoted program, no arguments.
pub(crate) fn command(program: &Path) -> String {
    format!("\"{}\"", program.display())
}

/// Whether the app (`id`, its desktop entry's file name) opens at login.
pub(crate) fn enabled(id: &str) -> bool {
    let mut size = 0u32;
    // SAFETY: asks only for the size of a string value.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(id),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && size > 2
}

/// Open `program` at login under `id`, or stop (`None`).
pub(crate) fn set(id: &str, program: Option<&Path>) -> bool {
    let mut key = HKEY::default();
    // SAFETY: opens (or creates) the user's Run key for writing; closed
    // below.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    let name = HSTRING::from(id);
    let written = match program {
        Some(program) => {
            let data = command(program)
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>();
            // SAFETY: a NUL-terminated UTF-16 string value.
            unsafe { RegSetValueExW(key, &name, None, REG_SZ, Some(&data)) == ERROR_SUCCESS }
        }
        None => {
            // SAFETY: deletes one value of the key opened above.
            let status = unsafe { RegDeleteValueW(key, &name) };
            status == ERROR_SUCCESS || status.0 == 2
        }
    };
    // SAFETY: closes the key opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_value_is_the_quoted_program() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\App\app.exe")),
            r#""C:\Program Files\App\app.exe""#
        );
    }
}
