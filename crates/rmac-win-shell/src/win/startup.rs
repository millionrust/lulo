//! The user's startup apps when Lulo is their shell (ADR 0023, "Lulo as
//! the shell").
//!
//! Explorer starts them at sign-in: the `Run` and `RunOnce` keys and the
//! Startup folders, skipping what Task Manager's Startup apps page turned
//! off (`Explorer\StartupApproved`). Without Explorer nobody would, so
//! `lulo-session --as-shell` does the same, once, after the Lulo layer is
//! on screen. Lulo's own entry is never started twice.

use std::path::{Path, PathBuf};

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
    RRF_RT_REG_BINARY,
};
use windows::Win32::System::Threading::{
    CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonStartup, FOLDERID_Startup, ShellExecuteExW, SEE_MASK_FLAG_NO_UI,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::model::startup;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const RUN_32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

fn open_key(
    root: HKEY,
    path: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Option<HKEY> {
    let mut key = HKEY::default();
    // SAFETY: opens an existing key into `key`; closed by the caller.
    let status = unsafe { RegOpenKeyExW(root, &HSTRING::from(path), None, access, &mut key) };
    (status == ERROR_SUCCESS).then_some(key)
}

/// Every string value of a key: (name, command), `REG_EXPAND_SZ` expanded.
fn string_values(root: HKEY, path: &str) -> Vec<(String, String)> {
    let Some(key) = open_key(root, path, KEY_READ) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for index in 0.. {
        let mut name = vec![0u16; 16_384];
        let mut name_length = name.len() as u32;
        let mut data = vec![0u8; 32_768];
        let mut data_length = data.len() as u32;
        let mut kind = 0u32;
        // SAFETY: buffers and their lengths as the call expects.
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_length,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut data_length),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        if kind != REG_SZ.0 && kind != REG_EXPAND_SZ.0 {
            continue;
        }
        let name = String::from_utf16_lossy(&name[..name_length as usize]);
        let units: Vec<u16> = data[..data_length as usize]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        let mut command = String::from_utf16_lossy(&units);
        if kind == REG_EXPAND_SZ.0 {
            command = expand(&command);
        }
        values.push((name, command));
    }
    // SAFETY: closes the key opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    values
}

fn expand(text: &str) -> String {
    let source = HSTRING::from(text);
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: expands into a buffer of the length given.
    let length = unsafe { ExpandEnvironmentStringsW(&source, Some(&mut buffer)) } as usize;
    if length == 0 || length > buffer.len() {
        return text.to_owned();
    }
    String::from_utf16_lossy(&buffer[..length.saturating_sub(1)])
}

/// The `StartupApproved` bytes for `name` under `root\APPROVED\list`.
fn approval(root: HKEY, list: &str, name: &str) -> Option<Vec<u8>> {
    let path = HSTRING::from(format!(r"{APPROVED}\{list}"));
    let name = HSTRING::from(name);
    let mut data = vec![0u8; 64];
    let mut size = data.len() as u32;
    // SAFETY: reads a binary value into a buffer of the size given.
    let status = unsafe {
        RegGetValueW(
            root,
            &path,
            &name,
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then(|| {
        data.truncate(size as usize);
        data
    })
}

/// Whether `name` is turned on in both the user's and the machine's
/// `StartupApproved\list`.
fn approved(list: &str, name: &str) -> bool {
    startup::approved(approval(HKEY_CURRENT_USER, list, name).as_deref())
        && startup::approved(approval(HKEY_LOCAL_MACHINE, list, name).as_deref())
}

/// Start `command` as the `Run` key holds it (a command line).
fn run_command(command: &str) {
    let mut line: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: a writable, NUL-terminated command line, as CreateProcessW
    // requires; the handles it returns are closed at once.
    let started = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            PCWSTR::null(),
            &startup,
            &mut process,
        )
    };
    match started {
        // SAFETY: the handles CreateProcessW returned.
        Ok(()) => unsafe {
            let _ = CloseHandle(process.hThread);
            let _ = CloseHandle(process.hProcess);
        },
        Err(error) => eprintln!("lulo-session: could not start a startup app: {error}"),
    }
}

/// Open a Startup folder item (usually a shortcut) as Explorer would.
fn open_item(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the string it points at outlive the call.
    if unsafe { ShellExecuteExW(&mut info) }.is_err() {
        eprintln!("lulo-session: could not open a Startup folder item");
    }
}

fn startup_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    super::catalog::known_folder(id)
}

/// Start the user's startup apps, once. Blocking: run it on its own thread.
pub fn run_all() {
    // SAFETY: this thread's COM apartment, for ShellExecuteEx.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let mut started = 0usize;
    for (root, path, list) in [
        (HKEY_CURRENT_USER, RUN, "Run"),
        (HKEY_LOCAL_MACHINE, RUN, "Run"),
        (HKEY_LOCAL_MACHINE, RUN_32, "Run32"),
    ] {
        for (name, command) in string_values(root, path) {
            if startup::is_lulo(&command) || !approved(list, &name) {
                continue;
            }
            run_command(&command);
            started += 1;
        }
    }
    // RunOnce: each entry is removed as it starts (a `!` name only after).
    let once = string_values(HKEY_CURRENT_USER, RUN_ONCE);
    if let Some(key) = open_key(HKEY_CURRENT_USER, RUN_ONCE, KEY_SET_VALUE) {
        for (name, command) in once {
            let after = name.starts_with('!');
            if !after {
                // SAFETY: deletes one value of the key opened above.
                let _ = unsafe { RegDeleteValueW(key, &HSTRING::from(name.as_str())) };
            }
            run_command(&command);
            started += 1;
            if after {
                // SAFETY: as above.
                let _ = unsafe { RegDeleteValueW(key, &HSTRING::from(name.as_str())) };
            }
        }
        // SAFETY: closes the key opened above.
        unsafe {
            let _ = RegCloseKey(key);
        }
    }
    for folder in [&FOLDERID_Startup, &FOLDERID_CommonStartup] {
        let Some(folder) = startup_folder(folder) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("desktop.ini") || !approved("StartupFolder", &name) {
                continue;
            }
            open_item(&entry.path());
            started += 1;
        }
    }
    super::trace(|| format!("startup apps started: {started}"));
}
