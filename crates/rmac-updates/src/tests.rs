//! Focused software-update contracts.

use super::*;

struct FakeSource {
    result: Result<Snapshot, Error>,
}

impl Source for FakeSource {
    fn snapshot(&self, _request: Request) -> SnapshotFuture<'_> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

fn package(kind: u32, id: &str) -> Event {
    Event::Package {
        info: kind,
        package_id: id.into(),
        summary: "Package summary".into(),
    }
}

fn installable_snapshot() -> Snapshot {
    Snapshot {
        updates: vec![
            Update::from_packagekit(8, "kernel;6.18;amd64;updates", "Kernel update").unwrap(),
        ],
        install_supported: true,
        ..Snapshot::default()
    }
}

#[test]
fn collector_classifies_and_deduplicates_modern_packagekit_enums() {
    let mut collector = Collector::default();
    collector
        .apply(package(8, "kernel;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(package(8, "kernel;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(package(9, "driver;2.0;amd64;updates"))
        .unwrap();
    collector.apply(Event::Finished { exit: 1 }).unwrap();

    let snapshot = collector.finish().unwrap();
    assert_eq!(snapshot.updates.len(), 2);
    assert_eq!(snapshot.security_count(), 1);
    assert_eq!(snapshot.blocked_count(), 1);
}

#[test]
fn backend_failure_is_typed_without_exposing_backend_text() {
    let mut collector = Collector::default();
    collector
        .apply(Event::BackendError {
            code: 48,
            detail: "denied for /private/path\nuser name".into(),
        })
        .unwrap();
    let error = collector.apply(Event::Finished { exit: 2 }).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Authorization);
    assert!(!error.detail().contains("private"));
    assert!(!error.detail().contains("user name"));
}

#[test]
fn common_backend_failures_have_private_safe_actionable_messages() {
    let network = packagekit_error(2, "/private/repository token");
    assert_eq!(network.kind(), ErrorKind::Backend);
    assert!(network.detail().contains("network"));
    assert!(!network.detail().contains("private"));

    let lock = packagekit_error(26, "pid 42 owned by user");
    assert!(lock.detail().contains("lock"));
    assert!(!lock.detail().contains("user"));

    let space = packagekit_error(46, "/var is full");
    assert!(space.detail().contains("disk space"));
    assert!(!space.detail().contains("/var"));
}

#[test]
fn malformed_package_ids_fail_closed() {
    let mut collector = Collector::default();
    let error = collector.apply(package(5, "missing-version")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Protocol);
    assert!(Update::from_packagekit(5, "name;1;arch;repo;extra", "bad").is_none());
}

#[test]
fn collector_bounds_untrusted_package_volume_and_ids() {
    let mut collector = Collector::default();
    for index in 0..=MAX_UPDATES {
        collector
            .apply(package(5, &format!("package-{index};1.0;amd64;updates")))
            .unwrap();
    }
    collector.apply(Event::Finished { exit: 1 }).unwrap();

    let snapshot = collector.finish().unwrap();
    assert_eq!(snapshot.updates.len(), MAX_UPDATES);
    assert!(snapshot.truncated);
    assert!(Update::from_packagekit(
        5,
        &format!("name;{};amd64;updates", "v".repeat(MAX_PACKAGE_ID_BYTES)),
        "oversized",
    )
    .is_none());
}

#[test]
fn simulation_collects_exact_changes_and_restart_requirement() {
    let mut collector = PlanCollector::new(&installable_snapshot()).unwrap();
    collector
        .apply(package(11, "kernel;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(package(12, "kernel-helper;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(Event::RestartRequired {
            kind: 6,
            package_id: "kernel;6.18;amd64;updates".into(),
        })
        .unwrap();
    collector.apply(Event::Finished { exit: 1 }).unwrap();

    let plan = collector.finish().unwrap();
    assert_eq!(plan.requested_ids(), vec!["kernel;6.18;amd64;updates"]);
    assert_eq!(plan.change_count(ChangeKind::Update), 1);
    assert_eq!(plan.change_count(ChangeKind::Install), 1);
    assert_eq!(plan.restart, RestartRequirement::SecuritySystem);
}

#[test]
fn simulation_rejects_untrusted_and_oversized_plans() {
    let mut untrusted = PlanCollector::new(&installable_snapshot()).unwrap();
    assert_eq!(
        untrusted
            .apply(package(23, "unsigned;1.0;amd64;third-party"))
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );

    let mut oversized = PlanCollector::new(&installable_snapshot()).unwrap();
    for index in 0..MAX_PLAN_CHANGES {
        oversized
            .apply(package(12, &format!("dependency-{index};1;amd64;updates")))
            .unwrap();
    }
    assert_eq!(
        oversized
            .apply(package(12, "one-too-many;1;amd64;updates"))
            .unwrap_err()
            .kind(),
        ErrorKind::Protocol
    );
}

#[test]
fn installability_requires_complete_supported_authority() {
    let mut snapshot = installable_snapshot();
    assert!(snapshot.can_prepare_install());
    snapshot.truncated = true;
    assert!(!snapshot.can_prepare_install());
    snapshot.truncated = false;
    snapshot.install_supported = false;
    assert!(!snapshot.can_prepare_install());
}

#[test]
fn progress_handles_unknown_percentages_and_cancellation() {
    let mut progress = InstallProgress::default();
    progress.set_percentage(87);
    assert_eq!(progress.percentage, Some(87));
    progress.set_percentage(101);
    assert_eq!(progress.percentage, None);
    assert_eq!(
        InstallPhase::from_packagekit(31),
        InstallPhase::WaitingForAuthorization
    );

    let cancellation = Cancellation::default();
    assert!(!cancellation.is_cancelled());
    cancellation.cancel();
    assert!(cancellation.is_cancelled());
}

#[test]
fn installation_collects_unique_packages_and_strongest_restart() {
    let mut collector = InstallCollector::default();
    collector
        .apply(package(10, "kernel;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(package(11, "kernel;6.18;amd64;updates"))
        .unwrap();
    collector
        .apply(Event::RestartRequired {
            kind: 4,
            package_id: "kernel;6.18;amd64;updates".into(),
        })
        .unwrap();
    collector
        .apply(Event::RestartRequired {
            kind: 6,
            package_id: "kernel;6.18;amd64;updates".into(),
        })
        .unwrap();
    collector.apply(Event::Finished { exit: 1 }).unwrap();

    let result = collector.finish().unwrap();
    assert_eq!(result.changed_packages, 1);
    assert_eq!(result.restart, RestartRequirement::SecuritySystem);
}

#[test]
fn installation_rejects_an_unbounded_result() {
    let mut collector = InstallCollector::default();
    for index in 0..MAX_PLAN_CHANGES {
        collector
            .apply(package(11, &format!("changed-{index};1;amd64;updates")))
            .unwrap();
    }
    assert_eq!(
        collector
            .apply(package(11, "one-too-many;1;amd64;updates"))
            .unwrap_err()
            .kind(),
        ErrorKind::Protocol
    );
}

#[test]
fn fake_source_preserves_success_and_unavailable_states() {
    let expected = installable_snapshot();
    let success = FakeSource {
        result: Ok(expected.clone()),
    };
    let unavailable = FakeSource {
        result: Err(Error::new(
            ErrorKind::Unavailable,
            "PackageKit is unavailable",
        )),
    };

    assert_eq!(
        futures_lite_for_test(success.snapshot(Request::cached())),
        Ok(expected)
    );
    assert_eq!(
        futures_lite_for_test(unavailable.snapshot(Request::cached()))
            .unwrap_err()
            .kind(),
        ErrorKind::Unavailable
    );
}

fn futures_lite_for_test<T>(future: impl Future<Output = T>) -> T {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixture future unexpectedly pending"),
    }
}
