use super::*;

fn types(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn text(value: &str) -> Draft {
    summarise(Kind::Text, "text/plain;charset=utf-8", value.as_bytes()).unwrap()
}

#[test]
fn password_manager_offers_are_recognised_by_their_hint() {
    let offer = types(&["text/plain;charset=utf-8", SENSITIVE_HINT_MIME]);
    assert!(has_sensitivity_hint(&offer));
    assert!(is_secret_hint(b"secret"));
    assert!(is_secret_hint(b"secret\n"));
    assert!(!is_secret_hint(b"public"));
    assert!(!has_sensitivity_hint(&types(&["text/plain"])));
}

#[test]
fn candidates_prefer_files_then_images_then_text() {
    let offer = types(&[
        "text/html",
        "TEXT",
        "image/jpeg",
        "text/plain;charset=utf-8",
        "image/png",
        "text/uri-list",
    ]);
    assert_eq!(
        candidates(&offer),
        vec![
            (Kind::Files, "text/uri-list".to_owned()),
            (Kind::Image, "image/png".to_owned()),
            (Kind::Text, "text/plain;charset=utf-8".to_owned()),
        ]
    );
    assert!(candidates(&types(&["application/x-private"])).is_empty());
}

#[test]
fn text_is_titled_by_its_first_nonblank_line() {
    let draft = text("\n   \n  first line  \nsecond");
    assert_eq!(draft.title, "first line");
    assert_eq!(draft.detail, "");
    assert!(summarise(Kind::Text, "text/plain", b"   \n ").is_none());
    assert!(summarise(Kind::Text, "text/plain", &[0xff, 0xfe]).is_none());
    let long = "x".repeat(PREVIEW_CHARS + 10);
    assert_eq!(text(&long).title.chars().count(), PREVIEW_CHARS + 1);
}

#[test]
fn oversized_payloads_are_skipped_not_truncated() {
    let big = vec![b'a'; MAX_TEXT_BYTES as usize + 1];
    assert!(summarise(Kind::Text, "text/plain", &big).is_none());
}

#[test]
fn file_lists_accept_only_local_files() {
    let draft = summarise(
        Kind::Files,
        "text/uri-list",
        b"# comment\r\nfile:///home/me/Documents/My%20Report.pdf\r\nfile://localhost/tmp/b.txt\r\n",
    )
    .unwrap();
    assert_eq!(draft.title, "My Report.pdf and 1 more");
    assert_eq!(draft.detail, "Documents");
    assert!(summarise(Kind::Files, "text/uri-list", b"https://example.com/").is_none());
    assert!(summarise(Kind::Files, "text/uri-list", b"file://server/share/a").is_none());
}

#[test]
fn png_images_report_their_size() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&1200_u32.to_be_bytes());
    png.extend_from_slice(&800_u32.to_be_bytes());
    let draft = summarise(Kind::Image, "image/png", &png).unwrap();
    assert_eq!(draft.title, "PNG Image");
    assert_eq!(draft.detail, "1200 × 800");
    assert!(summarise(Kind::Image, "image/x-unknown", &png).is_none());
}

#[test]
fn recopying_moves_an_entry_to_the_top_without_a_new_payload() {
    let mut history = History::default();
    let first = history.record(text("one"), 1_000);
    let second = history.record(text("two"), 2_000);
    assert!(first.new_payload && second.new_payload);
    assert_ne!(first.id, second.id);
    let again = history.record(text("one"), 3_000);
    assert_eq!(again.id, first.id);
    assert!(!again.new_payload);
    assert_eq!(history.entries()[0].id, first.id);
    assert_eq!(history.entries()[0].copied_at_ms, 3_000);
    assert_eq!(history.entries().len(), 2);
}

#[test]
fn history_is_bounded_by_count_and_evicts_the_oldest() {
    let mut history = History::default();
    let oldest = history.record(text("item 0"), 0).id;
    let mut evicted = Vec::new();
    for index in 1..=MAX_ENTRIES {
        evicted.extend(
            history
                .record(text(&format!("item {index}")), index as u64)
                .evicted,
        );
    }
    assert_eq!(history.entries().len(), MAX_ENTRIES);
    assert_eq!(evicted, vec![oldest]);
}

#[test]
fn history_expires_after_the_retention_window() {
    let mut history = History::default();
    let old = history.record(text("old"), 0).id;
    let fresh = history.record(text("fresh"), RETENTION_MS).id;
    assert_eq!(history.expire(RETENTION_MS), vec![old]);
    assert!(history.get(fresh).is_some());
    // An entry from the future (the clock went back) is dropped too.
    assert_eq!(history.expire(RETENTION_MS - 1), vec![fresh]);
}

#[test]
fn ids_are_never_reused_after_removal() {
    let mut history = History::default();
    let first = history.record(text("a"), 0).id;
    assert!(history.remove(first));
    assert!(!history.remove(first));
    let second = history.record(text("b"), 1).id;
    assert!(second > first);
    assert_eq!(history.clear(), vec![second]);
    assert!(history.entries().is_empty());
}

#[test]
fn wire_entries_round_trip_and_reject_relative_payloads() {
    let mut history = History::default();
    history.record(text("hello"), 5);
    let entry = history.entries()[0].clone();
    let wire = encode(
        &entry,
        std::path::Path::new("/run/user/1000/rmac/clipboard/item-1"),
    );
    let item = decode(wire.clone()).unwrap();
    assert_eq!(item.entry.title, "hello");
    assert_eq!(item.entry.kind, Kind::Text);
    let mut relative = wire;
    relative.7 = "item-1".into();
    assert!(decode(relative).is_none());
}

#[test]
fn subtitles_read_like_spotlight_rows() {
    let mut history = History::default();
    history.record(text("hello"), 0);
    let entry = &history.entries()[0];
    assert_eq!(subtitle(entry, 30_000), "Text · Just now");
    assert_eq!(subtitle(entry, 5 * 60_000), "Text · 5 min ago");
    assert_eq!(subtitle(entry, 3 * 60 * 60_000), "Text · 3 hr ago");
}
