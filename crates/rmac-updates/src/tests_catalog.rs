//! Lulo OS item, release notes, Automatic Updates, and menu bar status.

use super::*;

fn update(info: u32, id: &str) -> Update {
    Update::from_packagekit(info, id, "summary").unwrap()
}

fn lulo_snapshot() -> Snapshot {
    let mut snapshot = Snapshot {
        updates: vec![
            update(5, "rmac-session;0.9.1-1;amd64;lulo"),
            update(5, "rmac-apps;0.9.1-1;amd64;lulo"),
            update(5, "niri;26.04+lulo2-1;amd64;lulo"),
            update(8, "libc6;2.42-1ubuntu2;amd64;resolute-security"),
            update(5, "firefox;150.0-1;amd64;resolute-updates"),
            update(5, "zlib1g;1.3-2;amd64;resolute-updates"),
            update(9, "held;1.0-1;amd64;resolute-updates"),
        ],
        install_supported: true,
        ..Snapshot::default()
    };
    for (id, size) in [
        ("rmac-session;0.9.1-1;amd64;lulo", 30_000_000),
        ("rmac-apps;0.9.1-1;amd64;lulo", 12_000_000),
        ("niri;26.04+lulo2-1;amd64;lulo", 300_000),
    ] {
        snapshot.download_sizes.insert(id.into(), size);
    }
    snapshot
}

#[test]
fn lulo_os_packages_become_one_item_and_the_rest_other_updates() {
    let catalog = Catalog::from_snapshot(&lulo_snapshot());
    let lulo = catalog.lulo_os.as_ref().unwrap();
    assert_eq!(lulo.title(), "Lulo OS 0.9.1");
    assert_eq!(lulo.subtitle(), "0.9.1 — 42.3 MB");
    assert_eq!(lulo.packages.len(), 3);
    assert!(!lulo.security);
    assert_eq!(
        catalog
            .other
            .iter()
            .map(|update| update.name.as_str())
            .collect::<Vec<_>>(),
        ["firefox", "libc6", "zlib1g"]
    );
    assert_eq!(catalog.blocked, 1);
    assert_eq!(catalog.item_count(), 2);
    assert_eq!(
        catalog.other_summary().as_deref(),
        Some("firefox, libc6 and 1 more…")
    );
    // One unknown size makes the whole total unknown.
    assert_eq!(catalog.other_size(&lulo_snapshot()), None);
}

#[test]
fn component_only_lulo_updates_have_no_release_version() {
    let snapshot = Snapshot {
        updates: vec![update(5, "niri;26.04+lulo2-1;amd64;lulo")],
        ..Snapshot::default()
    };
    let lulo = Catalog::from_snapshot(&snapshot).lulo_os.unwrap();
    assert_eq!(lulo.title(), "Lulo OS Update");
    assert_eq!(lulo.subtitle(), "niri");
}

#[test]
fn versions_and_sizes_read_like_the_mac() {
    assert_eq!(display_version("0.9.1-1"), "0.9.1");
    assert_eq!(display_version("1:0.9.1-3"), "0.9.1");
    assert_eq!(display_version("0.9.0~beta.2-1"), "0.9.0 Beta 2");
    assert_eq!(display_version("26.04+lulo1-1"), "26.04+lulo1");
    assert_eq!(format_size(14_730_000_000), "14.73 GB");
    assert_eq!(format_size(10_000_000_000), "10 GB");
    assert_eq!(format_size(942_500_000), "942.5 MB");
    assert_eq!(format_size(42_300_000), "42.3 MB");
    assert_eq!(format_size(12_400), "12 KB");
    assert_eq!(format_size(1), "1 byte");
    assert_eq!(format_size(512), "512 bytes");
}

#[test]
fn a_selection_keeps_what_is_already_prepared_for_restart() {
    let mut snapshot = lulo_snapshot();
    snapshot.offline.prepared = vec!["firefox;150.0-1;amd64;resolute-updates".into()];
    let requested = resolve_selection(&snapshot, &[LULO_OS_ITEM.into()]).unwrap();
    let names = requested
        .iter()
        .map(|update| update.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["rmac-session", "rmac-apps", "niri", "firefox"]);
    // Blocked updates are never requested, even by ID.
    let held = resolve_selection(&snapshot, &["held;1.0-1;amd64;resolute-updates".into()]).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].name, "firefox");
    snapshot.offline.prepared.clear();
    assert_eq!(
        resolve_selection(&snapshot, &[]).unwrap_err().kind(),
        ErrorKind::Stale
    );
    snapshot.truncated = true;
    assert!(resolve_selection(&snapshot, &[LULO_OS_ITEM.into()]).is_err());
}

#[test]
fn offline_readiness_needs_every_item_prepared_and_triggered() {
    let ids = vec!["a;1;amd64;x".to_string(), "b;1;amd64;x".to_string()];
    let mut offline = OfflineStatus {
        prepared: ids.clone(),
        triggered: false,
    };
    assert!(!offline.ready(&ids));
    offline.triggered = true;
    assert!(offline.ready(&ids));
    assert!(!offline.ready(&["c;1;amd64;x".to_string()]));
    assert!(!offline.ready(&[]));
}

#[test]
fn a_plan_for_a_selection_skips_blocked_and_duplicate_updates() {
    let mut collector = PlanCollector::for_updates(vec![
        update(5, "b;1;amd64;x"),
        update(9, "held;1;amd64;x"),
        update(5, "a;1;amd64;x"),
        update(5, "a;1;amd64;x"),
    ])
    .unwrap();
    collector
        .apply(Event::Package {
            info: 11,
            package_id: "a;1;amd64;x".into(),
            summary: String::new(),
        })
        .unwrap();
    collector.apply(Event::Finished { exit: 1 }).unwrap();
    let plan = collector.finish().unwrap();
    assert_eq!(plan.requested_ids(), ["a;1;amd64;x", "b;1;amd64;x"]);
    assert!(PlanCollector::for_updates(vec![update(9, "held;1;amd64;x")]).is_err());
}

const NOTES_FIELD: &str = "Lulo-Release-Notes:\n Lulo OS 0.9.1 makes the Dock faster.\n .\n # Dock\n Stacks open instantly\n and remember their view.\n .\n # Settings\n Software Update works like the Mac's.";

fn packages_index(field: &str) -> String {
    format!(
        "Package: rmac-apps\nVersion: 0.9.1-1\nArchitecture: amd64\n\n\
         Package: rmac-session\nVersion: 0.9.1-1\nArchitecture: amd64\n\
         Description: Lulo OS session\n multi-line description\n{field}\n\
         Filename: pool/main/r/rmac/rmac-session_0.9.1-1_amd64.deb\n"
    )
}

#[test]
fn release_notes_decode_from_the_signed_index_field() {
    let notes = notes_in_index(&packages_index(NOTES_FIELD), "rmac-session", "0.9.1-1").unwrap();
    assert_eq!(
        notes.blocks,
        vec![
            NotesBlock::Paragraph("Lulo OS 0.9.1 makes the Dock faster.".into()),
            NotesBlock::Heading("Dock".into()),
            NotesBlock::Paragraph("Stacks open instantly and remember their view.".into()),
            NotesBlock::Heading("Settings".into()),
            NotesBlock::Paragraph("Software Update works like the Mac's.".into()),
        ]
    );
    // Another version, another package, or no field: no notes.
    assert!(notes_in_index(&packages_index(NOTES_FIELD), "rmac-session", "0.9.2-1").is_none());
    assert!(notes_in_index(&packages_index(NOTES_FIELD), "rmac-apps", "0.9.1-1").is_none());
    assert!(notes_in_index(&packages_index("X-Other: 1"), "rmac-session", "0.9.1-1").is_none());
}

#[test]
fn release_notes_are_read_only_from_the_lulo_archive_lists() {
    let root = std::env::temp_dir().join(format!("rmac-updates-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let lulo = "millionrust.github.io_lulo";
    let other = "ppa.example_ubuntu";
    std::fs::write(
        root.join(format!("{other}_dists_resolute_main_binary-amd64_Packages")),
        packages_index("Lulo-Release-Notes:\n Forged notes."),
    )
    .unwrap();
    std::fs::write(
        root.join(format!("{other}_dists_resolute_InRelease")),
        "Origin: example\nLabel: example\n",
    )
    .unwrap();
    let session = update(5, "rmac-session;0.9.1-1;amd64;lulo");
    assert!(lulo_release_notes(&root, &session).is_none());
    std::fs::write(
        root.join(format!("{lulo}_dists_resolute_main_binary-amd64_Packages")),
        packages_index(NOTES_FIELD),
    )
    .unwrap();
    std::fs::write(
        root.join(format!("{lulo}_dists_resolute_InRelease")),
        "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n\nOrigin: rmac\nLabel: rmac\nSuite: stable\nSHA256:\n x 1 y\n",
    )
    .unwrap();
    let notes = lulo_release_notes(&root, &session).unwrap();
    assert_eq!(
        notes.blocks[0],
        NotesBlock::Paragraph("Lulo OS 0.9.1 makes the Dock faster.".into())
    );
    // Another architecture's index is never consulted.
    assert!(lulo_release_notes(&root, &update(5, "rmac-session;0.9.1-1;arm64;lulo")).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn release_notes_refuse_control_characters() {
    assert!(ReleaseNotes::from_field("Fine line\nBad \u{1b}[31m line").is_none());
    assert!(ReleaseNotes::from_field("").is_none());
    assert!(ReleaseNotes::from_field(".\n.").is_none());
}

#[test]
fn automatic_update_switches_default_on_and_round_trip() {
    assert_eq!(AutomaticUpdates::parse(""), AutomaticUpdates::default());
    let parsed = AutomaticUpdates::parse(
        "# comment\ndownload-updates=false\ninstall-lulo-os = true\ninstall-security=maybe\nunknown=false\n",
    );
    assert!(!parsed.download);
    assert!(parsed.install_lulo_os);
    assert!(parsed.install_security);
    assert_eq!(parsed.summary(), "Off");
    assert_eq!(AutomaticUpdates::parse(&parsed.render()), parsed);
    assert_eq!(AutomaticUpdates::default().summary(), "On");

    let root = std::env::temp_dir().join(format!("rmac-updates-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let path = root.join("rmac/software-update.conf");
    assert_eq!(AutomaticUpdates::load(&path), AutomaticUpdates::default());
    parsed.save(&path).unwrap();
    assert_eq!(AutomaticUpdates::load(&path), parsed);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_menu_bar_status_counts_items_and_ignores_unknown_versions() {
    let status = UpdateStatus {
        updates: 2,
        restart_required: true,
    };
    assert_eq!(UpdateStatus::parse(&status.render()), status);
    assert_eq!(status.badge().as_deref(), Some("2 updates"));
    assert_eq!(
        UpdateStatus::parse("version=1\nupdates=1\n")
            .badge()
            .as_deref(),
        Some("1 update")
    );
    assert_eq!(
        UpdateStatus::parse("version=2\nupdates=5\n"),
        UpdateStatus::default()
    );
    assert_eq!(
        UpdateStatus::parse("version=1\nupdates=lots\n").badge(),
        None
    );
}
