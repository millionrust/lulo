use super::*;
use std::time::SystemTime;

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-recent-documents-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn store(root: &Path) -> Store {
    Store {
        path: root.join("state/recent-documents.json"),
    }
}

#[test]
fn records_refreshes_and_bounds_private_documents() {
    let root = root("record");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let first = root.join("first file.txt");
    let second = root.join("second.txt");
    std::fs::write(&first, b"one").unwrap();
    std::fs::write(&second, b"two").unwrap();

    assert_eq!(store.record(&first).unwrap(), RecordOutcome::Added);
    assert_eq!(store.record(&second).unwrap(), RecordOutcome::Added);
    assert_eq!(store.record(&first).unwrap(), RecordOutcome::Refreshed);
    let snapshot = store.load().unwrap();
    assert_eq!(
        snapshot.paths,
        [
            first.canonicalize().unwrap(),
            second.canonicalize().unwrap()
        ]
    );
    assert_eq!(snapshot.recovery, Recovery::None);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(store.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_current_recovers_from_last_good_then_empty() {
    let root = root("recovery");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let document = root.join("document.txt");
    std::fs::write(&document, b"text").unwrap();
    store.record(&document).unwrap();
    std::fs::write(&store.path, b"private malformed data").unwrap();
    let recovered = store.load().unwrap();
    assert_eq!(recovered.recovery, Recovery::LastGood);
    assert_eq!(recovered.paths, [document.canonicalize().unwrap()]);

    std::fs::write(store.backup_path(), b"also malformed").unwrap();
    let empty = store.load().unwrap();
    assert_eq!(empty.recovery, Recovery::Empty);
    assert!(empty.paths.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn unsafe_paths_and_diagnostics_never_disclose_documents() {
    use std::os::unix::fs::symlink;

    let root = root("unsafe-private-name-8472");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let directory = root.join("folder");
    let target = root.join("private-document-8472.txt");
    let link = root.join("document-link");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&target, b"text").unwrap();
    symlink(&target, &link).unwrap();

    assert_eq!(
        store.record(&directory).unwrap_err().kind,
        ErrorKind::UnsafeDocument
    );
    assert_eq!(
        store.record(Path::new("relative.txt")).unwrap_err().kind,
        ErrorKind::UnsafeDocument
    );
    assert_eq!(
        store.record(&link).unwrap_err().kind,
        ErrorKind::UnsafeDocument
    );
    let error = Store { path: link }.load().unwrap_err();
    let diagnostics = format!("{error:?} {error}");
    assert!(!diagnostics.contains("private-document"));
    assert!(!diagnostics.contains("8472"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clear_is_idempotent_and_durable() {
    let root = root("clear");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let document = root.join("document.txt");
    std::fs::write(&document, b"text").unwrap();
    store.record(&document).unwrap();

    assert!(store.clear().unwrap());
    assert!(!store.clear().unwrap());
    assert!(store.load().unwrap().paths.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_writers_preserve_every_document() {
    use std::sync::{Arc, Barrier};

    let root = root("concurrent");
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(store(&root));
    let barrier = Arc::new(Barrier::new(8));
    let mut writers = Vec::new();
    for index in 0..8 {
        let path = root.join(format!("document-{index}.txt"));
        std::fs::write(&path, b"text").unwrap();
        let store = store.clone();
        let barrier = barrier.clone();
        writers.push(std::thread::spawn(move || {
            barrier.wait();
            store.record(&path)
        }));
    }
    for writer in writers {
        assert_eq!(writer.join().unwrap().unwrap(), RecordOutcome::Added);
    }
    assert_eq!(store.load().unwrap().paths.len(), 8);
    std::fs::remove_dir_all(root).unwrap();
}
