//! Night Shift (Settings ▸ Displays ▸ Night Shift…): warmer colours at
//! night, through the compositor's gamma control.
//!
//! The policy is a small JSON file. `rmac-night-shift.service` runs
//! `rmac-night-shift --policy FILE`, which turns it into `wlsunset`
//! arguments and supervises that one process; with nothing to do it exits.
//! Settings saves the policy and restarts the service. Nothing polls: a
//! "Turn on until tomorrow" deadline is a single timer.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The daytime temperature, as macOS leaves colours alone by day.
pub const DAY_KELVIN: u32 = 6500;
/// The warmest night temperature (the slider's More Warm end).
pub const WARMEST_KELVIN: u32 = 2700;
/// The least warm night temperature (Less Warm).
pub const LEAST_WARM_KELVIN: u32 = 5500;
const MAX_POLICY_BYTES: usize = 4096;
#[cfg(target_os = "linux")]
const SERVICE: &str = "rmac-night-shift.service";

/// The Mac's Schedule pop-up. Sunset to Sunrise needs a location, which
/// Lulo OS does not have, so it is not offered.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Schedule {
    #[default]
    Off,
    /// Night from `from` until `to`, minutes after local midnight.
    Custom { from: u16, to: u16 },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Policy {
    pub schedule: Schedule,
    /// "Turn on until tomorrow": Unix seconds when it ends.
    pub manual_until: Option<u64>,
    /// Colour temperature, 0 (Less Warm) to 100 (More Warm).
    pub warmth: u8,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            schedule: Schedule::Off,
            manual_until: None,
            warmth: 50,
        }
    }
}

impl Policy {
    pub fn night_kelvin(&self) -> u32 {
        let warmth = u32::from(self.warmth.min(100));
        LEAST_WARM_KELVIN - (LEAST_WARM_KELVIN - WARMEST_KELVIN) * warmth / 100
    }

    /// Whether "Turn on until tomorrow" is in effect at `now`.
    pub fn manual_active(&self, now: u64) -> bool {
        self.manual_until.is_some_and(|until| now < until)
    }
}

/// `HH:MM` for minutes after midnight.
pub fn clock(minutes: u16) -> String {
    let minutes = minutes % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The `wlsunset` arguments for `policy` at local minute `now_minute` and
/// Unix time `now`, or None when Night Shift is off. A manual "until
/// tomorrow" without a schedule runs night from now until 07:00.
pub fn wlsunset_arguments(policy: &Policy, now: u64, now_minute: u16) -> Option<Vec<String>> {
    let (from, to) = match policy.schedule {
        Schedule::Custom { from, to } if from != to => {
            if policy.manual_active(now) {
                (now_minute, to)
            } else {
                (from, to)
            }
        }
        _ if policy.manual_active(now) => (now_minute, 7 * 60),
        _ => return None,
    };
    Some(vec![
        "-T".to_owned(),
        DAY_KELVIN.to_string(),
        "-t".to_owned(),
        policy.night_kelvin().to_string(),
        "-s".to_owned(),
        clock(from),
        "-S".to_owned(),
        clock(to),
        // Change at once at the edges, as Night Shift does.
        "-d".to_owned(),
        "60".to_owned(),
    ])
}

/// `$XDG_CONFIG_HOME/rmac/night-shift.json`.
pub fn policy_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .map(|base| base.join("rmac").join("night-shift.json"))
}

pub fn read_policy(path: &Path) -> Policy {
    rmac_storage::read_bounded_no_follow(path, MAX_POLICY_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn write_policy(path: &Path, policy: &Policy) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(policy).map_err(io::Error::other)?;
    bytes.push(b'\n');
    rmac_storage::atomic_write_private(path, &bytes)
}

/// Save the policy and restart the service so it takes effect now.
#[cfg(target_os = "linux")]
pub fn apply(policy: &Policy) -> io::Result<()> {
    let path = policy_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no configuration directory"))?;
    write_policy(&path, policy)?;
    let status = std::process::Command::new("/usr/bin/systemctl")
        .args(["--user", "restart", SERVICE])
        .stdin(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("the Night Shift service did not start"))
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_policy: &Policy) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Night Shift is a Lulo OS feature",
    ))
}

/// Whether `wlsunset` is installed (a recommended package).
pub fn available() -> bool {
    Path::new(WLSUNSET).exists()
}

const WLSUNSET: &str = "/usr/bin/wlsunset";

/// Run `wlsunset` for the policy until it exits, or until a manual "until
/// tomorrow" without a schedule ends. Returns at once when Night Shift is
/// off.
#[cfg(target_os = "linux")]
pub fn supervise(policy_path: &Path) -> io::Result<()> {
    let policy = read_policy(policy_path);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let Some(arguments) = wlsunset_arguments(&policy, now, local_minute(now)) else {
        return Ok(());
    };
    let mut child = std::process::Command::new(WLSUNSET)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .spawn()?;
    let deadline = (policy.schedule == Schedule::Off)
        .then_some(policy.manual_until)
        .flatten();
    match deadline {
        Some(until) => {
            // One sleep to the deadline, then stop; the child exiting early
            // is reported by the service manager's restart policy.
            std::thread::sleep(std::time::Duration::from_secs(until.saturating_sub(now)));
            let _ = child.kill();
            let _ = child.wait();
            Ok(())
        }
        None => {
            let status = child.wait()?;
            if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("wlsunset stopped"))
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn supervise(_policy_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Night Shift is a Lulo OS feature",
    ))
}

#[cfg(target_os = "linux")]
fn local_minute(now: u64) -> u16 {
    // SAFETY: localtime_r writes only into the provided tm.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = now as libc::time_t;
    let result = unsafe { libc::localtime_r(&time, &mut tm) };
    if result.is_null() {
        return 0;
    }
    (tm.tm_hour * 60 + tm.tm_min).clamp(0, 24 * 60 - 1) as u16
}

#[cfg(test)]
mod night_shift_tests {
    use super::*;

    #[test]
    fn off_runs_nothing_and_custom_runs_the_window() {
        let policy = Policy::default();
        assert_eq!(wlsunset_arguments(&policy, 0, 0), None);
        let custom = Policy {
            schedule: Schedule::Custom {
                from: 22 * 60,
                to: 7 * 60,
            },
            ..Policy::default()
        };
        let arguments = wlsunset_arguments(&custom, 0, 600).unwrap();
        assert!(arguments.windows(2).any(|pair| pair == ["-s", "22:00"]));
        assert!(arguments.windows(2).any(|pair| pair == ["-S", "07:00"]));
    }

    #[test]
    fn until_tomorrow_starts_now() {
        let policy = Policy {
            manual_until: Some(1000),
            ..Policy::default()
        };
        let arguments = wlsunset_arguments(&policy, 10, 15 * 60 + 5).unwrap();
        assert!(arguments.windows(2).any(|pair| pair == ["-s", "15:05"]));
        assert_eq!(wlsunset_arguments(&policy, 2000, 0), None);
    }

    #[test]
    fn warmth_maps_to_kelvin_and_round_trips() {
        let mut policy = Policy::default();
        policy.warmth = 0;
        assert_eq!(policy.night_kelvin(), LEAST_WARM_KELVIN);
        policy.warmth = 100;
        assert_eq!(policy.night_kelvin(), WARMEST_KELVIN);
        let text = serde_json::to_string(&policy).unwrap();
        assert_eq!(serde_json::from_str::<Policy>(&text).unwrap(), policy);
    }
}
