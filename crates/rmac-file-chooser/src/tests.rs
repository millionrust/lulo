use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use rmac_finder::listing::Item;

use crate::browser::{Browser, Location, Policy};
use crate::filter::{glob_match, CompiledFilter, MimeDatabase};
use crate::goto;
use crate::metrics;
use crate::outcome::{
    confirm_open, numbered_name, results, save_files_targets, save_target, Rejection, Selection,
};
use crate::parent::ParentWindow;
use crate::request::{Filter, Mode, RawOptions, Request, Rule};
use crate::service::Broker;

fn scratch(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("rmac-file-chooser-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn item(name: &str, is_dir: bool) -> Item {
    Item {
        name: name.to_owned(),
        path: PathBuf::from("/data").join(name),
        is_dir,
        size_bytes: 0,
        mtime: SystemTime::UNIX_EPOCH,
        kind: String::new(),
    }
}

fn open_policy(filter: CompiledFilter) -> Policy {
    Policy {
        mode: Mode::Open,
        directory: false,
        multiple: true,
        filter,
    }
}

#[test]
fn request_decodes_filters_choices_and_paths() {
    let options = RawOptions {
        accept_label: Some("_Open".into()),
        multiple: Some(true),
        filters: Some(vec![
            ("Images".into(), vec![(1, "image/*".into())]),
            ("Text".into(), vec![(0, "*.txt".into())]),
        ]),
        current_filter: Some(("Text".into(), vec![(0, "*.txt".into())])),
        choices: Some(vec![
            (
                "encoding".into(),
                "Encoding".into(),
                vec![
                    ("utf8".into(), "Unicode".into()),
                    ("latin1".into(), "Western".into()),
                ],
                "missing".into(),
            ),
            ("reencode".into(), "Re-encode".into(), vec![], "true".into()),
        ]),
        current_folder: Some(b"/tmp\0".to_vec()),
        ..RawOptions::default()
    };
    let request = Request::from_wire(
        Mode::Open,
        "org.example.App".into(),
        "wayland:abc123",
        "Open File".into(),
        options,
    )
    .unwrap();
    assert_eq!(request.accept_label(), "Open");
    assert!(request.multiple);
    assert_eq!(request.current_filter, Some(1));
    assert_eq!(request.choices[0].selected, "utf8");
    assert!(request.choices[1].is_checkbox() && request.choices[1].checked());
    assert_eq!(request.current_folder, Some(PathBuf::from("/tmp")));
    assert_eq!(request.parent, ParentWindow::Wayland("abc123".into()));
}

#[test]
fn request_rejects_bad_wire_values() {
    let relative = RawOptions {
        current_folder: Some(b"relative/path".to_vec()),
        ..RawOptions::default()
    };
    assert!(Request::from_wire(Mode::Open, String::new(), "", String::new(), relative).is_err());
    let bad_kind = RawOptions {
        filters: Some(vec![("X".into(), vec![(7, "*".into())])]),
        ..RawOptions::default()
    };
    assert!(Request::from_wire(Mode::Open, String::new(), "", String::new(), bad_kind).is_err());
    let bad_file = RawOptions {
        files: Some(vec![b"..".to_vec()]),
        ..RawOptions::default()
    };
    assert!(
        Request::from_wire(Mode::SaveFiles, String::new(), "", String::new(), bad_file).is_err()
    );
}

#[test]
fn save_request_names_and_folders() {
    let options = RawOptions {
        current_file: Some(b"/home/jake/Notes/plan.txt\0".to_vec()),
        multiple: Some(true),
        directory: Some(true),
        ..RawOptions::default()
    };
    let request =
        Request::from_wire(Mode::Save, String::new(), "", "Save".into(), options).unwrap();
    assert_eq!(request.initial_name(), "plan.txt");
    assert_eq!(
        request.initial_folder(),
        Some(PathBuf::from("/home/jake/Notes"))
    );
    assert!(!request.multiple && !request.directory);
    assert_eq!(request.accept_label(), "Save");

    let files = RawOptions {
        files: Some(vec![b"/tmp/a.txt\0".to_vec(), b"b.txt".to_vec()]),
        ..RawOptions::default()
    };
    let request =
        Request::from_wire(Mode::SaveFiles, String::new(), "", String::new(), files).unwrap();
    assert_eq!(request.files, ["a.txt", "b.txt"]);
}

#[test]
fn parent_window_handles_are_bounded() {
    assert_eq!(ParentWindow::parse("x11:1a2b"), ParentWindow::X11(0x1a2b));
    assert_eq!(ParentWindow::parse("x11:0"), ParentWindow::None);
    assert_eq!(ParentWindow::parse("wayland:"), ParentWindow::None);
    assert_eq!(ParentWindow::parse(""), ParentWindow::None);
    assert_eq!(
        ParentWindow::parse(&"wayland:x".repeat(64)),
        ParentWindow::None
    );
}

#[test]
fn globs_match_like_the_shell() {
    let m = |pattern: &str, text: &str| {
        glob_match(
            &pattern.chars().collect::<Vec<_>>(),
            &text.chars().collect::<Vec<_>>(),
        )
    };
    assert!(m("*.txt", "notes.txt"));
    assert!(!m("*.txt", "notes.txt.bak"));
    assert!(m("readme*", "readme.md"));
    assert!(m("file?.[ch]", "file1.c"));
    assert!(!m("file?.[!ch]", "file1.c"));
    assert!(m("[a-c]*", "beta"));
}

#[test]
fn mime_filters_expand_to_globs_and_subclasses() {
    let mut database = MimeDatabase::default();
    database.add_globs2(
        "# comment\n50:image/png:*.png\n50:image/jpeg:*.jpg\n50:text/plain:*.txt\n50:text/x-csrc:*.c\n50:application/x-compressed-tar:*.tar.gz\n",
    );
    database.add_subclasses("text/x-csrc text/plain\n");
    let text = CompiledFilter::compile(
        &Filter {
            name: "Text".into(),
            rules: vec![Rule::Mime("text/plain".into())],
        },
        &database,
    );
    assert!(text.accepts("NOTES.TXT"));
    assert!(text.accepts("main.c"));
    assert!(!text.accepts("photo.png"));
    let images = CompiledFilter::compile(
        &Filter {
            name: "Images".into(),
            rules: vec![Rule::Mime("image/*".into())],
        },
        &database,
    );
    assert!(images.accepts("a.jpg") && images.accepts("b.png") && !images.accepts("c.txt"));
    let archives = CompiledFilter::compile(
        &Filter {
            name: "Archives".into(),
            rules: vec![Rule::Mime("application/x-compressed-tar".into())],
        },
        &database,
    );
    assert!(archives.accepts("backup.tar.gz"));
    assert!(CompiledFilter::everything().accepts("anything"));
}

#[test]
fn open_confirmation_only_returns_existing_items_of_the_right_kind() {
    let root = scratch("open");
    let file = root.join("a.txt");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        confirm_open(&[file.clone()], false, false),
        Ok(vec![file.clone()])
    );
    assert_eq!(
        confirm_open(&[root.clone()], false, false),
        Err(Rejection::WrongKind)
    );
    assert_eq!(
        confirm_open(&[root.clone()], true, false),
        Ok(vec![root.clone()])
    );
    assert_eq!(
        confirm_open(&[file.clone(), root.join("b")], false, false),
        Err(Rejection::TooMany)
    );
    assert_eq!(
        confirm_open(&[root.join("gone")], false, false),
        Err(Rejection::Missing)
    );
    assert_eq!(
        confirm_open(&[root.join("x/../a.txt")], false, false),
        Err(Rejection::NotAbsolute)
    );
    assert_eq!(confirm_open(&[], false, true), Err(Rejection::Empty));
    let selection = Selection {
        paths: vec![root.join("a b.txt")],
        choices: vec![("k".into(), "v".into())],
        current_filter: Some(0),
    };
    let wire = results(&selection).unwrap();
    assert!(wire.uris[0].starts_with("file:///") && wire.uris[0].ends_with("a%20b.txt"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_targets_are_one_name_inside_an_existing_folder() {
    let root = scratch("save");
    assert_eq!(save_target(&root, "doc.txt"), Ok(root.join("doc.txt")));
    assert_eq!(save_target(&root, "a/b"), Err(Rejection::InvalidName));
    assert_eq!(save_target(&root, ".."), Err(Rejection::InvalidName));
    assert_eq!(
        save_target(&root.join("missing"), "x"),
        Err(Rejection::NotAFolder)
    );
    std::fs::write(root.join("a.txt"), b"x").unwrap();
    let targets = save_files_targets(&root, &["a.txt".into(), "b".into(), "b".into()]).unwrap();
    assert_eq!(
        targets,
        [root.join("a 2.txt"), root.join("b"), root.join("b 2")]
    );
    assert_eq!(numbered_name(".bashrc", 3), ".bashrc 3");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_history_and_enclosing_folder() {
    let mut browser = Browser::new(PathBuf::from("/home/jake"));
    assert!(browser.navigate(Location::Folder("/home/jake/Documents".into())));
    assert!(!browser.navigate(Location::Folder("/home/jake/Documents".into())));
    assert!(browser.go_enclosing());
    assert_eq!(browser.location(), &Location::Folder("/home/jake".into()));
    assert!(browser.go_back());
    assert_eq!(
        browser.location().folder(),
        Some(Path::new("/home/jake/Documents"))
    );
    assert!(browser.go_forward());
    assert!(!browser.can_go_forward());
    browser.navigate(Location::Search("a".into()));
    browser.navigate(Location::Search("ab".into()));
    assert!(browser.go_back());
    assert_eq!(browser.location(), &Location::Folder("/home/jake".into()));
}

#[test]
fn browser_dims_unmatched_files_and_skips_them() {
    let mut database = MimeDatabase::default();
    database.add_globs2("50:text/plain:*.txt\n");
    let filter = CompiledFilter::compile(
        &Filter {
            name: "Text".into(),
            rules: vec![Rule::Mime("text/plain".into())],
        },
        &database,
    );
    let policy = open_policy(filter);
    let mut browser = Browser::new(PathBuf::from("/data"));
    let generation = browser.generation();
    let items = vec![
        item("b.png", false),
        item("a.txt", false),
        item("Folder", true),
        item(".hidden.txt", false),
        item("c.txt", false),
    ];
    assert!(!browser.set_items(generation + 1, items.clone(), &policy, None));
    assert!(browser.set_items(generation, items, &policy, Some(Path::new("/data/c.txt"))));
    let names: Vec<_> = browser
        .rows()
        .iter()
        .map(|row| row.item.name.as_str())
        .collect();
    assert_eq!(names, ["a.txt", "b.png", "c.txt", "Folder"]);
    assert!(!browser.rows()[1].enabled);
    assert_eq!(browser.anchor(), Some(2));
    browser.move_selection(-1);
    assert_eq!(browser.anchor(), Some(0));
    browser.click(1, false, false, true);
    assert_eq!(browser.anchor(), Some(0));
    browser.click(3, false, true, true);
    assert_eq!(browser.selected_paths().len(), 3);
    browser.click(3, false, false, true);
    assert_eq!(
        browser.selected_folder(),
        Some(PathBuf::from("/data/Folder"))
    );
    assert!(!policy.choosable(&browser.rows()[3].item));
    assert!(policy.choosable(&browser.rows()[0].item));
}

#[test]
fn type_to_select_extends_within_a_second() {
    let policy = open_policy(CompiledFilter::everything());
    let mut browser = Browser::new(PathBuf::from("/data"));
    let generation = browser.generation();
    browser.set_items(
        generation,
        vec![
            item("alpha", false),
            item("beta", false),
            item("bravo", false),
        ],
        &policy,
        None,
    );
    let start = Instant::now();
    assert_eq!(browser.type_select("b", start), Some(1));
    assert_eq!(
        browser.type_select("r", start + Duration::from_millis(300)),
        Some(2)
    );
    assert_eq!(
        browser.type_select("a", start + Duration::from_secs(3)),
        Some(0)
    );
}

#[test]
fn go_to_folder_expands_home_and_selects_files() {
    let root = scratch("goto");
    std::fs::create_dir_all(root.join("Docs/Inner")).unwrap();
    std::fs::create_dir_all(root.join("Downloads")).unwrap();
    std::fs::write(root.join("Docs/note.txt"), b"x").unwrap();
    let home = root.clone();
    assert_eq!(goto::expand("~", Path::new("/"), &home), Some(home.clone()));
    assert_eq!(
        goto::expand("~/Docs/../Downloads", Path::new("/"), &home),
        Some(home.join("Downloads"))
    );
    let file = goto::resolve("Docs/note.txt", &root, &home).unwrap();
    assert_eq!(file.folder, root.join("Docs"));
    assert_eq!(file.select, Some(root.join("Docs/note.txt")));
    assert!(goto::resolve("~/Nope", &root, &home).is_none());
    let suggestions = goto::suggestions("~/D", &root, &home, false);
    assert_eq!(suggestions, [home.join("Docs"), home.join("Downloads")]);
    assert_eq!(goto::display(&home.join("Docs"), &home), "~/Docs");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn panel_heights_follow_the_measured_rows() {
    assert_eq!(metrics::compact_height(3), 181.0);
    assert_eq!(metrics::compact_height(2), 145.0);
    assert_eq!(metrics::expanded_height(2), 412.0);
    assert_eq!(metrics::expanded_toolbar_top(2), 99.0);
}

#[test]
fn broker_bounds_live_panels_and_reports_close() {
    let (broker, panels) = Broker::channel();
    let (close, panel_closed, adapter_closed) = Broker::close_pair();
    let task = broker.present(Request::default(), panel_closed, adapter_closed);
    let outcome = futures_lite::future::block_on(async {
        let presented = futures_lite::future::or(async { Some(task.await) }, async {
            let panel = panels.recv().await.unwrap();
            assert_eq!(panel.id, 1);
            close.close();
            assert!(panel.closed.recv().await.is_ok());
            futures_lite::future::pending::<Option<_>>().await
        })
        .await;
        presented.unwrap()
    });
    assert_eq!(outcome, Ok(crate::outcome::Outcome::Cancelled));
}
