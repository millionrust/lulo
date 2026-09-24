//! Focused durable storage contracts.

use super::*;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-storage-{label}-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn atomic_write_replaces_target_without_leaving_a_temp_file() {
    let root = temp_root("atomic");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("settings.txt");
    std::fs::write(&target, "before").unwrap();

    atomic_write(&target, b"after").unwrap();

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounded_read_rejects_a_file_before_returning_excess_bytes() {
    let root = temp_root("bounded-read");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("document.txt");
    std::fs::write(&target, b"12345").unwrap();

    assert_eq!(FileSystem.read_bounded(&target, 5).unwrap(), b"12345");
    assert_eq!(
        FileSystem.read_bounded(&target, 4).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn durable_removal_unlinks_the_exact_entry_and_reports_missing() {
    let root = temp_root("durable-remove");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("managed.bin");
    std::fs::write(&target, b"managed bytes").unwrap();

    remove_file_durable(&target).unwrap();

    assert!(!target.exists());
    assert_eq!(
        remove_file_durable(&target).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn private_bounded_read_refuses_symlinks_and_hard_links() {
    use std::os::unix::fs::symlink;

    let root = temp_root("private-bounded-read");
    std::fs::create_dir(&root).unwrap();
    let regular = root.join("regular");
    let symlink_path = root.join("symlink");
    let hard_link = root.join("hard-link");
    std::fs::write(&regular, b"private draft").unwrap();

    assert_eq!(
        read_bounded_no_follow(&regular, 64).unwrap(),
        b"private draft"
    );
    assert_eq!(
        fingerprint_bounded_no_follow(&regular, 64).unwrap(),
        FileFingerprint {
            byte_len: 13,
            sha256: Sha256::digest(b"private draft").into(),
        }
    );
    assert_eq!(
        fingerprint_bounded_no_follow(&regular, 12)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    symlink(&regular, &symlink_path).unwrap();
    assert!(read_bounded_no_follow(&symlink_path, 64).is_err());
    assert!(fingerprint_bounded_no_follow(&symlink_path, 64).is_err());
    std::fs::hard_link(&regular, &hard_link).unwrap();
    assert_eq!(
        read_bounded_no_follow(&regular, 64).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        read_bounded_no_follow(&hard_link, 64).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        fingerprint_bounded_no_follow(&regular, 64)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn private_directory_is_owner_only_and_never_accepts_a_link() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let root = temp_root("private-directory");
    std::fs::create_dir(&root).unwrap();
    let private = root.join("drafts");
    create_dir_all_private(&private).unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );

    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_dir_all_private(&private).unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );

    std::fs::remove_dir(&private).unwrap();
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    symlink(&elsewhere, &private).unwrap();
    assert_eq!(
        create_dir_all_private(&private).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn private_atomic_write_never_inherits_a_public_target_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = temp_root("private-atomic");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("notifications.json");
    std::fs::write(&target, "before").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

    atomic_write_private(&target, b"private notification").unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"private notification");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn private_create_new_never_replaces_and_uses_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = temp_root("private-create-new");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("legacy-note.md");

    write_new_private(&target, b"preserved source").unwrap();
    let error = write_new_private(&target, b"replacement").unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&target).unwrap(), b"preserved source");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_never_overwrites_an_existing_destination() {
    let root = temp_root("no-clobber");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source.png");
    let destination = root.join("destination.png");
    std::fs::write(&source, "new image").unwrap();
    std::fs::write(&destination, "existing image").unwrap();

    let error = copy_no_clobber(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "existing image"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_copy_removes_its_partial_destination() {
    let root = temp_root("partial-copy");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source-directory");
    let destination = root.join("destination");
    std::fs::create_dir(&source).unwrap();

    copy_no_clobber(&source, &destination).unwrap_err();

    assert!(!destination.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_stream_export_is_atomic_and_exactly_read_back() {
    let root = temp_root("checked-stream");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("notes.rmacnotes");
    let baseline = inspect_destination(&target, 1024).unwrap();

    let output = atomic_write_stream_checked(&target, baseline, 1024, |file| {
        file.write_all(b"manifest")?;
        file.write_all(b" + attachment")
    })
    .unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"manifest + attachment");
    assert_eq!(output.byte_len, 21);
    let expected_sha256: [u8; 32] = Sha256::digest(b"manifest + attachment").into();
    assert_eq!(output.sha256, expected_sha256);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_stream_export_replaces_the_exact_reviewed_file() {
    let root = temp_root("checked-stream-replace");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("note.md");
    std::fs::write(&target, b"reviewed content").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let baseline = inspect_destination(&target, 1024).unwrap();
    let baseline_debug = format!("{baseline:?}");
    let DestinationBaseline::Exact(reviewed) = baseline else {
        panic!("expected exact destination baseline");
    };
    assert!(baseline_debug.contains("<redacted>"));
    assert!(!baseline_debug.contains(&format!("{:?}", reviewed.sha256)));

    atomic_write_stream_checked(&target, baseline, 1024, |file| {
        file.write_all(b"replacement")
    })
    .unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"replacement");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_stream_export_refuses_a_destination_changed_after_review() {
    let root = temp_root("checked-stream-conflict");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("note.md");
    std::fs::write(&target, b"reviewed").unwrap();
    let baseline = inspect_destination(&target, 1024).unwrap();
    let changed_target = target.clone();

    let error = atomic_write_stream_checked(&target, baseline, 1024, move |file| {
        file.write_all(b"export candidate")?;
        std::fs::write(changed_target, b"changed elsewhere")
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&target).unwrap(), b"changed elsewhere");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_stream_export_removes_an_oversized_candidate_without_publishing() {
    let root = temp_root("checked-stream-oversized");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("note.md");
    let baseline = inspect_destination(&target, 8).unwrap();

    let error =
        atomic_write_stream_checked(&target, baseline, 8, |file| file.write_all(b"nine-byte"))
            .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_stream_copy_requires_the_complete_authoritative_fingerprint() {
    let root = temp_root("verified-private-copy");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("managed.bin");
    std::fs::write(&source, b"exact managed bytes").unwrap();
    let expected = fingerprint_bounded_no_follow(&source, 1024).unwrap();
    let mut copied = Vec::new();

    copy_verified_private_file(&source, expected, &mut copied).unwrap();
    assert_eq!(copied, b"exact managed bytes");

    std::fs::write(&source, b"substituted bytes").unwrap();
    let mut rejected = Vec::new();
    assert_eq!(
        copy_verified_private_file(&source, expected, &mut rejected),
        Err(VerifiedCopyError::Source(io::ErrorKind::InvalidData))
    );

    struct RefuseWrites;

    impl io::Write for RefuseWrites {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::WriteZero))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    std::fs::write(&source, b"exact managed bytes").unwrap();
    assert_eq!(
        copy_verified_private_file(&source, expected, &mut RefuseWrites),
        Err(VerifiedCopyError::Destination(io::ErrorKind::WriteZero))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_stream_creation_is_bounded_hashed_and_never_leaves_a_partial_file() {
    let root = temp_root("private-stream-create");
    std::fs::create_dir(&root).unwrap();
    let target = root.join("attachment.bin");
    let bytes = b"exact streamed attachment";

    let fingerprint =
        write_new_private_stream(&target, &mut bytes.as_slice(), bytes.len() as u64).unwrap();

    assert_eq!(fingerprint.byte_len, bytes.len() as u64);
    let expected_sha256: [u8; 32] = Sha256::digest(bytes).into();
    assert_eq!(fingerprint.sha256, expected_sha256);
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let oversized = root.join("oversized.bin");
    assert_eq!(
        write_new_private_stream(&oversized, &mut bytes.as_slice(), 3)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!oversized.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_failure_preserves_operation_path_and_kind() {
    let failure = Failure::from_io(
        "save settings",
        Path::new("settings.json"),
        io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
    );

    assert_eq!(failure.operation, "save settings");
    assert_eq!(failure.path, Path::new("settings.json"));
    assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
    assert!(failure.to_string().contains("save settings"));
}

/// The same [`contract::assert_backend_contract`] that
/// `fake::tests::fake_satisfies_the_shared_backend_contract` runs against
/// [`InMemoryBackend`], run here against the real host filesystem in a
/// scratch directory. Unlike the network/Bluetooth live contracts, this
/// one is safe to run unconditionally: it never touches anything but its
/// own throwaway tempdir.
#[test]
fn host_filesystem_satisfies_the_shared_backend_contract() {
    let root = temp_root("backend-contract");
    contract::assert_backend_contract(&FileSystem, &root);
    std::fs::remove_dir_all(&root).unwrap();
}
