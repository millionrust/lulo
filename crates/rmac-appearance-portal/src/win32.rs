//! Windows' own light or dark app mode (Settings ▸ Personalisation ▸
//! Colours ▸ "Choose your app mode"), which Lulo's Automatic appearance
//! follows there, read from
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`.
//!
//! The watch is one thread parked in `RegNotifyChangeKeyValue` on that key:
//! it wakes only when Windows writes it, never on a timer.

use async_channel::Sender;
use rmac_appearance::{Capabilities, ColorScheme, Event, Snapshot};
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD,
};

const PERSONALIZE: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

/// `AppsUseLightTheme`: 1 for light, 0 for dark; `None` when Windows has
/// not written it (a fresh profile uses light).
fn apps_use_light_theme() -> Option<bool> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `value` is a u32 and `size` says so.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    Some(value != 0)
}

pub(crate) fn current() -> Snapshot {
    let light = apps_use_light_theme().unwrap_or(true);
    snapshot_for(light)
}

fn snapshot_for(light: bool) -> Snapshot {
    Snapshot {
        available: true,
        color_scheme: if light {
            ColorScheme::PreferLight
        } else {
            ColorScheme::PreferDark
        },
        capabilities: Capabilities {
            color_scheme: true,
            ..Capabilities::default()
        },
        ..Snapshot::default()
    }
}

/// Send a fresh snapshot after each change to the app mode, until `events`
/// closes.
pub(crate) fn watch_changes(events: Sender<Event>) {
    let spawned = std::thread::Builder::new()
        .name("lulo app mode watch".into())
        .spawn(move || {
            let mut key = HKEY::default();
            // SAFETY: `key` receives an open handle, closed below.
            if unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PERSONALIZE,
                    None,
                    KEY_NOTIFY | KEY_READ,
                    &mut key,
                )
            }
            .is_err()
            {
                return;
            }
            let mut last = apps_use_light_theme();
            loop {
                // SAFETY: a synchronous wait on the open key.
                let waited = unsafe {
                    RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false)
                };
                if waited.is_err() {
                    break;
                }
                let now = apps_use_light_theme();
                if now == last {
                    continue;
                }
                last = now;
                if events
                    .send_blocking(Event::Snapshot(snapshot_for(now.unwrap_or(true))))
                    .is_err()
                {
                    break;
                }
            }
            // SAFETY: the key opened above.
            let _ = unsafe { RegCloseKey(key) };
        });
    if let Err(error) = spawned {
        eprintln!("rmac-appearance-portal: could not watch the Windows app mode: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_mode_becomes_a_colour_scheme_preference() {
        assert_eq!(snapshot_for(true).color_scheme, ColorScheme::PreferLight);
        assert_eq!(snapshot_for(false).color_scheme, ColorScheme::PreferDark);
        assert!(snapshot_for(false).available);
        assert!(snapshot_for(false).capabilities.color_scheme);
    }

    #[test]
    fn the_current_mode_reads_without_failing() {
        assert!(current().available);
    }
}
