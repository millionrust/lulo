//! Focused application catalog contracts.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn environment() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/user")),
        data_home: Some(PathBuf::from("/home/user/.local/share")),
        data_dirs: vec![PathBuf::from("/usr/share")],
        icon_theme: None,
        desktops: vec!["niri".into()],
        locale: "en_GB.UTF-8".into(),
        path: vec![PathBuf::from("/usr/bin")],
        theme_cache: RefCell::new(HashMap::new()),
    }
}

fn application(id: &str, name: &str) -> Application {
    Application {
        id: id.into(),
        name: name.into(),
        generic_name: None,
        keywords: Vec::new(),
        source: PathBuf::from(format!("/apps/{id}")),
        icon: None,
        categories: Vec::new(),
        mime_types: Vec::new(),
        launch: LaunchSpec::OpenPath(PathBuf::from(format!("/apps/{id}"))),
        actions: Vec::new(),
    }
}

#[test]
fn desktop_entry_resolution_is_exact_with_one_portal_suffix_alias() {
    let catalog = vec![
        application("org.example.Chat.desktop", "Chat"),
        application("org.example.Chat.Beta.desktop", "Chat Beta"),
    ];
    assert_eq!(
        find_desktop_entry(&catalog, "org.example.Chat")
            .map(|application| application.name.as_str()),
        Some("Chat")
    );
    assert_eq!(
        find_desktop_entry(&catalog, "org.example.Chat.desktop")
            .map(|application| application.name.as_str()),
        Some("Chat")
    );
    assert!(find_desktop_entry(&catalog, "Chat").is_none());
    assert!(find_desktop_entry(&catalog, "org.example").is_none());
}

#[test]
fn activation_spawn_argv_preserves_arguments_terminal_and_working_directory() {
    let command = LaunchSpec::Command {
        program: "demo".into(),
        args: vec!["--title".into(), "Private document".into()],
        working_dir: Some("/home/user/Documents".into()),
        terminal: true,
    };
    assert_eq!(
        activation_spawn_argv_with_terminal(&command, Some("rmac-terminal".into())).unwrap(),
        [
            "/usr/bin/env",
            "--chdir",
            "/home/user/Documents",
            "--",
            "rmac-terminal",
            "-e",
            "demo",
            "--title",
            "Private document",
        ]
    );
    assert!(activation_spawn_argv_with_terminal(
        &LaunchSpec::OpenPath("/Applications/Demo.app".into()),
        None
    )
    .is_none());
}

#[test]
fn source_inventory_uses_authoritative_catalog_paths_and_launch_metadata() {
    let mut flatpak = application("org.example.Flatpak.desktop", "Flatpak App");
    flatpak.source =
        PathBuf::from("/var/lib/flatpak/exports/share/applications/org.example.Flatpak.desktop");
    let mut snap = application("snap-app.desktop", "Snap App");
    snap.source = PathBuf::from("/var/lib/snapd/desktop/applications/snap-app.desktop");
    let mut appimage = application("demo.desktop", "AppImage App");
    appimage.launch = LaunchSpec::Command {
        program: "/home/user/Applications/Demo.AppImage".into(),
        args: Vec::new(),
        working_dir: None,
        terminal: false,
    };
    let mut system = application("native.desktop", "System App");
    system.source = PathBuf::from("/usr/share/applications/native.desktop");

    let inventory = source_inventory(&[flatpak, snap, appimage, system]);
    assert_eq!(inventory.flatpak, 1);
    assert_eq!(inventory.snap, 1);
    assert_eq!(inventory.appimage, 1);
    assert_eq!(inventory.system_desktop_entries, 1);
    assert_eq!(inventory.total(), 4);
}

#[test]
fn parses_localized_visible_application_and_exec_codes() {
    let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("/apps/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nName[en_GB]=Demonstration\nGenericName=Tool\nGenericName[en_GB]=Developer Tool\nKeywords=code;editor;\nKeywords[en_GB]=develop;build;\nExec=demo --title %c %% %f\nIcon=demo\nCategories=Development;Utility;\nMimeType=text/plain;text/markdown;text/plain;invalid;\nOnlyShowIn=niri;\n",
            &environment(),
        )
        .unwrap();

    assert_eq!(entry.name, "Demonstration");
    assert_eq!(entry.generic_name.as_deref(), Some("Developer Tool"));
    assert_eq!(entry.keywords, ["develop", "build"]);
    let searchable = entry.searchable_text();
    assert!(searchable.contains("demonstration"));
    assert!(searchable.contains("developer tool"));
    assert!(searchable.contains("develop"));
    assert!(searchable.contains("utility"));
    assert_eq!(entry.categories, ["Development", "Utility"]);
    assert_eq!(entry.mime_types, ["text/plain", "text/markdown"]);
    assert_eq!(
        entry.launch,
        LaunchSpec::Command {
            program: "demo".into(),
            args: vec!["--title".into(), "Demonstration".into(), "%".into()],
            working_dir: None,
            terminal: false,
        }
    );
}

#[test]
fn association_outputs_are_bounded_exact_and_unambiguous() {
    assert_eq!(parse_mime_output(b"text/plain\n").unwrap(), "text/plain");
    assert!(parse_mime_output(b"text/plain\nimage/png\n").is_err());
    assert!(parse_mime_output(b"text plain\n").is_err());
    assert!(parse_mime_output(&vec![b'a'; MAX_ASSOCIATION_OUTPUT_BYTES + 1]).is_err());

    assert_eq!(
        parse_default_application_output(b"org.example.Editor.desktop\n").unwrap(),
        Some("org.example.Editor.desktop".into())
    );
    assert_eq!(parse_default_application_output(b"\n").unwrap(), None);
    for invalid in [
        b"editor\nother.desktop\n".as_slice(),
        b"../editor.desktop\n",
        b"editor.desktop;other.desktop\n",
        b"editor\n",
    ] {
        assert!(parse_default_application_output(invalid).is_err());
    }
}

#[test]
fn matching_file_handlers_are_exact_and_put_the_default_first() {
    let mut image = application("image.desktop", "Image");
    image.mime_types = vec!["image/png".into()];
    let mut alternate = application("alternate.desktop", "Alternate");
    alternate.mime_types = vec!["text/plain".into()];
    let mut default = application("default.desktop", "Default");
    default.mime_types = vec!["text/plain".into()];

    let handlers = matching_file_handlers(
        vec![image, alternate, default],
        "text/plain",
        Some("default.desktop"),
    );
    assert_eq!(
        handlers
            .iter()
            .map(|application| application.id.as_str())
            .collect::<Vec<_>>(),
        ["default.desktop", "alternate.desktop"]
    );
}

#[test]
fn mime_capabilities_are_bounded_and_reject_unsafe_tokens() {
    let values = (0..(MAX_MIME_TYPES + 10))
        .map(|index| format!("application/x-rmac-{index};"))
        .collect::<String>();
    let value = Some(values);
    let types = bounded_mime_types(value.as_ref());
    assert_eq!(types.len(), MAX_MIME_TYPES);
    assert!(valid_mime_type("application/vnd.example+json"));
    assert!(!valid_mime_type("text/plain;application/x-shellscript"));
    assert!(!valid_mime_type("../text/plain"));
    assert!(!valid_mime_type("text/\nplain"));
    assert!(!valid_mime_type(&format!(
        "text/{}",
        "a".repeat(MAX_MIME_TYPE_BYTES)
    )));
}

#[test]
fn parses_bounded_localized_desktop_actions_in_declared_order() {
    let entry = parse_desktop_entry(
            "demo.desktop",
            Path::new("/apps/demo.desktop"),
            "[Desktop Entry]\nType=Application\nName=Demo\nName[en_GB]=Demonstration\nExec=demo\nPath=/work\nTerminal=true\nActions=New-Window;Duplicate;Missing;New-Window;bad_id;\n\
             [Desktop Action New-Window]\nName=New Window\nName[en_GB]=Fresh Window\nExec=demo --new-window --title %c\n\
             [Desktop Action Duplicate]\nName=Duplicate\nExec=demo --duplicate\n\
             [Desktop Action Missing]\nName=Missing Exec\n\
             [Desktop Action bad_id]\nName=Invalid identifier\nExec=demo --invalid\n",
            &environment(),
        )
        .unwrap();

    assert_eq!(entry.actions.len(), 2);
    assert_eq!(entry.actions[0].id, "New-Window");
    assert_eq!(entry.actions[0].name, "Fresh Window");
    assert_eq!(
        entry.actions[0].launch,
        LaunchSpec::Command {
            program: "demo".into(),
            args: vec![
                "--new-window".into(),
                "--title".into(),
                "Demonstration".into()
            ],
            working_dir: Some("/work".into()),
            terminal: true,
        }
    );
    assert_eq!(entry.actions[1].id, "Duplicate");
}

#[test]
fn desktop_actions_reject_invalid_or_excessive_metadata() {
    assert!(valid_action_id("New-Window"));
    assert!(!valid_action_id("new_window"));
    assert!(!valid_action_id(""));
    assert!(!valid_action_id(&"a".repeat(MAX_ACTION_ID_BYTES + 1)));
    assert!(expand_exec(
        &"x".repeat(32 * 1024 + 1),
        "Demo",
        None,
        Path::new("demo.desktop")
    )
    .is_none());

    let action_ids = (0..40)
        .map(|index| format!("Action{index};"))
        .collect::<String>();
    let action_groups = (0..40)
        .map(|index| {
            format!(
                "[Desktop Action Action{index}]\nName=Action {index}\nExec=demo --action {index}\n"
            )
        })
        .collect::<String>();
    let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\nActions={action_ids}\n{action_groups}"
        );
    let entry = parse_desktop_entry(
        "demo.desktop",
        Path::new("demo.desktop"),
        &contents,
        &environment(),
    )
    .unwrap();
    assert_eq!(entry.actions.len(), MAX_DESKTOP_ACTIONS);
    assert_eq!(entry.actions.first().unwrap().id, "Action0");
    assert_eq!(entry.actions.last().unwrap().id, "Action31");

    let keywords = (0..70)
        .map(|index| format!("keyword{index};"))
        .collect::<String>();
    let contents = format!(
            "[Desktop Entry]\nType=Application\nName=Demo\nGenericName={}\nKeywords={keywords}\nExec=demo\n",
            "g".repeat(MAX_GENERIC_NAME_BYTES + 1)
        );
    let entry = parse_desktop_entry(
        "demo.desktop",
        Path::new("demo.desktop"),
        &contents,
        &environment(),
    )
    .unwrap();
    assert!(entry.generic_name.is_none());
    assert_eq!(entry.keywords.len(), MAX_SEARCH_KEYWORDS);
    assert_eq!(entry.keywords.last().unwrap(), "keyword63");
}

#[test]
fn hidden_no_display_and_desktop_exclusions_are_ignored() {
    for extra in [
        "Hidden=true",
        "NoDisplay=true",
        "NotShowIn=niri;",
        "OnlyShowIn=GNOME;",
    ] {
        let contents =
            format!("[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\n{extra}\n");
        assert!(parse_desktop_entry(
            "demo.desktop",
            Path::new("demo.desktop"),
            &contents,
            &environment()
        )
        .is_none());
    }
}

#[test]
fn malformed_exec_and_unknown_field_codes_are_rejected() {
    assert!(tokenize_exec("demo \"unterminated").is_none());
    assert!(expand_exec("demo %Z", "Demo", None, Path::new("demo.desktop")).is_none());
}

#[test]
fn catalog_events_ignore_reads_and_unrelated_files() {
    use notify::event::{AccessKind, AccessMode, CreateKind};

    let read = Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
        .add_path(PathBuf::from("/usr/share/applications/demo.desktop"));
    let unrelated = Event::new(EventKind::Create(CreateKind::File))
        .add_path(PathBuf::from("/usr/share/applications/readme.txt"));
    #[cfg(not(target_os = "macos"))]
    let catalog_entry = Event::new(EventKind::Create(CreateKind::File))
        .add_path(PathBuf::from("/usr/share/applications/demo.desktop"));
    #[cfg(target_os = "macos")]
    let catalog_entry = Event::new(EventKind::Create(CreateKind::File))
        .add_path(PathBuf::from("/Applications/Demo.app/Contents/Info.plist"));

    assert!(!catalog_event_is_relevant(&read));
    assert!(!catalog_event_is_relevant(&unrelated));
    assert!(catalog_event_is_relevant(&catalog_entry));
}

#[test]
fn configured_theme_prefers_gtk4_then_gtk3_and_kde() {
    let root = temporary_directory("theme-config");
    std::fs::create_dir_all(root.join("gtk-3.0")).unwrap();
    std::fs::create_dir_all(root.join("gtk-4.0")).unwrap();
    std::fs::write(root.join("kdeglobals"), "[Icons]\nTheme=Breeze\n").unwrap();
    std::fs::write(
        root.join("gtk-3.0/settings.ini"),
        "[Settings]\ngtk-icon-theme-name=Adwaita\n",
    )
    .unwrap();
    std::fs::write(
        root.join("gtk-4.0/settings.ini"),
        "[Settings]\ngtk-icon-theme-name=Yaru\n",
    )
    .unwrap();

    assert_eq!(configured_icon_theme(&root, false).as_deref(), Some("Yaru"));
    assert_eq!(
        configured_icon_theme(&root, true).as_deref(),
        Some("Breeze")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn icon_metadata_rejects_parent_paths_and_invalid_names() {
    let metadata = "[../apps]\nSize=64\nType=Fixed\n";

    assert!(parse_icon_directory(metadata, "../apps").is_none());
    assert_eq!(normalized_icon_name("demo.svg"), Some("demo"));
    assert_eq!(normalized_icon_name("../demo"), None);
    assert_eq!(valid_theme_name("../../theme"), None);
}

#[test]
fn icon_lookup_honors_theme_inheritance_and_base_precedence() {
    let root = temporary_directory("icon-theme");
    let user = root.join("user");
    let system = root.join("system");
    write_theme(
        &user,
        "Child",
        "Parent",
        "16x16/apps",
        "Size=16\nType=Fixed",
    );
    write_theme(&system, "Parent", "", "64x64/apps", "Size=64\nType=Fixed");
    let child_icon = user.join("icons/Child/16x16/apps/demo.png");
    let system_parent_icon = system.join("icons/Parent/64x64/apps/demo.png");
    std::fs::write(&child_icon, b"child").unwrap();
    std::fs::write(&system_parent_icon, b"system parent").unwrap();
    let mut environment = environment();
    environment.home = None;
    environment.data_home = Some(user.clone());
    environment.data_dirs = vec![system.clone()];
    environment.icon_theme = Some("Child".into());

    // A current-theme icon wins before a closer inherited icon.
    assert_eq!(resolve_icon("demo", &environment), Some(child_icon.clone()));

    std::fs::remove_file(child_icon).unwrap();
    assert_eq!(
        resolve_icon("demo", &environment),
        Some(system_parent_icon.clone())
    );

    // A user extension of the inherited theme overrides its system icon.
    let user_parent_icon = user.join("icons/Parent/64x64/apps/demo.png");
    std::fs::create_dir_all(user_parent_icon.parent().unwrap()).unwrap();
    std::fs::write(&user_parent_icon, b"user parent").unwrap();
    assert_eq!(resolve_icon("demo", &environment), Some(user_parent_icon));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_themed_resolver_uses_physical_edge_and_rejects_unsafe_names() {
    let root = temporary_directory("shared-icon-resolver");
    write_theme(
        &root,
        "Scaled",
        "",
        "32x32@2/apps",
        "Size=32\nScale=2\nType=Fixed",
    );
    let icon = root.join("icons/Scaled/32x32@2/apps/message.png");
    std::fs::write(&icon, b"icon").unwrap();
    let mut environment = environment();
    environment.home = None;
    environment.data_home = Some(root.clone());
    environment.data_dirs.clear();
    environment.icon_theme = Some("Scaled".into());
    let resolver = ThemedIconResolver { environment };

    assert_eq!(resolver.resolve("message", 64), Some(icon));
    assert!(resolver.resolve("../message", 64).is_none());
    assert!(resolver.resolve("message", 0).is_none());
    assert!(resolver.resolve("message", 513).is_none());
    assert!(!format!("{resolver:?}").contains("Scaled"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn icon_lookup_uses_hicolor_then_unthemed_fallbacks() {
    let root = temporary_directory("icon-fallback");
    write_theme(&root, "hicolor", "", "48x48/apps", "Size=48\nType=Fixed");
    let themed = root.join("icons/hicolor/48x48/apps/demo.png");
    std::fs::write(&themed, b"themed").unwrap();
    let mut environment = environment();
    environment.home = None;
    environment.data_home = Some(root.clone());
    environment.data_dirs.clear();
    environment.icon_theme = Some("MissingTheme".into());

    assert_eq!(resolve_icon("demo", &environment), Some(themed.clone()));

    std::fs::remove_file(themed).unwrap();
    let unthemed = root.join("icons/demo.svg");
    std::fs::write(&unthemed, b"unthemed").unwrap();
    assert_eq!(resolve_icon("demo", &environment), Some(unthemed));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn icon_directory_types_apply_size_and_scale_metadata() {
    let fixed = parse_icon_directory("[fixed]\nSize=64\nScale=2\nType=Fixed\n", "fixed").unwrap();
    let scalable = parse_icon_directory(
        "[scalable]\nSize=64\nType=Scalable\nMinSize=32\nMaxSize=128\n",
        "scalable",
    )
    .unwrap();
    let threshold = parse_icon_directory(
        "[threshold]\nSize=64\nType=Threshold\nThreshold=4\n",
        "threshold",
    )
    .unwrap();

    assert!(directory_matches(&fixed, 64, 2));
    assert!(!directory_matches(&fixed, 64, 1));
    assert!(directory_matches(&scalable, 96, 1));
    assert!(directory_matches(&threshold, 68, 1));
    assert_eq!(directory_distance(&threshold, 72, 1), 4);
}

#[test]
fn user_hidden_entry_suppresses_lower_priority_system_entry() {
    let root = std::env::temp_dir().join(format!(
        "rmac-apps-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let user = root.join("user");
    let system = root.join("system");
    std::fs::create_dir_all(user.join("applications")).unwrap();
    std::fs::create_dir_all(system.join("applications")).unwrap();
    std::fs::write(
        user.join("applications/demo.desktop"),
        "[Desktop Entry]\nType=Application\nName=Demo\nHidden=true\nExec=/bin/sh\n",
    )
    .unwrap();
    std::fs::write(
        system.join("applications/demo.desktop"),
        "[Desktop Entry]\nType=Application\nName=System Demo\nExec=/bin/sh\n",
    )
    .unwrap();
    std::fs::write(
        system.join("applications/other.desktop"),
        "[Desktop Entry]\nType=Application\nName=Other\nTryExec=/bin/sh\nExec=/bin/sh\n",
    )
    .unwrap();
    let environment = Environment {
        home: None,
        data_home: Some(user),
        data_dirs: vec![system],
        icon_theme: None,
        desktops: vec!["niri".into()],
        locale: "C".into(),
        path: vec![PathBuf::from("/bin")],
        theme_cache: RefCell::new(HashMap::new()),
    };

    let applications = discover_linux(&environment).unwrap();

    assert_eq!(applications.len(), 1);
    assert_eq!(applications[0].name, "Other");
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_directory(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-apps-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn write_theme(
    data_directory: &Path,
    theme: &str,
    inherits: &str,
    directory: &str,
    directory_metadata: &str,
) {
    let root = data_directory.join("icons").join(theme);
    std::fs::create_dir_all(root.join(directory)).unwrap();
    std::fs::write(
            root.join("index.theme"),
            format!(
                "[Icon Theme]\nName={theme}\nComment=Test\nInherits={inherits}\nDirectories={directory}\n\n[{directory}]\n{directory_metadata}\n"
            ),
        )
        .unwrap();
}

#[test]
fn only_an_absolute_desktop_entry_path_becomes_the_working_directory() {
    let entry =
        |value: &str| std::collections::HashMap::from([("Path".to_owned(), value.to_owned())]);
    assert_eq!(
        crate::platform::working_directory(&entry("/home/user/Projects")),
        Some(std::path::PathBuf::from("/home/user/Projects"))
    );
    for relative in ["", "Projects", "./bin", "../..", "~/Documents"] {
        assert_eq!(crate::platform::working_directory(&entry(relative)), None);
    }
    assert_eq!(
        crate::platform::working_directory(&std::collections::HashMap::new()),
        None
    );
}
