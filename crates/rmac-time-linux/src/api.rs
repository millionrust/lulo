use rmac_time::{ClockTarget, Error, ErrorKind, Service, Snapshot};

use crate::system::{system_set_ntp, system_set_time, system_set_timezone, system_snapshot};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error> {
        let before = self.snapshot()?;
        if !before.can_ntp {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "no compatible network time service is installed",
            ));
        }
        if before.ntp_enabled == enabled {
            return Ok(before);
        }
        system_set_ntp(enabled)?;
        let after = self.snapshot()?;
        if after.ntp_enabled != enabled {
            return Err(Error::new(
                ErrorKind::Mismatch,
                "timedated did not confirm the requested automatic-time state",
            ));
        }
        Ok(after)
    }

    fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error> {
        let snapshot = self.snapshot()?;
        snapshot.validate_timezone(timezone)?;
        if snapshot.timezone == timezone {
            return Ok(snapshot);
        }
        system_set_timezone(timezone)?;
        let after = self.snapshot()?;
        if after.timezone != timezone {
            return Err(Error::new(
                ErrorKind::Mismatch,
                "timedated did not confirm the requested time zone",
            ));
        }
        Ok(after)
    }

    fn set_time(&self, target: &ClockTarget) -> Result<Snapshot, Error> {
        let before = self.snapshot()?;
        if before.ntp_enabled {
            return Err(Error::new(
                ErrorKind::Conflict,
                "turn off automatic time before setting the clock manually",
            ));
        }
        system_set_time(target.time_usec())?;
        // Interactive authorization can take arbitrarily long and occurs
        // before timedated applies the clock. Only account for time spent
        // obtaining the authoritative post-mutation snapshot.
        let started = std::time::Instant::now();
        let after = self.snapshot()?;
        if !rmac_time::clock_readback_matches(
            target.time_usec(),
            after.time_usec,
            started.elapsed(),
        ) {
            return Err(Error::new(
                ErrorKind::Mismatch,
                "timedated did not confirm the requested system time",
            ));
        }
        Ok(after)
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_ntp(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_ntp(enabled)
}

pub fn set_timezone(timezone: &str) -> Result<Snapshot, Error> {
    SystemService.set_timezone(timezone)
}

pub fn set_time(target: &ClockTarget) -> Result<Snapshot, Error> {
    SystemService.set_time(target)
}
