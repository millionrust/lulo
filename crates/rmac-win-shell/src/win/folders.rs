//! "Use Files for Folders" (ADR 0023 "Lulo mode", WIN-OS-46): an opt-in,
//! per-user switch in the Lulo menu that makes Lulo's Files the app that
//! opens folders everywhere, as the Finder opens them on the Mac. Off by
//! default; Lulo's own surfaces (the Dock, the desktop, Spotlight) open
//! folders in Files either way.
//!
//! On, it adds an "Open in Files" verb under
//! `HKCU\Software\Classes\Directory\shell` and `…\Drive\shell` and makes it
//! the default verb there, recording the default it replaced under
//! `HKCU\Software\Lulo\Shell`. Off removes the verb and puts the recorded
//! default back. Nothing outside the user's own hive is written, so it
//! needs no administrator and never touches other users. Uninstalling Lulo
//! turns it off (`lulo-session --uninstall`), so no folder ever opens with
//! an app that is gone.

use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

use super::registry;

/// The verb Lulo adds.
const VERB: &str = "LuloFiles";
/// The classes Lulo's verb is added to: folders and drives.
const CLASSES: [&str; 2] = [r"Software\Classes\Directory", r"Software\Classes\Drive"];
/// The registry value under `HKCU\Software\Lulo\Shell` recording that the
/// switch is on.
const ON_RECORD: &str = "FilesForFolders";

fn previous_record(class: &str) -> String {
    format!(
        "FilesForFolders.Previous.{}",
        class.rsplit('\\').next().unwrap_or(class)
    )
}

fn read_string(path: &str, name: &str) -> Option<String> {
    let mut size = 0u32;
    let path_w = HSTRING::from(path);
    let name_w = HSTRING::from(name);
    // SAFETY: asks for the size first, then reads into a buffer that size.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            &path_w,
            &name_w,
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
            &path_w,
            &name_w,
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

fn write_string(path: &str, name: &str, value: &str) -> bool {
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under the user's own hive, writes one
    // NUL-terminated string and closes the key.
    unsafe {
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let data = value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>();
        let status = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&data));
        let _ = RegCloseKey(key);
        status == ERROR_SUCCESS
    }
}

fn delete_default(path: &str) {
    let mut key = HKEY::default();
    // SAFETY: opens an existing key, deletes its default value, closes it.
    unsafe {
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ) == ERROR_SUCCESS
        {
            let _ = RegDeleteValueW(key, windows::core::PCWSTR::null());
            let _ = RegCloseKey(key);
        }
    }
}

/// Whether folders open in Files.
pub fn is_on() -> bool {
    registry::get_dword(ON_RECORD).is_some()
}

/// The command line the verb runs.
pub fn command(files_exe: &Path) -> String {
    format!("\"{}\" --path \"%1\"", files_exe.display())
}

/// Make Files the app that opens folders and drives, for this user.
pub fn turn_on(files_exe: &Path) -> bool {
    if is_on() {
        return true;
    }
    let command = command(files_exe);
    for class in CLASSES {
        let shell = format!(r"{class}\shell");
        let previous = read_string(&shell, "").unwrap_or_default();
        if !registry::set_string(&previous_record(class), &previous) {
            return false;
        }
        let verb = format!(r"{shell}\{VERB}");
        if !(write_string(&verb, "", "Open in Files")
            && write_string(&format!(r"{verb}\command"), "", &command)
            && write_string(&shell, "", VERB))
        {
            turn_off();
            return false;
        }
    }
    registry::set_dword(ON_RECORD, 1)
}

/// Give folders back to Explorer: remove the verb and put back the default
/// verb that was there before. Safe to repeat.
pub fn turn_off() {
    for class in CLASSES {
        let shell = format!(r"{class}\shell");
        // SAFETY: deletes the key Lulo added, under the user's own hive.
        unsafe {
            let _ = RegDeleteTreeW(
                HKEY_CURRENT_USER,
                &HSTRING::from(format!(r"{shell}\{VERB}")),
            );
        }
        let record = previous_record(class);
        match registry::get_string(&record) {
            Some(previous) if !previous.is_empty() => {
                write_string(&shell, "", &previous);
            }
            _ => {
                if read_string(&shell, "").as_deref() == Some(VERB) {
                    delete_default(&shell);
                }
            }
        }
        registry::delete_value(&record);
    }
    registry::delete_value(ON_RECORD);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verb_opens_the_folder_in_files() {
        assert_eq!(
            command(Path::new(r"C:\Lulo\rmac-files.exe")),
            r#""C:\Lulo\rmac-files.exe" --path "%1""#
        );
        assert_eq!(
            previous_record(r"Software\Classes\Directory"),
            "FilesForFolders.Previous.Directory"
        );
    }
}
