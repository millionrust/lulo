use super::*;

#[derive(Clone)]
struct FakeService {
    snapshot: Snapshot,
    mutation: Result<(), Error>,
}

impl Service for FakeService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(self.snapshot.clone())
    }

    fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error> {
        self.mutation.clone()?;
        let mut snapshot = self.snapshot.clone();
        snapshot.ntp_enabled = enabled;
        Ok(snapshot)
    }

    fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error> {
        self.snapshot.validate_timezone(timezone)?;
        self.mutation.clone()?;
        let mut snapshot = self.snapshot.clone();
        snapshot.timezone = timezone.into();
        Ok(snapshot)
    }

    fn set_time(&self, target: &ClockTarget) -> Result<Snapshot, Error> {
        self.mutation.clone()?;
        if self.snapshot.ntp_enabled {
            return Err(Error::new(
                ErrorKind::Conflict,
                "turn off automatic time before setting the clock manually",
            ));
        }
        let mut snapshot = self.snapshot.clone();
        snapshot.time_usec = target.time_usec();
        Ok(snapshot)
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        timezone: "UTC".into(),
        can_ntp: true,
        ntp_enabled: true,
        synchronized: true,
        timezones: vec!["Asia/Kolkata".into(), "UTC".into()],
        ..Snapshot::default()
    }
}

#[test]
fn timezone_validation_rejects_paths_and_unknown_values() {
    let snapshot = snapshot();
    assert!(snapshot.validate_timezone("Asia/Kolkata").is_ok());
    assert_eq!(
        snapshot
            .validate_timezone("../etc/passwd")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTimezone
    );
    assert_eq!(
        snapshot
            .validate_timezone("Mars/Olympus")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTimezone
    );
}

#[test]
fn timezone_inventory_is_sorted_deduplicated_and_bounded() {
    let mut values = (0..=MAX_TIMEZONES)
        .map(|index| format!("Etc/Zone_{index}"))
        .collect::<Vec<_>>();
    values.push("UTC".into());
    values.push("UTC".into());
    values.push("../invalid".into());

    let (timezones, truncated) = normalize_timezones(values);

    assert_eq!(timezones.len(), MAX_TIMEZONES);
    assert!(truncated);
    assert!(timezones.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn fake_mutations_return_refreshed_authoritative_state() {
    let service = FakeService {
        snapshot: snapshot(),
        mutation: Ok(()),
    };

    assert!(!service.set_ntp(false).unwrap().ntp_enabled);
    assert_eq!(
        service.set_timezone("Asia/Kolkata").unwrap().timezone,
        "Asia/Kolkata"
    );
}

#[test]
fn failed_fake_mutation_preserves_previous_snapshot() {
    let before = snapshot();
    let service = FakeService {
        snapshot: before.clone(),
        mutation: Err(Error::new(
            ErrorKind::Authorization,
            "authorization was cancelled",
        )),
    };

    assert!(service.set_ntp(false).is_err());
    assert_eq!(service.snapshot().unwrap(), before);
}

#[test]
fn clock_targets_are_unambiguous_bounded_and_canonical() {
    let target = ClockTarget::parse("2026-07-18 11:30:00 +05:30").unwrap();
    assert_eq!(target.display(), "2026-07-18 11:30:00 +05:30");
    assert_eq!(target.time_usec(), 1_784_354_400_000_000);
    assert_eq!(
        ClockTarget::parse("2026-07-18 11:30:00")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTime
    );
    assert!(ClockTarget::parse("1969-12-31 23:59:59 +00:00").is_err());
    assert!(ClockTarget::parse("2262-01-01 00:00:00 +00:00").is_err());
    assert!(ClockTarget::parse("2026-07-18 11:30:00 +05:30\nprivate").is_err());
    assert!(ClockTarget::parse(" 2026-07-18 11:30:00 +05:30").is_err());
    assert!(ClockTarget::parse("2026-07-18 11:30:00 +05:30\n").is_err());
}

#[test]
fn clock_readback_accounts_for_transaction_elapsed_time() {
    let target = 1_700_000_000_000_000;
    assert!(clock_readback_matches(
        target,
        target + 2_500_000,
        std::time::Duration::from_millis(2_500)
    ));
    assert!(!clock_readback_matches(
        target,
        target + 30_000_000,
        std::time::Duration::from_secs(1)
    ));
}

#[test]
fn manual_clock_requires_automatic_time_to_be_off() {
    let service = FakeService {
        snapshot: snapshot(),
        mutation: Ok(()),
    };
    let target = ClockTarget::parse("2026-07-18 11:30:00 +05:30").unwrap();
    assert_eq!(
        service.set_time(&target).unwrap_err().kind(),
        ErrorKind::Conflict
    );

    let mut manual_snapshot = snapshot();
    manual_snapshot.ntp_enabled = false;
    let service = FakeService {
        snapshot: manual_snapshot,
        mutation: Ok(()),
    };
    assert_eq!(
        service.set_time(&target).unwrap().time_usec,
        target.time_usec()
    );
}
