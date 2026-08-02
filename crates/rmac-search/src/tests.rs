//! Focused cross-platform search contracts.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn filesystem_search_filters_hidden_entries_and_honors_cancellation() {
    let root = temporary_directory("filenames");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(root.join("nested/Quarterly Report.txt"), b"report").unwrap();
    std::fs::write(root.join(".hidden/secret-report.txt"), b"secret").unwrap();
    let cancel = AtomicBool::new(false);

    let paths = filesystem_search(&root, "REPORT", Options::new(&cancel)).unwrap();
    assert_eq!(paths, [root.join("nested/Quarterly Report.txt")]);

    let mut limited = Options::new(&cancel);
    limited.include_hidden = true;
    limited.limit = 1;
    assert_eq!(
        filesystem_search(&root, "report", limited).unwrap().len(),
        1
    );

    cancel.store(true, Ordering::Release);
    assert!(matches!(
        filesystem_search(&root, "report", Options::new(&cancel)),
        Err(Error::Cancelled)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ranked_search_orders_name_relevance_before_content_and_explains_content() {
    let root = temporary_directory("ranked");
    std::fs::create_dir_all(&root).unwrap();
    let exact = root.join("NEEDLE");
    let prefix = root.join("needle planning.txt");
    let substring = root.join("a-needle-copy.txt");
    let content = root.join("meeting notes.txt");
    std::fs::write(&exact, b"nothing").unwrap();
    std::fs::write(&prefix, b"nothing").unwrap();
    std::fs::write(&substring, b"nothing").unwrap();
    std::fs::write(
        &content,
        b"First line\n  The requested NEEDLE is in this line.  \nLast line",
    )
    .unwrap();
    let cancel = AtomicBool::new(false);

    let report = filesystem_ranked_search(&root, "needle", Options::new(&cancel)).unwrap();

    assert_eq!(
        report
            .matches
            .iter()
            .map(|result| (&result.path, result.kind))
            .collect::<Vec<_>>(),
        [
            (&exact, MatchKind::ExactName),
            (&prefix, MatchKind::NamePrefix),
            (&substring, MatchKind::NameSubstring),
            (&content, MatchKind::Content),
        ]
    );
    assert_eq!(
        report.matches[3].excerpt.as_deref(),
        Some("The requested NEEDLE is in this line.")
    );
    assert!(!report.results_truncated);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ranked_search_reports_result_traversal_and_content_bounds() {
    let root = temporary_directory("ranked-bounds");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("needle-a"), b"none").unwrap();
    std::fs::write(root.join("needle-b"), b"none").unwrap();
    std::fs::write(root.join("needle-c"), b"none").unwrap();
    let cancel = AtomicBool::new(false);
    let mut limited = Options::new(&cancel);
    limited.limit = 2;

    let report = filesystem_ranked_search(&root, "needle", limited).unwrap();
    assert_eq!(report.matches.len(), 2);
    assert!(report.results_truncated);

    let mut traversal_limited = Options::new(&cancel);
    traversal_limited.max_entries = 1;
    let report = filesystem_ranked_search(&root, "absent query", traversal_limited).unwrap();
    assert_eq!(report.scanned_entries, 1);
    assert!(report.entry_limit_reached);

    let content_root = temporary_directory("ranked-content-bound");
    std::fs::create_dir_all(&content_root).unwrap();
    std::fs::write(content_root.join("body.txt"), b"prefix needle suffix").unwrap();
    let mut content_limited = Options::new(&cancel);
    content_limited.max_content_file_bytes = 6;
    content_limited.max_total_content_bytes = 6;
    let report = filesystem_ranked_search(&content_root, "needle", content_limited).unwrap();
    assert!(report.matches.is_empty());
    assert_eq!(report.content_bytes, 6);
    assert!(report.content_partially_scanned);

    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(content_root).unwrap();
}

#[cfg(unix)]
#[test]
fn ranked_content_search_prunes_hidden_excluded_and_symbolic_link_inputs() {
    use std::os::unix::fs::symlink;

    let root = temporary_directory("ranked-safety");
    let hidden = root.join(".hidden");
    let excluded = root.join("Private");
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::create_dir_all(&excluded).unwrap();
    let visible = root.join("visible.txt");
    std::fs::write(&visible, b"contains private phrase").unwrap();
    std::fs::write(hidden.join("hidden.txt"), b"contains private phrase").unwrap();
    std::fs::write(excluded.join("excluded.txt"), b"contains private phrase").unwrap();
    symlink(&visible, root.join("alias.txt")).unwrap();
    let cancel = AtomicBool::new(false);
    let exclusions = vec![excluded];
    let mut options = Options::new(&cancel);
    options.excluded_roots = &exclusions;

    let report = filesystem_ranked_search(&root, "private phrase", options).unwrap();

    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].path, visible);
    assert_eq!(report.matches[0].kind, MatchKind::Content);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ranked_search_rejects_unbounded_or_control_queries_and_cancels() {
    let root = temporary_directory("ranked-query");
    std::fs::create_dir_all(&root).unwrap();
    let cancel = AtomicBool::new(false);

    assert!(matches!(
        filesystem_ranked_search(
            &root,
            &"x".repeat(MAX_QUERY_BYTES + 1),
            Options::new(&cancel)
        ),
        Err(Error::InvalidQuery(_))
    ));
    assert!(matches!(
        filesystem_ranked_search(&root, "line\nbreak", Options::new(&cancel)),
        Err(Error::InvalidQuery(_))
    ));
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        filesystem_ranked_search(&root, "anything", Options::new(&cancel)),
        Err(Error::Cancelled)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recent_xbel_keeps_existing_file_urls_in_modified_order() {
    let root = temporary_directory("recents");
    std::fs::create_dir_all(&root).unwrap();
    let older = root.join("older file.txt");
    let newer = root.join("newer.txt");
    let undated = root.join("undated.txt");
    std::fs::write(&older, b"older").unwrap();
    std::fs::write(&newer, b"newer").unwrap();
    std::fs::write(&undated, b"undated").unwrap();
    let older_url = url::Url::from_file_path(&older).unwrap();
    let newer_url = url::Url::from_file_path(&newer).unwrap();
    let undated_url = url::Url::from_file_path(&undated).unwrap();
    let xbel = root.join("recently-used.xbel");
    std::fs::write(
        &xbel,
        format!(
            "<?xml version=\"1.0\"?><xbel version=\"1.0\">\
                 <bookmark href=\"{older_url}\" modified=\"2026-01-01T00:00:00Z\"/>\
                 <bookmark href=\"https://example.com\" modified=\"2026-03-01T00:00:00Z\"/>\
                 <bookmark href=\"{newer_url}\" modified=\"2026-02-01T00:00:00Z\"/>\
                 <bookmark href=\"{undated_url}\"/>\
                 </xbel>"
        ),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);

    let paths = recent_from_path(&xbel, Options::new(&cancel), None).unwrap();
    assert_eq!(paths, [newer.clone(), older, undated]);
    let boundary = recent_modified_unix_ms("2026-01-15T00:00:00Z").unwrap();
    assert_eq!(
        recent_from_path(&xbel, Options::new(&cancel), Some(boundary)).unwrap(),
        [newer]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rmac_recents_precede_and_canonically_deduplicate_desktop_recents() {
    let root = temporary_directory("merged-recents");
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    let stale = root.join("stale.txt");
    std::fs::write(&first, b"one").unwrap();
    std::fs::write(&second, b"two").unwrap();
    let cancel = AtomicBool::new(false);

    let paths = merge_recent_paths(
        vec![first.clone(), stale],
        vec![first.clone(), second.clone()],
        Options::new(&cancel),
    )
    .unwrap();
    assert_eq!(paths, [first, second]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn filename_and_recent_searches_prune_excluded_roots_and_stale_records() {
    let root = temporary_directory("exclusions");
    let included = root.join("Documents");
    let excluded = root.join("Private");
    std::fs::create_dir_all(&included).unwrap();
    std::fs::create_dir_all(&excluded).unwrap();
    let visible = included.join("Visible Report.txt");
    let secret = excluded.join("Secret Report.txt");
    let stale = root.join("Deleted Report.txt");
    std::fs::write(&visible, b"visible").unwrap();
    std::fs::write(&secret, b"secret").unwrap();
    let cancel = AtomicBool::new(false);
    let exclusions = vec![excluded.clone()];
    let mut options = Options::new(&cancel);
    options.excluded_roots = &exclusions;

    assert_eq!(
        filesystem_search(&root, "report", options).unwrap(),
        std::slice::from_ref(&visible)
    );

    let xbel = root.join("recently-used.xbel");
    let visible_url = url::Url::from_file_path(&visible).unwrap();
    let secret_url = url::Url::from_file_path(&secret).unwrap();
    let stale_url = url::Url::from_file_path(&stale).unwrap();
    std::fs::write(
        &xbel,
        format!(
            "<?xml version=\"1.0\"?><xbel version=\"1.0\">\
                 <bookmark href=\"{secret_url}\" modified=\"2026-03-01T00:00:00Z\"/>\
                 <bookmark href=\"{stale_url}\" modified=\"2026-02-01T00:00:00Z\"/>\
                 <bookmark href=\"{visible_url}\" modified=\"2026-01-01T00:00:00Z\"/>\
                 </xbel>"
        ),
    )
    .unwrap();
    assert_eq!(recent_from_path(&xbel, options, None).unwrap(), [visible]);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn canonical_exclusions_cover_recent_paths_reached_through_symlinks() {
    use std::os::unix::fs::symlink;

    let root = temporary_directory("symlink-exclusion");
    let excluded = root.join("Private");
    let alias = root.join("Alias");
    std::fs::create_dir_all(&excluded).unwrap();
    let secret = excluded.join("Secret.txt");
    std::fs::write(&secret, b"secret").unwrap();
    symlink(&excluded, &alias).unwrap();
    let aliased_secret = alias.join("Secret.txt");
    assert!(aliased_secret.exists());
    assert!(is_excluded(
        &aliased_secret,
        std::slice::from_ref(&excluded)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recent_xbel_distinguishes_missing_and_malformed_stores() {
    let root = temporary_directory("recent-errors");
    std::fs::create_dir_all(&root).unwrap();
    let missing = root.join("missing.xbel");
    let malformed = root.join("malformed.xbel");
    std::fs::write(&malformed, "<xbel><bookmark").unwrap();
    let cancel = AtomicBool::new(false);

    assert!(recent_from_path(&missing, Options::new(&cancel), None)
        .unwrap()
        .is_empty());
    assert!(matches!(
        recent_from_path(&malformed, Options::new(&cancel), None),
        Err(Error::InvalidData { .. })
    ));
    assert_eq!(
        Error::RecentDocuments.to_string(),
        "recent documents are temporarily unavailable"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-search-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
