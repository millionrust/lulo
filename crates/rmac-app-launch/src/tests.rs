use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::application::{may_fallback, runnable_program};
use crate::document::is_regular_document;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn errors_are_actionable_and_redact_commands() {
    let error = Error {
        kind: ErrorKind::Io(std::io::ErrorKind::NotFound),
    };
    assert_eq!(
        error.to_string(),
        "the application executable is unavailable"
    );
    assert!(!format!("{error:?}").contains("/home"));
}

#[test]
fn fallback_never_bypasses_compositor_rejection_or_protocol_failure() {
    assert!(may_fallback(rmac_compositor::ActionErrorKind::Unavailable));
    assert!(may_fallback(rmac_compositor::ActionErrorKind::Transport));
    assert!(may_fallback(rmac_compositor::ActionErrorKind::Unsupported));
    assert!(!may_fallback(rmac_compositor::ActionErrorKind::Rejected));
    assert!(!may_fallback(rmac_compositor::ActionErrorKind::Protocol));
}

#[test]
fn integration_errors_are_private_and_actionable() {
    let open = ItemError {
        operation: ItemOperation::Open,
    };
    let reveal = ItemError {
        operation: ItemOperation::Reveal,
    };
    assert_eq!(open.to_string(), "the item could not be opened");
    assert_eq!(reveal.to_string(), "the item could not be revealed");
    assert_eq!(
        AssociationError.to_string(),
        "compatible applications could not be loaded"
    );
    assert_eq!(
        OpenWithError {
            default_changed: true,
        }
        .to_string(),
        "the default application changed, but the file could not be opened"
    );
    let diagnostics = format!(
        "{open:?} {reveal:?} {:?} {:?}",
        AssociationError,
        OpenWithError {
            default_changed: false,
        }
    );
    assert!(!diagnostics.contains("/home"));
    assert!(!diagnostics.contains("gio"));
    assert_eq!(
        RecentDocumentError.to_string(),
        "the document could not be added to Recents"
    );
}

#[test]
fn notification_documents_are_absolute_regular_files_without_final_symlinks() {
    let root = std::env::temp_dir().join(format!(
        "rmac-app-launch-document-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let document = root.join("document.txt");
    std::fs::write(&document, b"private").unwrap();
    assert!(is_regular_document(&document));
    assert!(!is_regular_document(&root));
    assert!(!is_regular_document(Path::new("document.txt")));

    #[cfg(unix)]
    {
        let link = root.join("document-link.txt");
        std::os::unix::fs::symlink(&document, &link).unwrap();
        assert!(!is_regular_document(&link));
    }

    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn launches_check_the_program_before_the_compositor_spawns_it() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!(
        "rmac-app-launch-program-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("tool");
    std::fs::write(&tool, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let plain = bin.join("plain");
    std::fs::write(&plain, b"data").unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    let dangling = bin.join("dangling");
    std::os::unix::fs::symlink(root.join("gone"), &dangling).unwrap();
    let path = std::ffi::OsString::from(format!("/nonexistent-rmac:{}", bin.display()));

    assert_eq!(runnable_program(&tool, None), Ok(()));
    assert_eq!(runnable_program(Path::new("tool"), Some(&path)), Ok(()));
    assert_eq!(
        runnable_program(Path::new("missing"), Some(&path)),
        Err(std::io::ErrorKind::NotFound)
    );
    assert_eq!(
        runnable_program(&dangling, None),
        Err(std::io::ErrorKind::NotFound)
    );
    assert_eq!(
        runnable_program(&bin, None),
        Err(std::io::ErrorKind::NotFound)
    );
    assert_eq!(
        runnable_program(Path::new(""), Some(&path)),
        Err(std::io::ErrorKind::NotFound)
    );
    // Root may execute anything; the permission check only holds for users.
    // SAFETY: `geteuid` has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        assert_eq!(
            runnable_program(&plain, None),
            Err(std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(
            runnable_program(Path::new("plain"), Some(&path)),
            Err(std::io::ErrorKind::PermissionDenied)
        );
    }

    std::fs::remove_dir_all(root).unwrap();
}
