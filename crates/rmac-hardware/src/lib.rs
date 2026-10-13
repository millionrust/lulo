//! Lulo's hardware profile (ADR 0027): what this PC has (battery, panel
//! backlight, touchscreen, pen, touchpad, pointing stick, mouse,
//! fingerprint, cameras, audio, radios, sensors, displays, GPU, memory,
//! CPU) and the low-spec class, so the shell shows only the controls the
//! PC can use and turns down what it cannot afford.
//!
//! Detection reads a few hundred small sysfs files on Linux and a handful
//! of Win32 calls on Windows, in a few milliseconds, always on a thread of
//! its own ([`start`]). The result is cached for the process ([`current`])
//! and detected again only when a device is added or removed: a kernel
//! uevent socket on Linux, `CM_Register_Notification` on Windows. Nothing
//! polls. Surfaces read [`HardwareProfile::adaptations`] rather than the
//! raw fields, so the rules for what to show live in one place.

#[cfg(any(target_os = "linux", windows, test))]
mod gpu;
#[cfg(any(target_os = "linux", test))]
pub mod linux;
mod model;
pub mod store;
#[cfg(test)]
mod tests;
mod watch;
#[cfg(any(windows, test))]
pub mod windows;
#[cfg(windows)]
mod windows_probe;

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub use model::*;

/// Detection's budget: the CI check fails above it.
pub const BUDGET_MICROS: u64 = 20_000;

static CURRENT: Mutex<Option<HardwareProfile>> = Mutex::new(None);
static SUBSCRIBERS: Mutex<Vec<async_channel::Sender<HardwareProfile>>> = Mutex::new(Vec::new());
static STARTED: OnceLock<()> = OnceLock::new();

/// Detect now, on the calling thread (never the UI thread): read, classify
/// and time. Platforms without a detector get an empty standard profile.
pub fn detect() -> HardwareProfile {
    let started = Instant::now();
    #[cfg(target_os = "linux")]
    let profile = linux::detect(&linux::Roots::system());
    #[cfg(windows)]
    let profile = windows::profile(&windows_probe::facts());
    #[cfg(not(any(target_os = "linux", windows)))]
    let profile = HardwareProfile {
        cpu_threads: std::thread::available_parallelism()
            .map(|threads| threads.get() as u32)
            .unwrap_or(0),
        ..HardwareProfile::default()
    };
    let mut profile = profile.classify();
    profile.detection_micros = started.elapsed().as_micros() as u64;
    profile
}

/// The profile detected in this process, once [`start`] has finished its
/// first detection.
pub fn current() -> Option<HardwareProfile> {
    CURRENT.lock().ok().and_then(|current| current.clone())
}

/// What to show and turn down; everything shown until detection is done.
pub fn adaptations() -> Adaptations {
    current()
        .map(|profile| profile.adaptations())
        .unwrap_or_default()
}

/// Two profiles describe the same hardware (detection time aside).
pub fn same_hardware(left: &HardwareProfile, right: &HardwareProfile) -> bool {
    let mut right = right.clone();
    right.detection_micros = left.detection_micros;
    *left == right
}

fn publish(profile: HardwareProfile) {
    let changed = {
        let Ok(mut current) = CURRENT.lock() else {
            return;
        };
        let changed = current
            .as_ref()
            .is_none_or(|known| !same_hardware(known, &profile));
        *current = Some(profile.clone());
        changed
    };
    if !changed {
        return;
    }
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        subscribers.retain(|subscriber| !subscriber.is_closed());
        for subscriber in subscribers.iter() {
            let _ = subscriber.try_send(profile.clone());
        }
    }
}

/// Start detecting, once per process: the first detection on a thread of
/// its own, stored in Lulo's settings ([`store::record`], which logs the
/// one-line summary on the first run and after a change), then again on
/// each hot-plug.
pub fn start() {
    STARTED.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("lulo-hardware".into())
            .spawn(|| {
                let profile = detect();
                store::record(&profile);
                publish(profile);
                watch::run(|| {
                    let profile = detect();
                    store::record(&profile);
                    publish(profile);
                });
            });
        if let Err(error) = spawned {
            eprintln!("lulo-hardware: no hardware profile: {error}");
        }
    });
}

/// The profile now (if known) and whenever the hardware changes. Starts
/// detection if nothing has yet.
pub fn subscribe() -> async_channel::Receiver<HardwareProfile> {
    let (sender, receiver) = async_channel::bounded(4);
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        if let Some(profile) = current() {
            let _ = sender.try_send(profile);
        }
        subscribers.push(sender);
    }
    start();
    receiver
}
