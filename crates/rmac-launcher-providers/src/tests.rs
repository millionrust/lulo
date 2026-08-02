//! Focused launcher provider contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn application(id: &str, name: &str) -> rmac_apps::Application {
    rmac_apps::Application {
        id: id.into(),
        name: name.into(),
        generic_name: None,
        keywords: Vec::new(),
        source: PathBuf::from(format!("/apps/{id}")),
        icon: None,
        categories: vec!["Utility".into()],
        mime_types: Vec::new(),
        launch: rmac_apps::LaunchSpec::Command {
            program: id.into(),
            args: vec!["--new".into()],
            working_dir: None,
            terminal: false,
        },
        actions: Vec::new(),
    }
}

#[test]
fn application_provider_preserves_exact_launch_spec() {
    let mut terminal = application("terminal.desktop", "Terminal");
    terminal.generic_name = Some("Console".into());
    terminal.keywords = vec!["shell".into(), "command line".into()];
    terminal.actions.push(rmac_apps::DesktopAction {
        id: "New-Window".into(),
        name: "Fresh Window".into(),
        icon: None,
        launch: terminal.launch.clone(),
    });
    let provider = ApplicationProvider::new(vec![terminal]);
    let results = provider
        .search("term", &Cancellation::default())
        .expect("search succeeds");
    assert_eq!(results.len(), 1);
    assert!(matches!(
        &results[0].primary,
        Action::LaunchApplication { app_id, spec: rmac_apps::LaunchSpec::Command { args, .. } }
            if app_id == "terminal.desktop" && args == &["--new"]
    ));
    assert!(matches!(
        &results[0].alternate,
        Some(Action::RevealApplication { source })
            if source == Path::new("/apps/terminal.desktop")
    ));
    for metadata_query in ["console", "shell", "fresh window"] {
        assert_eq!(
            provider
                .search(metadata_query, &Cancellation::default())
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn application_catalog_replacement_is_atomic_shared_and_revision_stable() {
    let provider = ApplicationProvider::new(vec![application("terminal.desktop", "Terminal")]);
    let clone = provider.clone();
    let revision = provider.revision();
    assert!(revision > 0);
    assert!(!provider.replace_catalog(vec![application("terminal.desktop", "Terminal")]));
    assert_eq!(provider.revision(), revision);

    assert!(provider.replace_catalog(vec![
        application("notes.desktop", "Notes"),
        application("notes.desktop", "Duplicate Notes"),
        application("", "Invalid"),
    ]));
    assert!(provider.revision() > revision);
    let results = clone
        .search("", &Cancellation::default())
        .expect("shared catalog search succeeds");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.local, "notes.desktop");
    assert_eq!(results[0].title, "Notes");
}

#[test]
fn settings_provider_matches_keywords_and_deduplicates_panes() {
    let provider = SettingsProvider::new(vec![
        SettingEntry {
            pane_id: "sound".into(),
            title: "Sound".into(),
            subtitle: Some("Output and input".into()),
            keywords: vec!["volume".into()],
        },
        SettingEntry {
            pane_id: "sound".into(),
            title: "Duplicate".into(),
            subtitle: None,
            keywords: Vec::new(),
        },
    ]);
    let results = provider
        .search("volume", &Cancellation::default())
        .expect("search succeeds");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id.local, "sound");
}

#[test]
fn system_settings_catalog_has_stable_unique_panes_and_linux_synonyms() {
    let entries = system_settings_entries();
    assert_eq!(entries.len(), 24);
    let unique: BTreeSet<_> = entries.iter().map(|entry| &entry.pane_id).collect();
    assert_eq!(unique.len(), entries.len());
    let provider = SettingsProvider::system_settings();
    let touchpad = provider
        .search("touchpad", &Cancellation::default())
        .expect("settings search succeeds");
    assert_eq!(touchpad[0].id.local, "trackpad");
    let firewall = provider
        .search("firewall", &Cancellation::default())
        .expect("settings search succeeds");
    assert_eq!(firewall[0].id.local, "privacy-security");
    assert!(entries.iter().all(|entry| entry.pane_id != "assistant"));
    assert!(entries.iter().all(|entry| entry.pane_id != "screen-time"));
    assert!(entries.iter().all(|entry| !entry.pane_id.contains(' ')));
}

#[derive(Default)]
struct FakeFileSearch {
    paths: Vec<PathBuf>,
}

impl FileSearch for FakeFileSearch {
    fn filenames(
        &self,
        _: &Path,
        _: &str,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error> {
        if options.cancel.load(Ordering::Acquire) {
            return Err(rmac_search::Error::Cancelled);
        }
        Ok(self.paths.clone())
    }

    fn recents(
        &self,
        options: rmac_search::Options<'_>,
    ) -> Result<Vec<PathBuf>, rmac_search::Error> {
        if options.cancel.load(Ordering::Acquire) {
            return Err(rmac_search::Error::Cancelled);
        }
        Ok(self.paths.clone())
    }
}

#[test]
fn file_provider_is_private_scoped_deduplicated_and_revealable() {
    let root = temporary_directory("scope");
    let excluded = root.join("Private");
    std::fs::create_dir_all(&excluded).expect("create test directories");
    let report = root.join("Report.txt");
    let secret = excluded.join("Secret Report.txt");
    std::fs::write(&report, b"report").expect("write report");
    std::fs::write(&secret, b"secret").expect("write secret");
    let settings = rmac_shell_settings::SpotlightSettings {
        excluded_paths: vec![excluded.to_string_lossy().into_owned()],
        include_removable_mounts: false,
    };
    let provider = FileProvider::scoped(
        root.clone(),
        &settings,
        FakeFileSearch {
            paths: vec![
                report.clone(),
                report,
                secret,
                root.join("stale-report.txt"),
                PathBuf::from("relative.txt"),
            ],
        },
    )
    .expect("scope is valid");
    assert!(provider.descriptor().privacy.private_content);
    let results = provider
        .search("report", &Cancellation::default())
        .expect("search succeeds");
    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0].alternate,
        Some(Action::RevealFile { .. })
    ));
    std::fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn recent_documents_outside_the_root_require_removable_mount_opt_in() {
    let root = temporary_directory("home");
    let external = temporary_directory("external");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&external).expect("create external root");
    let document = external.join("External.txt");
    std::fs::write(&document, b"external").expect("write external file");

    let default_provider = FileProvider::new(
        root.clone(),
        FakeFileSearch {
            paths: vec![document.clone()],
        },
    );
    assert!(default_provider
        .search("", &Cancellation::default())
        .expect("recent search succeeds")
        .is_empty());

    let opted_in = FileProvider::scoped(
        root.clone(),
        &rmac_shell_settings::SpotlightSettings {
            excluded_paths: Vec::new(),
            include_removable_mounts: true,
        },
        FakeFileSearch {
            paths: vec![document],
        },
    )
    .expect("scope is valid");
    assert_eq!(
        opted_in
            .search("", &Cancellation::default())
            .expect("recent search succeeds")
            .len(),
        1
    );
    std::fs::remove_dir_all(root).expect("remove root");
    std::fs::remove_dir_all(external).expect("remove external root");
}

#[test]
fn provider_execution_requires_exact_admission_and_honors_cancellation() {
    let provider = CalculatorProvider;
    let descriptor = provider.descriptor();
    let mut session = rmac_launcher::Session::default();
    let admitted = session.begin("2+2", vec![descriptor]);
    assert!(execute(&admitted, &provider).is_some());

    let unadmitted = session.begin("2+2", Vec::new());
    assert!(execute(&unadmitted, &provider).is_none());
    admitted.cancellation.cancel();
    assert!(execute(&admitted, &provider).is_none());
}

#[test]
fn calculator_is_bounded_and_respects_precedence_parentheses_and_unary() {
    let provider = CalculatorProvider;
    for (expression, expected) in [
        ("2 + 3 * 4", "14"),
        ("(2 + 3) * 4", "20"),
        ("-5 / 2", "-2.5"),
    ] {
        let results = provider
            .search(expression, &Cancellation::default())
            .expect("calculator succeeds");
        assert_eq!(results[0].title, expected);
    }
    let too_long = "1+".repeat(200);
    for expression in ["1 / 0", "2 +", "hello", too_long.as_str()] {
        assert!(provider
            .search(expression, &Cancellation::default())
            .expect("invalid expression is not an error")
            .is_empty());
    }
}

#[test]
fn cancellation_flag_is_compatible_with_search_options() {
    let cancellation = Cancellation::default();
    let options = rmac_search::Options::new(cancellation.flag());
    assert!(!options.cancel.load(Ordering::Acquire));
    cancellation.cancel();
    assert!(options.cancel.load(Ordering::Acquire));
    let _: &AtomicBool = cancellation.flag();
}

#[test]
fn provider_descriptors_use_distinct_stable_ids() {
    let descriptors = [
        ApplicationProvider::default().descriptor(),
        SettingsProvider::default().descriptor(),
        FileProvider::system(PathBuf::from("/home/alex")).descriptor(),
        CalculatorProvider.descriptor(),
    ];
    let unique: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| (descriptor.id.clone(), descriptor.category))
        .collect();
    assert_eq!(unique.len(), descriptors.len());
}

fn temporary_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-launcher-providers-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock follows epoch")
            .as_nanos()
    ))
}
