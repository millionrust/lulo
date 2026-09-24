use super::*;
use crate::facts::{
    graphics_label, is_drm_card_name, memory_from_meminfo, normalize_pci_id, os_release_value,
    processor_from_cpuinfo,
};
use crate::host::{normalize_static_hostname, verify_static_hostname};
use crate::watch::owner_change_event;

#[derive(Clone)]
struct FakeService {
    snapshot: Snapshot,
    mutation: Result<(), Error>,
}

impl Service for FakeService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(self.snapshot.clone())
    }

    fn set_static_hostname(&self, hostname: &str) -> Result<Snapshot, Error> {
        let hostname = normalize_static_hostname(hostname)?;
        self.mutation.clone()?;
        let mut snapshot = self.snapshot.clone();
        snapshot.hostname = hostname.clone();
        snapshot.static_hostname = Some(hostname);
        Ok(snapshot)
    }
}

#[test]
fn hostname_validation_matches_static_hostname_constraints() {
    for valid in ["rmac", "studio-pc", "A1"] {
        assert!(validate_static_hostname(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "has space", "-leading", "trailing-", "two.labels"] {
        assert_eq!(
            validate_static_hostname(invalid).unwrap_err().kind(),
            ErrorKind::InvalidName,
            "{invalid}"
        );
    }
    assert!(validate_static_hostname(&"a".repeat(64)).is_err());
    assert_eq!(normalize_static_hostname("Studio-PC").unwrap(), "studio-pc");
}

#[test]
fn fake_service_returns_refreshed_authoritative_hostname() {
    let service = FakeService {
        snapshot: Snapshot {
            hostname: "before".into(),
            operating_system: "Ubuntu".into(),
            ..Snapshot::default()
        },
        mutation: Ok(()),
    };

    let snapshot = service.set_static_hostname("after").unwrap();

    assert_eq!(snapshot.hostname, "after");
    assert_eq!(snapshot.static_hostname.as_deref(), Some("after"));
}

#[test]
fn display_hostname_prefers_the_authoritative_static_value() {
    let snapshot = Snapshot {
        hostname: "transient".into(),
        static_hostname: Some("static-name".into()),
        pretty_hostname: Some("Unrelated Pretty Name".into()),
        ..Snapshot::default()
    };

    assert_eq!(snapshot.display_hostname(), "static-name");
}

#[test]
fn failed_fake_mutation_does_not_replace_last_known_good_state() {
    let before = Snapshot {
        hostname: "before".into(),
        static_hostname: Some("before".into()),
        ..Snapshot::default()
    };
    let service = FakeService {
        snapshot: before.clone(),
        mutation: Err(Error::new(
            ErrorKind::Authorization,
            "set the hostname",
            "authorization was cancelled",
        )),
    };

    assert!(service.set_static_hostname("after").is_err());
    assert_eq!(service.snapshot().unwrap(), before);
}

#[test]
fn diagnostics_exclude_identity_and_serial_data() {
    let snapshot = Snapshot {
        hostname: "private-host".into(),
        static_hostname: Some("private-host".into()),
        operating_system: "Ubuntu 26.04 LTS".into(),
        kernel: "Linux 6.18".into(),
        architecture: "x86_64".into(),
        processor: Some("Example CPU".into()),
        graphics: Some("Example GPU".into()),
        ..Snapshot::default()
    };

    let report = snapshot.diagnostic_report();

    assert!(report.contains("Ubuntu 26.04 LTS"));
    assert!(report.contains("Example GPU"));
    assert!(!report.contains("private-host"));
    assert!(!report.to_ascii_lowercase().contains("serial"));
    assert!(!report.to_ascii_lowercase().contains("user"));
}

#[test]
fn diagnostics_drop_control_characters_in_public_snapshot_fields() {
    let snapshot = Snapshot {
        operating_system: "Ubuntu\nHostname: private-host".into(),
        kernel: "Linux 6.18".into(),
        architecture: "x86_64".into(),
        processor: Some("CPU\tserial".into()),
        memory: Some("17.2 GB".into()),
        ..Snapshot::default()
    };

    let report = snapshot.diagnostic_report();

    assert!(!report.contains("private-host"));
    assert!(!report.contains("CPU"));
    assert!(report.contains("Memory: 17.2 GB"));
}

#[test]
fn hostname_readback_must_match_the_requested_static_name() {
    let stale = Snapshot {
        hostname: "old-name".into(),
        static_hostname: Some("old-name".into()),
        ..Snapshot::default()
    };

    let error = verify_static_hostname(stale, "new-name").unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Mutation);
}

#[test]
fn standard_linux_fact_files_are_parsed_without_private_fields() {
    assert_eq!(
        os_release_value(
            "NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\n",
            "PRETTY_NAME"
        )
        .as_deref(),
        Some("Ubuntu 26.04 LTS")
    );
    assert_eq!(
        processor_from_cpuinfo("processor : 0\nmodel name : Example CPU\n").as_deref(),
        Some("Example CPU")
    );
    assert_eq!(
        memory_from_meminfo("MemTotal:       16777216 kB\n").as_deref(),
        Some("17.2 GB")
    );
}

#[test]
fn graphics_labels_use_only_driver_and_public_pci_ids() {
    assert!(is_drm_card_name("card0"));
    assert!(is_drm_card_name("card12"));
    assert!(!is_drm_card_name("card0-DP-1"));
    assert!(!is_drm_card_name("renderD128"));
    assert_eq!(normalize_pci_id("0x10DE".into()).as_deref(), Some("10de"));
    assert_eq!(normalize_pci_id("not-an-id".into()), None);
    assert_eq!(
        graphics_label(Some("nvidia"), Some("10de"), Some("2684")).as_deref(),
        Some("NVIDIA (nvidia, 10de:2684)")
    );
    assert_eq!(
        graphics_label(Some("amdgpu"), None, None).as_deref(),
        Some("amdgpu")
    );
}

#[test]
fn hostname_owner_loss_is_routine_idle_exit_not_an_error() {
    // systemd-hostnamed is bus-activated and exits when idle; the owner
    // going empty is expected and must not be surfaced as Unavailable.
    assert_eq!(owner_change_event("org.freedesktop.hostname1", ""), None);
    assert_eq!(
        owner_change_event("org.freedesktop.hostname1", ":1.42"),
        Some(WatchEvent::Changed)
    );
    assert_eq!(owner_change_event("org.example.Other", ":1.42"), None);
    assert_eq!(owner_change_event("org.example.Other", ""), None);
}
