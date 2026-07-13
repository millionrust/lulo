//! Platform-neutral software-update snapshots and collection rules.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

pub const MAX_UPDATES: usize = 512;
const MAX_TEXT_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateKind {
    Security,
    Important,
    BugFix,
    Enhancement,
    Blocked,
    Low,
    #[default]
    Normal,
    Unknown,
}

impl UpdateKind {
    pub fn from_packagekit(value: &str) -> Self {
        match value {
            "security" => Self::Security,
            "important" => Self::Important,
            "bugfix" => Self::BugFix,
            "enhancement" => Self::Enhancement,
            "blocked" => Self::Blocked,
            "low" => Self::Low,
            "normal" => Self::Normal,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::Important => "Important",
            Self::BugFix => "Bug fix",
            Self::Enhancement => "Enhancement",
            Self::Blocked => "Blocked",
            Self::Low => "Low priority",
            Self::Normal => "Update",
            Self::Unknown => "Update",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub package_id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub kind: UpdateKind,
}

impl Update {
    pub fn from_packagekit(info: &str, package_id: &str, summary: &str) -> Option<Self> {
        if package_id.len() > MAX_TEXT_BYTES || package_id.chars().any(char::is_control) {
            return None;
        }
        let mut fields = package_id.split(';');
        let name = bounded_text(fields.next()?);
        let version = bounded_text(fields.next()?);
        if name.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            package_id: package_id.to_string(),
            name,
            version,
            summary: bounded_text(summary),
            kind: UpdateKind::from_packagekit(info),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub updates: Vec<Update>,
    pub truncated: bool,
}

impl Snapshot {
    pub fn security_count(&self) -> usize {
        self.updates
            .iter()
            .filter(|update| update.kind == UpdateKind::Security)
            .count()
    }

    pub fn blocked_count(&self) -> usize {
        self.updates
            .iter()
            .filter(|update| update.kind == UpdateKind::Blocked)
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub cache_age_seconds: u32,
}

impl Request {
    pub const fn cached() -> Self {
        Self {
            cache_age_seconds: 3600,
        }
    }

    pub const fn refresh() -> Self {
        Self {
            cache_age_seconds: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Unavailable,
    Timeout,
    Protocol,
    Backend,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl AsRef<str>) -> Self {
        Self {
            kind,
            detail: bounded_text(detail.as_ref()),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub type SnapshotFuture<'a> = Pin<Box<dyn Future<Output = Result<Snapshot, Error>> + Send + 'a>>;

/// Injectable asynchronous boundary for PackageKit and deterministic fixtures.
pub trait Source {
    fn snapshot(&self, request: Request) -> SnapshotFuture<'_>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Package {
        info: String,
        package_id: String,
        summary: String,
    },
    BackendError {
        code: String,
        detail: String,
    },
    Finished {
        exit: String,
    },
}

#[derive(Default)]
pub struct Collector {
    snapshot: Snapshot,
    backend_error: Option<Error>,
    finished: bool,
}

impl Collector {
    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service sent data after finishing",
            ));
        }
        match event {
            Event::Package {
                info,
                package_id,
                summary,
            } => {
                let Some(update) = Update::from_packagekit(&info, &package_id, &summary) else {
                    return Err(Error::new(
                        ErrorKind::Protocol,
                        "the update service returned an invalid package identifier",
                    ));
                };
                if self.snapshot.updates.len() >= MAX_UPDATES {
                    self.snapshot.truncated = true;
                } else if !self
                    .snapshot
                    .updates
                    .iter()
                    .any(|existing| existing.package_id == update.package_id)
                {
                    self.snapshot.updates.push(update);
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(Error::new(
                    ErrorKind::Backend,
                    format!("{}: {}", bounded_text(&code), bounded_text(&detail)),
                ));
            }
            Event::Finished { exit } => {
                self.finished = true;
                if exit == "cancelled" {
                    return Err(Error::new(
                        ErrorKind::Cancelled,
                        "the update check was cancelled",
                    ));
                }
                if exit != "success" {
                    return Err(self.backend_error.take().unwrap_or_else(|| {
                        Error::new(
                            ErrorKind::Backend,
                            format!("the update service finished with status {exit}"),
                        )
                    }));
                }
                if let Some(error) = self.backend_error.take() {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Snapshot, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service ended without a completion signal",
            ));
        }
        Ok(self.snapshot)
    }
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_TEXT_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
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

    fn package(kind: &str, id: &str) -> Event {
        Event::Package {
            info: kind.into(),
            package_id: id.into(),
            summary: "Package summary".into(),
        }
    }

    #[test]
    fn collector_classifies_and_deduplicates_updates() {
        let mut collector = Collector::default();
        collector
            .apply(package("security", "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package("security", "kernel;6.18;amd64;updates"))
            .unwrap();
        collector
            .apply(package("blocked", "driver;2.0;amd64;updates"))
            .unwrap();
        collector
            .apply(Event::Finished {
                exit: "success".into(),
            })
            .unwrap();

        let snapshot = collector.finish().unwrap();
        assert_eq!(snapshot.updates.len(), 2);
        assert_eq!(snapshot.security_count(), 1);
        assert_eq!(snapshot.blocked_count(), 1);
    }

    #[test]
    fn backend_failure_is_typed_and_bounded() {
        let mut collector = Collector::default();
        collector
            .apply(Event::BackendError {
                code: "failed-initialization".into(),
                detail: "repository failed\nprivate continuation".into(),
            })
            .unwrap();
        let error = collector
            .apply(Event::Finished {
                exit: "failed".into(),
            })
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Backend);
        assert!(!error.detail().contains('\n'));
        assert!(error.detail().len() <= MAX_TEXT_BYTES);
    }

    #[test]
    fn malformed_package_ids_fail_closed() {
        let mut collector = Collector::default();
        let error = collector
            .apply(package("normal", "missing-version"))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Protocol);
    }

    #[test]
    fn collector_bounds_untrusted_package_volume_and_ids() {
        let mut collector = Collector::default();
        for index in 0..=MAX_UPDATES {
            collector
                .apply(package(
                    "normal",
                    &format!("package-{index};1.0;amd64;updates"),
                ))
                .unwrap();
        }
        collector
            .apply(Event::Finished {
                exit: "success".into(),
            })
            .unwrap();

        let snapshot = collector.finish().unwrap();
        assert_eq!(snapshot.updates.len(), MAX_UPDATES);
        assert!(snapshot.truncated);
        assert!(Update::from_packagekit(
            "normal",
            &format!("name;{};amd64;updates", "v".repeat(MAX_TEXT_BYTES)),
            "oversized",
        )
        .is_none());
    }

    #[test]
    fn fake_source_preserves_success_and_unavailable_states() {
        let expected = Snapshot {
            updates: vec![Update::from_packagekit(
                "normal",
                "example;2.0;amd64;updates",
                "Example",
            )
            .unwrap()],
            truncated: false,
        };
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
}
