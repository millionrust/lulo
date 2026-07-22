use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rmac-wallpaper-portal-{label}-{}-{unique}",
        std::process::id()
    ))
}

fn fixture(root: &Path, name: &str, color: [u8; 4]) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let path = root.join(name);
    image::RgbaImage::from_pixel(8, 4, image::Rgba(color))
        .save(&path)
        .unwrap();
    path
}

fn request(path: &Path) -> rmac_wallpaper::portal::Request {
    rmac_wallpaper::portal::Request {
        app_id: "org.example.Photos".into(),
        uri: url::Url::from_file_path(path).unwrap().into(),
        set_on: rmac_wallpaper::portal::SetOn::Background,
        show_preview: false,
    }
}

fn importer(root: &Path) -> Importer {
    Importer::new(root.join("managed"), root.join("config/shell.json")).unwrap()
}

fn visible_entries(root: &Path) -> Vec<String> {
    let mut entries = std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != LOCK_FILE)
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[test]
fn preparation_always_produces_a_bounded_private_preview() {
    let root = root("prepare");
    let source = fixture(&root, "private-photo.png", [1, 2, 3, 255]);
    let importer = importer(&root);

    let prepared = importer.prepare(request(&source)).unwrap();

    assert_eq!(prepared.app_id(), "org.example.Photos");
    assert_eq!(prepared.image().physical_size().width, 8);
    assert_eq!(prepared.image().physical_size().height, 4);
    assert!(prepared.byte_len() > 0);
    assert_eq!(prepared.format(), rmac_wallpaper_system::ImageFormat::Png);
    let debug = format!("{prepared:?}");
    assert!(!debug.contains("private-photo"));
    assert_eq!(visible_entries(&root.join("managed")).len(), 1);

    drop(prepared);
    assert!(visible_entries(&root.join("managed")).is_empty());
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_decode_and_decline_leave_settings_and_imports_untouched() {
    let root = root("decline");
    std::fs::create_dir_all(&root).unwrap();
    let corrupt = root.join("corrupt.png");
    std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
    let importer = importer(&root);
    let error = importer.prepare(request(&corrupt)).unwrap_err();
    assert_eq!(
        error.kind,
        ErrorKind::Decode(rmac_wallpaper_image::ErrorKind::Decode)
    );
    assert!(visible_entries(&root.join("managed")).is_empty());

    let source = fixture(&root, "valid.png", [4, 5, 6, 255]);
    let prepared = importer.prepare(request(&source)).unwrap();
    assert_eq!(
        importer.finish(prepared, Consent::Decline).unwrap(),
        Outcome::Cancelled
    );
    assert!(visible_entries(&root.join("managed")).is_empty());
    assert!(!root.join("config/shell.json").exists());

    let cancelled = importer.prepare(request(&source)).unwrap();
    assert_eq!(
        importer.finish(cancelled, Consent::Cancel).unwrap(),
        Outcome::Cancelled
    );
    assert!(visible_entries(&root.join("managed")).is_empty());
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acceptance_imports_exact_reviewed_bytes_and_commits_whole_desktop_fill() {
    let root = root("accept");
    let source = fixture(&root, "selected.png", [10, 20, 30, 255]);
    let settings_path = root.join("config/shell.json");
    let store = rmac_shell_settings::ShellSettingsStore::new(settings_path.clone());
    let original = rmac_shell_settings::ShellSettings {
        dock: rmac_shell_settings::DockSettings {
            autohide: true,
            ..Default::default()
        },
        wallpaper: rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some("builtin:rmac-aurora".into()),
                fit: rmac_shell_settings::WallpaperFit::Fit,
            },
            per_output: [(
                "DP-1".into(),
                rmac_shell_settings::WallpaperSelection::default(),
            )]
            .into_iter()
            .collect(),
        },
        ..Default::default()
    };
    store.save(&original).unwrap();
    let importer = Importer::new(root.join("managed"), settings_path).unwrap();
    let prepared = importer.prepare(request(&source)).unwrap();
    let reviewed = rmac_storage::fingerprint_bounded_regular_no_follow(
        &source,
        rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
    )
    .unwrap();
    std::fs::write(&source, b"changed after preview").unwrap();

    assert_eq!(
        importer.finish(prepared, Consent::Accept).unwrap(),
        Outcome::Applied {
            cleanup_pending: false
        }
    );

    let loaded = store.load().unwrap().settings;
    assert!(loaded.dock.autohide);
    assert_eq!(
        loaded.wallpaper.default.fit,
        rmac_shell_settings::WallpaperFit::Fill
    );
    assert!(loaded.wallpaper.per_output.is_empty());
    let managed = PathBuf::from(loaded.wallpaper.default.source.unwrap());
    assert_eq!(managed.parent(), Some(root.join("managed").as_path()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(std::fs::metadata(&managed).unwrap().nlink(), 1);
    }
    assert_eq!(
        rmac_storage::fingerprint_bounded_no_follow(
            &managed,
            rmac_wallpaper_system::MAX_WALLPAPER_BYTES
        )
        .unwrap(),
        reviewed
    );
    assert_eq!(visible_entries(&root.join("managed")).len(), 1);
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_failure_removes_only_a_new_import_and_keeps_reusable_content() {
    let root = root("settings-failure");
    let source = fixture(&root, "selected.png", [40, 50, 60, 255]);
    let healthy_settings = root.join("config/shell.json");
    let importer = Importer::new(root.join("managed"), healthy_settings.clone()).unwrap();
    let first = importer.prepare(request(&source)).unwrap();
    importer.finish(first, Consent::Accept).unwrap();
    let existing = visible_entries(&root.join("managed"));
    assert_eq!(existing.len(), 1);
    drop(importer);

    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"block").unwrap();
    let failing = Importer::new(root.join("managed"), blocker.join("shell.json")).unwrap();
    let novel = fixture(&root, "novel.png", [70, 80, 90, 255]);
    let new_import = failing.prepare(request(&novel)).unwrap();
    let error = failing.finish(new_import, Consent::Accept).unwrap_err();
    assert_eq!(error.operation, Operation::ReadSettings);
    assert_eq!(visible_entries(&root.join("managed")), existing);

    let second = failing.prepare(request(&source)).unwrap();
    let error = failing.finish(second, Consent::Accept).unwrap_err();
    assert_eq!(error.operation, Operation::ReadSettings);
    assert_eq!(visible_entries(&root.join("managed")), existing);
    assert!(!format!("{error:?}").contains(root.to_string_lossy().as_ref()));
    drop(failing);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_request_cannot_cross_import_authorities() {
    let root = root("wrong-authority");
    let source = fixture(&root, "selected.png", [2, 4, 8, 255]);
    let first = Importer::new(
        root.join("first-managed"),
        root.join("first-config/shell.json"),
    )
    .unwrap();
    let second = Importer::new(
        root.join("second-managed"),
        root.join("second-config/shell.json"),
    )
    .unwrap();
    let prepared = first.prepare(request(&source)).unwrap();

    let error = second.finish(prepared, Consent::Accept).unwrap_err();

    assert_eq!(error.kind, ErrorKind::WrongAuthority);
    assert!(visible_entries(&root.join("first-managed")).is_empty());
    drop(first);
    drop(second);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn exclusive_authority_recovers_only_recognized_unreferenced_files() {
    let root = root("recovery");
    let managed = root.join("managed");
    let settings = root.join("config/shell.json");
    let importer = Importer::new(managed.clone(), settings.clone()).unwrap();
    let busy = Importer::new(managed.clone(), settings.clone()).unwrap_err();
    assert_eq!(busy.kind, ErrorKind::AuthorityBusy);
    drop(importer);

    std::fs::write(managed.join(".incoming-crashed"), b"staged").unwrap();
    std::fs::write(
        managed.join(format!("{}.png", "a".repeat(HASH_HEX_BYTES))),
        b"orphan",
    )
    .unwrap();
    std::fs::write(managed.join("leave-me.txt"), b"unknown").unwrap();
    let recovered = Importer::new(managed.clone(), settings).unwrap();
    assert_eq!(visible_entries(&managed), ["leave-me.txt"]);
    drop(recovered);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn portal_response_mapping_is_exact() {
    assert_eq!(
        Outcome::Applied {
            cleanup_pending: false
        }
        .response() as u32,
        0
    );
    assert_eq!(Outcome::Cancelled.response() as u32, 1);
    assert_eq!(PortalResponse::Other as u32, 2);
}
