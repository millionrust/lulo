//! Appearance on Windows: Lulo's light/dark choice and accent colour are
//! also Windows' own, as Lulo OS hands them to GTK and Qt
//! (`rmac_gtk_settings::sync_toolkit_appearance`). Windows reads them from
//! the per-user Personalize, DWM and Explorer Accent keys, and every open
//! window re-reads them on the `ImmersiveColorSet` setting change.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_BINARY, REG_DWORD};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};

const PERSONALIZE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
const DWM: PCWSTR = w!(r"Software\Microsoft\Windows\DWM");
const ACCENT: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent");

/// Make Windows' apps, taskbar and title bars follow `effective`.
pub(crate) fn sync(effective: &rmac_appearance::ResolvedAppearance) -> Result<(), String> {
    let light = u32::from(effective.color_scheme == rmac_appearance::ResolvedColorScheme::Light);
    set_dword(PERSONALIZE, w!("AppsUseLightTheme"), light)?;
    set_dword(PERSONALIZE, w!("SystemUsesLightTheme"), light)?;

    let (red, green, blue) = effective.accent_color.components();
    let rgb = [channel(red), channel(green), channel(blue)];
    // A colour Windows chose from the wallpaper would replace it again.
    set_dword(PERSONALIZE, w!("ColorPrevalence"), 0)?;
    set_dword(DWM, w!("AccentColor"), abgr(rgb))?;
    set_dword(DWM, w!("ColorizationColor"), 0xc400_0000 | argb(rgb))?;
    set_dword(DWM, w!("ColorizationAfterglow"), 0xc400_0000 | argb(rgb))?;
    set_dword(ACCENT, w!("AccentColorMenu"), abgr(rgb))?;
    set_dword(ACCENT, w!("StartColorMenu"), abgr(shade(rgb, -0.4)))?;
    set_binary(ACCENT, w!("AccentPalette"), &palette(rgb))?;

    // Tell every window, as Windows' own Settings does; a hung window is
    // skipped rather than waited for.
    // SAFETY: a broadcast with a static NUL-terminated string.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
    Ok(())
}

fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `0xAABBGGRR`, opaque: how DWM and Explorer store a colour.
fn abgr([red, green, blue]: [u8; 3]) -> u32 {
    0xff00_0000 | (u32::from(blue) << 16) | (u32::from(green) << 8) | u32::from(red)
}

fn argb([red, green, blue]: [u8; 3]) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

/// The colour moved toward white (`amount` > 0) or black (< 0).
fn shade(rgb: [u8; 3], amount: f64) -> [u8; 3] {
    rgb.map(|value| {
        let value = f64::from(value);
        let target = if amount >= 0.0 { 255.0 } else { 0.0 };
        (value + (target - value) * amount.abs()).round() as u8
    })
}

/// Explorer's eight-entry accent palette, lightest to darkest (four bytes
/// each: R, G, B, 0), with the accent itself in the middle as Windows
/// builds it.
fn palette(rgb: [u8; 3]) -> [u8; 32] {
    let steps = [0.6, 0.4, 0.2, 0.0, -0.2, -0.4, -0.6, -0.8];
    let mut bytes = [0u8; 32];
    for (index, amount) in steps.into_iter().enumerate() {
        let [red, green, blue] = shade(rgb, amount);
        bytes[index * 4..index * 4 + 4].copy_from_slice(&[red, green, blue, 0]);
    }
    bytes
}

fn set_dword(key: PCWSTR, name: PCWSTR, value: u32) -> Result<(), String> {
    // SAFETY: both names are static NUL-terminated strings; the data is a
    // live u32.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            REG_DWORD.0,
            Some(&value as *const u32 as *const core::ffi::c_void),
            4,
        )
    };
    status
        .ok()
        .map_err(|error| format!("could not update Windows' appearance: {error}"))
}

fn set_binary(key: PCWSTR, name: PCWSTR, value: &[u8]) -> Result<(), String> {
    // SAFETY: as `set_dword`, with `value.len()` bytes of data.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            REG_BINARY.0,
            Some(value.as_ptr() as *const core::ffi::c_void),
            value.len() as u32,
        )
    };
    status
        .ok()
        .map_err(|error| format!("could not update Windows' accent colour: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_use_windows_byte_orders() {
        assert_eq!(abgr([0x13, 0x72, 0xf9]), 0xfff9_7213);
        assert_eq!(argb([0x13, 0x72, 0xf9]), 0x0013_72f9);
        let palette = palette([0x13, 0x72, 0xf9]);
        assert_eq!(&palette[12..16], &[0x13, 0x72, 0xf9, 0]);
        assert_eq!(shade([100, 100, 100], 1.0), [255, 255, 255]);
        assert_eq!(shade([100, 100, 100], -1.0), [0, 0, 0]);
    }
}
