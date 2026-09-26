use super::*;
use crate::file_ops::copy_item;
use crate::view::go_to_folder_controller::{pending_selection_action, PendingSelectionAction};
use crate::view::selection_controller::pathname_clipboard_text;
use crate::view::sidebar_favourites::{dedupe_absolute_directories, extra_favourite_place};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rmac-files-main-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("test directory should be created");
        Self(path)
    }

    fn new_short(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let path =
            PathBuf::from("/tmp").join(format!("rmac-{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).expect("short test directory should be created");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn checked_listing_rejects_a_replacement_at_the_same_path() {
    let root = TestDirectory::new("directory-replacement");
    let current = root.0.join("current");
    std::fs::create_dir(&current).unwrap();
    std::fs::write(current.join("old.txt"), "old").unwrap();
    let expected = directory_state::Identity::capture(&current).unwrap();
    let moved = root.0.join("moved");
    std::fs::rename(&current, &moved).unwrap();
    std::fs::create_dir(&current).unwrap();
    std::fs::write(current.join("new.txt"), "new").unwrap();

    let error = match read_entries_checked(&current, true, Some(expected)) {
        Ok(_) => panic!("the replacement must not be accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn pending_selection_waits_for_a_transfer_result_to_appear() {
    let copy = PathBuf::from("/tmp/report copy.txt");
    let original = PathBuf::from("/tmp/report.txt");
    let entries = [original.clone()];

    assert_eq!(
        pending_selection_action(&copy, entries.iter(), true),
        PendingSelectionAction::Wait,
        "a watcher reload can finish before the duplicate transfer creates its destination"
    );
    assert_eq!(
        pending_selection_action(&copy, entries.iter(), false),
        PendingSelectionAction::Discard,
        "a failed transfer must not leave a stale selection request"
    );
    assert_eq!(
        pending_selection_action(&copy, [&original, &copy].into_iter(), true),
        PendingSelectionAction::Select(1)
    );
}

#[test]
fn mount_disappearance_uses_opaque_identity_not_display_name() {
    let mount = |identity: &str, name: &str, path: &str| rmac_mounts::Mount {
        identity: identity.into(),
        name: name.into(),
        path: PathBuf::from(path),
        ejectable: true,
    };
    let previous = vec![mount("linux:42", "Drive", "/media/drive")];

    assert!(disappeared_mount_roots(
        &previous,
        &[mount("linux:42", "Renamed Drive", "/media/drive")]
    )
    .is_empty());
    assert_eq!(
        disappeared_mount_roots(&previous, &[mount("linux:99", "Drive", "/media/drive")]),
        [PathBuf::from("/media/drive")]
    );
}

#[test]
fn recursive_copy_refuses_a_destination_inside_the_source() {
    let root = TestDirectory::new("copy-descendant");
    let source = root.0.join("source");
    let child = source.join("child");
    let destination = child.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&child).unwrap();
    std::fs::write(source.join("keep"), b"source bytes").unwrap();

    let error = copy_item(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!destination.exists());
    assert_eq!(std::fs::read(source.join("keep")).unwrap(), b"source bytes");
}

#[test]
fn recursive_copy_detects_a_descendant_reached_through_a_symlink() {
    let root = TestDirectory::new("copy-symlink-descendant");
    let source = root.0.join("source");
    let child = source.join("child");
    let routed_parent = root.0.join("routed-parent");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&child).unwrap();
    std::os::unix::fs::symlink(&child, &routed_parent).unwrap();
    let destination = routed_parent.join("source");

    let error = copy_item(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!child.join("source").exists());
}

#[test]
fn recursive_copy_preserves_a_symlink_without_traversing_its_target() {
    let root = TestDirectory::new("copy-symlink");
    let target = root.0.join("target");
    let source = root.0.join("source-link");
    let destination = root.0.join("destination-link");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("private"), b"target bytes").unwrap();
    std::os::unix::fs::symlink("target", &source).unwrap();

    copy_item(&source, &destination).unwrap();

    assert!(std::fs::symlink_metadata(&destination)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::read_link(&destination).unwrap(),
        PathBuf::from("target")
    );
}

#[test]
fn recursive_copy_never_replaces_an_existing_file() {
    let root = TestDirectory::new("copy-conflict");
    let source = root.0.join("source");
    let destination = root.0.join("destination");
    std::fs::write(&source, b"source bytes").unwrap();
    std::fs::write(&destination, b"destination bytes").unwrap();

    let error = copy_item(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&destination).unwrap(), b"destination bytes");
}

#[test]
fn recursive_copy_refuses_special_files_without_opening_them() {
    let root = TestDirectory::new_short("special");
    let source = root.0.join("source.socket");
    let destination = root.0.join("destination");
    let _listener = std::os::unix::net::UnixListener::bind(&source).unwrap();

    let error = copy_item(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert!(!destination.exists());
}

#[test]
fn permanent_delete_prompt_is_explicitly_irreversible_and_path_free() {
    let (single_title, single_message) = permanent_delete_prompt(1, Some("report.txt"));
    let (multiple_title, multiple_message) = permanent_delete_prompt(3, None);

    assert!(single_title.contains("“report.txt”"));
    assert!(single_message.contains("can’t undo"));
    assert!(!single_title.contains("/home/"));
    assert!(!single_message.contains("/home/"));
    assert_eq!(
        multiple_title,
        "Are you sure you want to delete these 3 items?"
    );
    assert_eq!(
        multiple_message,
        "These items will be deleted immediately. You can’t undo this action."
    );
}

#[test]
fn permanent_delete_dialog_name_replaces_controls_and_bounds_length() {
    assert_eq!(sanitize_dialog_name("line\nbreak"), "line\u{fffd}break");
    let long = "a".repeat(121);
    let sanitized = sanitize_dialog_name(&long);

    assert_eq!(sanitized.chars().count(), 121);
    assert!(sanitized.ends_with('…'));
}

#[test]
fn ranked_search_summary_discloses_every_incomplete_scope() {
    let report = rmac_search::SearchReport {
        matches: vec![rmac_search::SearchMatch {
            path: PathBuf::from("/scope/result"),
            kind: rmac_search::MatchKind::Content,
            excerpt: Some("match".to_string()),
        }],
        scanned_entries: 100_000,
        content_bytes: 64 * 1024 * 1024,
        results_truncated: true,
        entry_limit_reached: true,
        content_partially_scanned: true,
        skipped_errors: 2,
    };

    let summary = ranked_search_summary(&report, 1);

    assert!(summary.contains("1 result"));
    assert!(summary.contains("100000 items checked"));
    assert!(summary.contains("result limit reached"));
    assert!(summary.contains("item limit reached"));
    assert!(summary.contains("some content not searched"));
    assert!(summary.contains("2 unavailable"));
}

#[test]
fn ranked_search_entry_presents_match_reason_without_private_absolute_path() {
    let root = TestDirectory::new("search-presentation");
    let folder = root.0.join("Documents");
    let path = folder.join("notes.txt");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&path, b"needle").unwrap();

    let entry = search_entry_for(
        &root.0,
        rmac_search::SearchMatch {
            path,
            kind: rmac_search::MatchKind::Content,
            excerpt: Some("the needle line".to_string()),
        },
    )
    .unwrap();

    let detail = entry.search_detail.unwrap().to_string();
    assert_eq!(detail, "Contents · the needle line");
    assert!(!detail.contains(root.0.to_string_lossy().as_ref()));
}

#[test]
fn generic_documents_carry_a_short_extension_badge() {
    assert_eq!(document_badge("Archive.tar.gz").as_deref(), Some("GZ"));
    assert_eq!(document_badge("notes.txt").as_deref(), Some("TXT"));
    assert_eq!(document_badge("README"), None);
    assert_eq!(document_badge(".bashrc"), None);
    assert_eq!(document_badge("draft.markdown"), None);
    assert_eq!(document_badge("weird.t-x"), None);
}

#[test]
fn artwork_rasters_follow_the_drawn_size_and_are_embedded() {
    assert_eq!(item_artwork_path(true, 16.0), "icons/folder-artwork-16.svg");
    assert_eq!(item_artwork_path(true, 64.0), "icons/folder-artwork-96.svg");
    assert_eq!(
        item_artwork_path(true, 300.0),
        "icons/folder-artwork-320.svg"
    );
    assert_eq!(
        item_artwork_path(false, 13.0),
        "icons/document-artwork-16.svg"
    );
    assert_eq!(
        item_artwork_path(false, 128.0),
        "icons/document-artwork-96.svg"
    );
    for path in [
        "icons/folder-artwork-16.svg",
        "icons/folder-artwork-96.svg",
        "icons/folder-artwork-320.svg",
        "icons/document-artwork-16.svg",
        "icons/document-artwork-96.svg",
        "icons/document-artwork-320.svg",
    ] {
        assert!(
            CombinedAssets.load(path).ok().flatten().is_some(),
            "{path} should be embedded"
        );
    }
}

#[test]
fn column_index_for_selection_finds_the_column_holding_the_entry() {
    let root = TestDirectory::new("column-index");
    let child = root.0.join("child");
    std::fs::create_dir(&child).unwrap();
    let grandchild_file = child.join("leaf.txt");
    std::fs::write(&grandchild_file, b"leaf").unwrap();

    let col_stack = vec![root.0.clone(), child.clone()];
    let leaf_entry = entry_for(&grandchild_file).unwrap();
    let child_entry = entry_for(&child).unwrap();

    assert_eq!(
        column_index_for_selection(&col_stack, &leaf_entry),
        Some(1),
        "the file's column is the one showing its parent directory"
    );
    assert_eq!(
        column_index_for_selection(&col_stack, &child_entry),
        Some(0),
        "the folder itself is shown one column to the left of its own contents"
    );

    let unrelated = TestDirectory::new("column-index-unrelated");
    let stray_file = unrelated.0.join("stray.txt");
    std::fs::write(&stray_file, b"stray").unwrap();
    let stray_entry = entry_for(&stray_file).unwrap();
    assert_eq!(column_index_for_selection(&col_stack, &stray_entry), None);
}

#[test]
fn column_vertical_target_moves_within_the_column_and_clamps_at_the_ends() {
    let root = TestDirectory::new("column-vertical");
    let mut names = ["a.txt", "b.txt", "c.txt"];
    for name in names.iter() {
        std::fs::write(root.0.join(name), b"x").unwrap();
    }
    let mut entries = read_entries(&root.0, true);
    sort_entries(&mut entries, SortKey::Name, true);
    names.sort_unstable();
    assert_eq!(
        entries
            .iter()
            .map(|e| e.name.to_string())
            .collect::<Vec<_>>(),
        names
    );

    // No current selection: the first row is picked, whether stepping up or down.
    assert_eq!(
        column_vertical_target(&entries, None, 1).map(|e| e.name.to_string()),
        Some("a.txt".to_string())
    );
    assert_eq!(
        column_vertical_target(&entries, None, -1).map(|e| e.name.to_string()),
        Some("a.txt".to_string())
    );

    // Down moves forward one row.
    let first = entries[0].path.clone();
    assert_eq!(
        column_vertical_target(&entries, Some(&first), 1).map(|e| e.name.to_string()),
        Some("b.txt".to_string())
    );
    // Up from the first row clamps to itself.
    assert_eq!(
        column_vertical_target(&entries, Some(&first), -1).map(|e| e.name.to_string()),
        Some("a.txt".to_string())
    );
    // Down from the last row clamps to itself.
    let last = entries[2].path.clone();
    assert_eq!(
        column_vertical_target(&entries, Some(&last), 1).map(|e| e.name.to_string()),
        Some("c.txt".to_string())
    );
    // Up from the last row moves back one.
    assert_eq!(
        column_vertical_target(&entries, Some(&last), -1).map(|e| e.name.to_string()),
        Some("b.txt".to_string())
    );

    // An empty column has no target at all.
    assert!(column_vertical_target(&[], None, 1).is_none());
}

#[test]
fn pathname_clipboard_text_is_one_absolute_path_per_line() {
    assert_eq!(pathname_clipboard_text(&[]), "");
    assert_eq!(
        pathname_clipboard_text(&[PathBuf::from("/tmp/one.txt")]),
        "/tmp/one.txt"
    );
    assert_eq!(
        pathname_clipboard_text(&[PathBuf::from("/tmp/one.txt"), PathBuf::from("/tmp/two.txt"),]),
        "/tmp/one.txt\n/tmp/two.txt"
    );
}

#[test]
fn sidebar_favourites_drop_relative_and_duplicate_paths_and_cap_the_list() {
    let paths = vec![
        PathBuf::from("relative/not-a-favourite"),
        PathBuf::from("/home/jake/Projects"),
        PathBuf::from("/home/jake/Projects"),
        PathBuf::from("/home/jake/Music"),
    ];

    assert_eq!(
        dedupe_absolute_directories(paths),
        [
            PathBuf::from("/home/jake/Projects"),
            PathBuf::from("/home/jake/Music"),
        ]
    );

    let too_many: Vec<PathBuf> = (0..40)
        .map(|index| PathBuf::from(format!("/home/jake/folder-{index}")))
        .collect();
    assert_eq!(dedupe_absolute_directories(too_many).len(), 32);
}

#[test]
fn extra_favourite_place_is_named_after_the_folder_not_its_full_path() {
    let place = extra_favourite_place(Path::new("/home/jake/Projects/rmac"));
    assert_eq!(place.name.as_ref(), "rmac");
    assert_eq!(place.path, PathBuf::from("/home/jake/Projects/rmac"));
    assert!(place.kind == PlaceKind::Item);
}

#[test]
fn trash_deletion_dates_read_like_the_rest_of_the_list() {
    let label = trash_updates::deletion_label("2024-02-28T23:05:00");
    assert!(!label.contains('T'), "{label}");
    assert!(label.contains("2024") || label.contains("Feb"), "{label}");
    assert_eq!(trash_updates::deletion_label("not a date"), "not a date");
}
