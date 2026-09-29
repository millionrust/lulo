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
fn clear_removes_records_and_advances_the_desktop_boundary() {
    let root = root("clear");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let document = root.join("document.txt");
    std::fs::write(&document, b"text").unwrap();
    store.record(&document).unwrap();

    assert_eq!(store.clear().unwrap(), 1);
    let first = store.load().unwrap();
    assert!(first.paths.is_empty());
    let first_boundary = first.cleared_before_unix_ms.unwrap();
    assert_eq!(store.clear().unwrap(), 0);
    let second = store.load().unwrap();
    assert!(second.paths.is_empty());
    assert!(second.cleared_before_unix_ms.unwrap() > first_boundary);
    assert_eq!(store.record(&document).unwrap(), RecordOutcome::Added);
    let repopulated = store.load().unwrap();
    assert_eq!(repopulated.paths, [document.canonicalize().unwrap()]);
    assert_eq!(
        repopulated.cleared_before_unix_ms,
        second.cleared_before_unix_ms
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn app_scoped_recording_filters_the_merged_view() {
    let root = root("app-scoped");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let editor_doc = root.join("notes.txt");
    let preview_doc = root.join("scan.png");
    let untagged_doc = root.join("shared.txt");
    std::fs::write(&editor_doc, b"one").unwrap();
    std::fs::write(&preview_doc, b"two").unwrap();
    std::fs::write(&untagged_doc, b"three").unwrap();

    assert_eq!(
        store.record_for_app(&editor_doc, "text_editor").unwrap(),
        RecordOutcome::Added
    );
    assert_eq!(
        store.record_for_app(&preview_doc, "preview").unwrap(),
        RecordOutcome::Added
    );
    assert_eq!(store.record(&untagged_doc).unwrap(), RecordOutcome::Added);

    assert_eq!(
        store.load_for_app("text_editor").unwrap(),
        [editor_doc.canonicalize().unwrap()]
    );
    assert_eq!(
        store.load_for_app("preview").unwrap(),
        [preview_doc.canonicalize().unwrap()]
    );
    assert!(store.load_for_app("nobody").unwrap().is_empty());
    // The merged view every other consumer reads still sees all three.
    assert_eq!(store.load().unwrap().paths.len(), 3);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn clear_for_app_only_removes_its_own_entries() {
    let root = root("app-clear");
    std::fs::create_dir_all(&root).unwrap();
    let store = store(&root);
    let editor_doc = root.join("notes.txt");
    let preview_doc = root.join("scan.png");
    std::fs::write(&editor_doc, b"one").unwrap();
    std::fs::write(&preview_doc, b"two").unwrap();
    store.record_for_app(&editor_doc, "text_editor").unwrap();
    store.record_for_app(&preview_doc, "preview").unwrap();

    assert_eq!(store.clear_for_app("text_editor").unwrap(), 1);
    assert!(store.load_for_app("text_editor").unwrap().is_empty());
    assert_eq!(
        store.load_for_app("preview").unwrap(),
        [preview_doc.canonicalize().unwrap()]
    );
    // Clearing again removes nothing more, and does not disturb Preview's.
    assert_eq!(store.clear_for_app("text_editor").unwrap(), 0);
    assert_eq!(store.load().unwrap().paths.len(), 1);
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
