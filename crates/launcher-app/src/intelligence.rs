//! What a "Lulo can do this" row does when it is picked (ADR 0024 §8's
//! action table), through the services Lulo already uses for each job:
//!
//! - appearance: the rmac theme store, as System Settings ▸ Appearance;
//! - volume, Wi-Fi, Bluetooth, Do Not Disturb: Control Centre's typed
//!   operations (`rmac-quick-settings-system`);
//! - brightness: the display OSD's logind backlight call (`rmac-osd`);
//! - timer: Clock's locked store and its systemd ring timer;
//! - open an app and search files are ordinary launcher actions and never
//!   reach this module.
//!
//! Every function here blocks (session-bus calls, a settings write, a
//! `systemctl --user` call): callers run them on the blocking pool.

use rmac_intelligence::{
    AppearanceMode, BrightnessChange, Intent, Level, VolumeChange, STEP_PERCENT,
};
use rmac_launcher::{Action, Category, SearchResult};
use rmac_launcher_providers::Provider as _;
use rmac_quick_settings_system::Backend as _;

/// Carry out `intent`. Errors are short and safe to show.
pub(crate) fn perform(intent: &Intent) -> Result<(), String> {
    let system = rmac_quick_settings_system::SystemBackend;
    match intent {
        Intent::Appearance { mode } => set_appearance(*mode),
        Intent::Volume(Level::Percent(level)) => {
            system.set_output_volume(*level)?;
            system.set_output_muted(false)
        }
        Intent::Volume(Level::Change(change)) => match change {
            VolumeChange::Mute => system.set_output_muted(true),
            VolumeChange::Unmute => system.set_output_muted(false),
            VolumeChange::Up | VolumeChange::Down => {
                let current = system.audio()?.output.volume;
                let next = step(current, *change == VolumeChange::Up);
                system.set_output_volume(next)?;
                system.set_output_muted(false)
            }
        },
        Intent::Brightness(level) => {
            let target = match level {
                Level::Percent(level) => *level,
                Level::Change(change) => {
                    let current = rmac_osd::brightness().map_err(|error| error.to_string())?;
                    step(current, *change == BrightnessChange::Up)
                }
            };
            // A backlight at 0 % is a black screen; keep it readable.
            rmac_osd::set_brightness(target.max(1))
                .map(drop)
                .map_err(|error| error.to_string())
        }
        Intent::Wifi { on } => system.set_wifi_enabled(*on),
        Intent::Bluetooth { on } => system.set_bluetooth_powered(*on),
        Intent::DoNotDisturb { on } => system.set_focus_enabled(*on),
        Intent::Timer { .. } => start_timer(intent.timer_seconds().unwrap_or(0)),
        Intent::OpenApp { .. } | Intent::SearchFiles { .. } | Intent::None => {
            Err("that request is not a system change".into())
        }
    }
}

fn step(current: u8, up: bool) -> u8 {
    if up {
        current.saturating_add(STEP_PERCENT).min(100)
    } else {
        current.saturating_sub(STEP_PERCENT)
    }
}

/// Save Light or Dark the way System Settings ▸ Appearance does, then tell
/// third-party toolkits.
fn set_appearance(mode: AppearanceMode) -> Result<(), String> {
    let host = async_io::block_on(rmac_appearance_portal::snapshot()).unwrap_or_else(|_| {
        rmac_appearance::Snapshot::unavailable(
            "The desktop Settings portal is temporarily unavailable.",
        )
    });
    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the appearance preferences are unavailable".to_owned())?;
    let mut preferences = store
        .load(&host)
        .map_err(|_| "the appearance preferences could not be read".to_owned())?
        .preferences;
    preferences.color_scheme = match mode {
        AppearanceMode::Dark => rmac_theme::SchemePreference::Dark,
        AppearanceMode::Light => rmac_theme::SchemePreference::Light,
    };
    store
        .save(&preferences, &host)
        .map_err(|_| "the appearance preference could not be saved".to_owned())?;
    #[cfg(target_os = "linux")]
    if let Err(error) = rmac_gtk_settings::sync_toolkit_appearance(&preferences) {
        eprintln!("rmac-launcher: {error}");
    }
    Ok(())
}

/// Start a Clock timer that rings even with Clock closed, as Clock's own
/// Timers tab does.
fn start_timer(seconds: u64) -> Result<(), String> {
    if seconds == 0 {
        return Err("the timer has no length".into());
    }
    let now = rmac_clock::now_millis();
    let (state, ()) = rmac_clock::store::update(|state| {
        let id = state.allocate_id();
        rmac_clock::changes::Change::StartTimer {
            id,
            duration: seconds * 1000,
            now,
        }
        .apply(state);
    })
    .map_err(|error| format!("the timer could not be saved: {error}"))?;
    schedule(&state, now)
}

/// Clock's executable: beside Spotlight in a development install, else the
/// packaged app in `/usr/bin` (Spotlight itself lives in `/usr/libexec/rmac`).
#[cfg(target_os = "linux")]
fn clock_executable() -> Result<std::path::PathBuf, String> {
    let sibling = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("rmac-clock");
    [sibling, std::path::PathBuf::from("/usr/bin/rmac-clock")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "Clock is not installed".to_owned())
}

#[cfg(target_os = "linux")]
fn schedule(state: &rmac_clock::store::State, now: u64) -> Result<(), String> {
    let clock = clock_executable()?;
    let zone = rmac_clock::tz::local_zone();
    rmac_clock::schedule::apply_with(state, now, &|utc| zone.offset_at(utc), &clock)
        .map_err(|error| format!("the timer may not ring: {error}"))
}

#[cfg(not(target_os = "linux"))]
fn schedule(state: &rmac_clock::store::State, now: u64) -> Result<(), String> {
    let zone = rmac_clock::tz::local_zone();
    rmac_clock::schedule::apply(state, now, &|utc| zone.offset_at(utc))
        .map_err(|error| format!("the timer may not ring: {error}"))
}

/// The installed app an "open …" request names: an exact name first, then
/// a name that starts with it, then any match. `None` when nothing
/// installed matches, so no row is shown for an app that is not there.
pub(crate) fn resolve_app(
    applications: &rmac_launcher_providers::ApplicationProvider,
    name: &str,
) -> Option<SearchResult> {
    let results = applications
        .search(name, &rmac_launcher::Cancellation::default())
        .ok()?;
    let wanted = name.trim().to_lowercase();
    let rank = |result: &SearchResult| {
        let title = result.title.to_lowercase();
        if title == wanted {
            0
        } else if title.starts_with(&wanted) {
            1
        } else if title.split(' ').any(|word| word == wanted) {
            2
        } else {
            3
        }
    };
    results
        .into_iter()
        .filter(|result| {
            result.category == Category::Applications
                && matches!(result.primary, Action::LaunchApplication { .. })
        })
        .min_by_key(rank)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stay_in_range() {
        assert_eq!(step(95, true), 100);
        assert_eq!(step(40, true), 50);
        assert_eq!(step(5, false), 0);
        assert_eq!(step(40, false), 30);
    }

    #[test]
    fn launcher_actions_never_reach_the_system_executor() {
        assert!(perform(&Intent::None).is_err());
        assert!(perform(&Intent::OpenApp {
            app: "Notes".into()
        })
        .is_err());
    }
}
