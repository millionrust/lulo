use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::snapshot::candidate_watch_targets;

struct FakeBackend {
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    user_dirs: RefCell<io::Result<Option<String>>>,
    existing: Vec<PathBuf>,
    trash_entries: RefCell<Result<Vec<TrashEntryId>, &'static str>>,
    purged: RefCell<Vec<TrashEntryId>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            home: Some(PathBuf::from("/home/alex")),
            config_home: None,
            user_dirs: RefCell::new(Ok(None)),
            existing: vec![
                PathBuf::from("/home/alex"),
                PathBuf::from("/home/alex/Downloads"),
            ],
            trash_entries: RefCell::new(Ok(Vec::new())),
            purged: RefCell::new(Vec::new()),
        }
    }
}

impl Backend for FakeBackend {
    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn config_home(&self) -> Option<PathBuf> {
        self.config_home.clone()
    }

    fn read_optional(&self, _: &Path) -> io::Result<Option<String>> {
        self.user_dirs.replace(Ok(None))
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        Ok(self.existing.iter().any(|existing| existing == path))
    }

    fn trash_count(&self) -> Result<usize, String> {
        self.trash_entries
            .borrow()
            .as_ref()
            .map(Vec::len)
            .map_err(|error| (*error).to_owned())
    }

    fn trash_entries(&self) -> Result<Vec<TrashEntryId>, String> {
        self.trash_entries
            .borrow()
            .as_ref()
            .cloned()
            .map_err(|error| (*error).to_owned())
    }

    fn purge_trash(&self, reviewed: &[TrashEntryId]) -> Result<(), String> {
        let mut inventory = self.trash_entries.borrow_mut();
        let entries = inventory.as_mut().map_err(|error| (*error).to_owned())?;
        if reviewed.iter().any(|reviewed| !entries.contains(reviewed)) {
            return Err("Trash changed after the deletion review".into());
        }
        self.purged.borrow_mut().extend_from_slice(reviewed);
        entries.retain(|entry| !reviewed.contains(entry));
        Ok(())
    }
}

fn trash_entries(count: u8) -> RefCell<Result<Vec<TrashEntryId>, &'static str>> {
    RefCell::new(Ok((0..count)
        .map(|index| TrashEntryId::from_authority_bytes(&[index]))
        .collect()))
}

#[test]
fn snapshot_uses_configured_downloads_and_complete_trash_count() {
    let backend = FakeBackend {
        user_dirs: RefCell::new(Ok(Some("XDG_DOWNLOAD_DIR=\"$HOME/Transfers\"\n".into()))),
        existing: vec![
            PathBuf::from("/home/alex"),
            PathBuf::from("/home/alex/Transfers"),
        ],
        trash_entries: trash_entries(3),
        ..Default::default()
    };
    let report = snapshot(&backend).expect("snapshot succeeds");
    assert_eq!(
        report.snapshot.downloads.path,
        Path::new("/home/alex/Transfers")
    );
    assert!(report.snapshot.downloads.exists);
    assert!(report.snapshot.downloads_configured);
    assert_eq!(report.snapshot.trash.item_count, 3);
    assert!(!report.snapshot.trash.empty);
    assert!(report.warnings.is_empty());
}

#[test]
fn malformed_user_dirs_falls_back_with_a_visible_warning() {
    let backend = FakeBackend {
        user_dirs: RefCell::new(Ok(Some("XDG_DOWNLOAD_DIR=\"relative\"\n".into()))),
        ..Default::default()
    };
    let report = snapshot(&backend).expect("fallback succeeds");
    assert_eq!(
        report.snapshot.downloads.path,
        Path::new("/home/alex/Downloads")
    );
    assert_eq!(report.warnings[0].operation, Operation::ReadUserDirs);
}

#[test]
fn trash_failure_does_not_hide_other_places() {
    let backend = FakeBackend {
        trash_entries: RefCell::new(Err("mount disappeared")),
        ..Default::default()
    };
    let report = snapshot(&backend).expect("places remain available");
    assert!(report.snapshot.downloads.exists);
    assert!(!report.snapshot.trash.available);
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.operation == Operation::InspectTrash));
}

#[test]
fn empty_trash_requires_confirmation_and_refreshes_authority() {
    let backend = FakeBackend {
        trash_entries: trash_entries(2),
        ..Default::default()
    };
    let snapshot = rmac_places::TrashSnapshot {
        available: true,
        empty: false,
        item_count: 2,
    };
    let review = prepare_empty_trash(&snapshot, &backend)
        .expect("review prepares")
        .expect("nonempty review");
    assert_eq!(review.item_count(), 2);
    assert!(confirm_empty_trash(review.clone(), false).is_none());
    let confirmation = confirm_empty_trash(review, true).expect("user confirmed");
    let refreshed = empty_trash(confirmation, &backend).expect("purge succeeds");
    assert_eq!(backend.purged.borrow().len(), 2);
    assert!(refreshed.empty);
    assert_eq!(refreshed.item_count, 0);
}

#[test]
fn items_added_after_review_are_never_purged() {
    let backend = FakeBackend {
        trash_entries: trash_entries(2),
        ..Default::default()
    };
    let snapshot = rmac_places::TrashSnapshot {
        available: true,
        empty: false,
        item_count: 2,
    };
    let review = prepare_empty_trash(&snapshot, &backend)
        .expect("review prepares")
        .expect("nonempty review");
    let added = TrashEntryId::from_authority_bytes(b"added after confirmation");
    backend
        .trash_entries
        .borrow_mut()
        .as_mut()
        .expect("inventory")
        .push(added);

    let refreshed = empty_trash(
        confirm_empty_trash(review, true).expect("confirmed"),
        &backend,
    )
    .expect("only reviewed entries purge");
    assert_eq!(backend.purged.borrow().len(), 2);
    assert_eq!(backend.trash_entries().unwrap(), [added]);
    assert_eq!(refreshed.item_count, 1);
    assert!(!refreshed.empty);
}

#[test]
fn stale_or_duplicate_review_authority_fails_closed() {
    let backend = FakeBackend {
        trash_entries: trash_entries(2),
        ..Default::default()
    };
    let snapshot = rmac_places::TrashSnapshot {
        available: true,
        empty: false,
        item_count: 2,
    };
    let review = prepare_empty_trash(&snapshot, &backend)
        .expect("review prepares")
        .expect("nonempty review");
    backend
        .trash_entries
        .borrow_mut()
        .as_mut()
        .expect("inventory")
        .pop();
    assert!(empty_trash(
        confirm_empty_trash(review, true).expect("confirmed"),
        &backend,
    )
    .is_err());
    assert!(backend.purged.borrow().is_empty());

    let duplicate = TrashEntryId::from_authority_bytes(b"same");
    let backend = FakeBackend {
        trash_entries: RefCell::new(Ok(vec![duplicate, duplicate])),
        ..Default::default()
    };
    assert!(prepare_empty_trash(&snapshot, &backend).is_err());
}

#[test]
fn empty_trash_review_debug_redacts_entry_authority() {
    let backend = FakeBackend {
        trash_entries: trash_entries(1),
        ..Default::default()
    };
    let review = prepare_empty_trash(
        &rmac_places::TrashSnapshot {
            available: true,
            empty: false,
            item_count: 1,
        },
        &backend,
    )
    .unwrap()
    .unwrap();
    let debug = format!("{review:?}");
    assert!(debug.contains("<private>"));
    assert!(!debug.contains("TrashEntryId"));
}

#[test]
fn watch_targets_cover_config_place_parents_and_every_known_trash_bin() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rmac-places-watch-{}-{unique}", std::process::id()));
    let home = root.join("home");
    let config = home.join(".config");
    let data = home.join(".local/share");
    let downloads_parent = root.join("media");
    let trash = root.join("mounted/.Trash-1000");
    for path in [&config, &data, &downloads_parent, &trash] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::create_dir_all(trash.join("files")).unwrap();
    std::fs::create_dir_all(trash.join("info")).unwrap();
    let mounts = root.join("mounts");
    std::fs::write(&mounts, []).unwrap();
    let report = Report {
        snapshot: rmac_places::Snapshot {
            home: rmac_places::Place {
                path: home.clone(),
                exists: true,
            },
            downloads: rmac_places::Place {
                path: downloads_parent.join("Downloads"),
                exists: false,
            },
            downloads_configured: true,
            trash: rmac_places::TrashSnapshot::default(),
        },
        warnings: Vec::new(),
    };

    let targets = candidate_watch_targets(
        &report,
        &config,
        &data,
        std::slice::from_ref(&trash),
        Some(&mounts),
    );
    for expected in [
        config,
        home,
        downloads_parent,
        data,
        trash.join("files"),
        trash.join("info"),
        mounts,
    ] {
        assert!(targets.contains(&expected), "missing {expected:?}");
    }
    assert_eq!(targets.iter().collect::<BTreeSet<_>>().len(), targets.len());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn watch_failure_debug_redacts_backend_diagnostics() {
    let event = WatchEvent::Failed {
        detail: "/home/alex/private mount failed".into(),
    };
    let debug = format!("{event:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("alex"));
}
