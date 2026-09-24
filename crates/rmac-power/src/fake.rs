//! In-memory [`PowerService`] double for app and view-model tests.
//!
//! `FakePowerService` holds its state in a `Mutex` and never touches
//! UPower or power-profiles-daemon, so app crates can exercise power
//! snapshots and profile switching without a D-Bus session or hardware.
//! See [`crate::contract::assert_power_service_contract`] for the
//! assertions every implementation is expected to satisfy.

use std::sync::Mutex;

use super::*;

/// A snapshot-and-mutate power service seam, introduced alongside the
/// fake it exists to make possible. Unlike `WifiService`/`BluetoothService`
/// this trait has no production caller yet: `rmac-power`'s existing public
/// `snapshot`/`set_profile` functions are untouched, and `SystemPowerService`
/// just delegates to them, so adopting this trait in a consumer (such as
/// `rmac-quick-settings-system::Backend`) is a follow-up, not part of this
/// change.
pub trait PowerService {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_profile(&self, profile: PowerProfile) -> Result<(), Error>;
}

/// Delegates to this crate's own public `snapshot`/`set_profile`
/// functions, which dispatch to UPower/power-profiles-daemon on Linux.
pub struct SystemPowerService;

impl PowerService for SystemPowerService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        crate::snapshot()
    }

    fn set_profile(&self, profile: PowerProfile) -> Result<(), Error> {
        crate::set_profile(profile)
    }
}

/// An in-memory [`PowerService`] seeded with a battery and supported
/// profiles. Construct with [`FakePowerService::new`], seed it with
/// [`with_battery`](Self::with_battery) / [`with_profiles`](Self::with_profiles),
/// then pass `&fake` anywhere a `&impl PowerService` is expected.
pub struct FakePowerService {
    state: Mutex<Snapshot>,
}

impl Default for FakePowerService {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePowerService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(Snapshot::default()),
        }
    }

    pub fn with_battery(self, battery: Battery) -> Self {
        self.state.lock().unwrap().battery = Some(battery);
        self
    }

    pub fn with_profiles(self, profiles: Profiles) -> Self {
        self.state.lock().unwrap().profiles = profiles;
        self
    }
}

impl PowerService for FakePowerService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(self.state.lock().unwrap().clone())
    }

    fn set_profile(&self, profile: PowerProfile) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.profiles.supported.contains(&profile) {
            return Err(Error::new(
                "change the power profile",
                "this profile is not supported",
            ));
        }
        state.profiles.active = Some(profile);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    #[test]
    fn fake_satisfies_the_shared_power_service_contract() {
        let service = FakePowerService::new().with_profiles(Profiles {
            available: true,
            active: Some(PowerProfile::Balanced),
            supported: vec![PowerProfile::Balanced, PowerProfile::PowerSaver],
            performance_degraded: None,
        });
        contract::assert_power_service_contract(&service);
    }

    #[test]
    fn setting_an_unsupported_profile_is_rejected() {
        let service = FakePowerService::new().with_profiles(Profiles {
            available: true,
            active: Some(PowerProfile::Balanced),
            supported: vec![PowerProfile::Balanced],
            performance_degraded: None,
        });
        assert!(service.set_profile(PowerProfile::Performance).is_err());
    }
}
