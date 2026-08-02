//! Focused input service contracts.

use super::*;

const CONFIG: &str = r#"
input {
    keyboard {
        repeat-delay 450
        repeat-rate 32
        numlock false
        track-layout "global"
    }
    mouse {
        natural-scroll
        accel-speed -0.25
        accel-profile "flat"
        scroll-method "on-button-down"
        middle-emulation
    }
    touchpad {
        tap
        dwt false
        accel-speed 0.2
        middle-emulation
    }
    warp-mouse-to-focus
}
"#;

fn effective(source: &str) -> EffectiveConfig {
    let document = KdlDocument::parse_v1(source).unwrap();
    let mut effective = EffectiveConfig::default();
    for node in document.nodes() {
        if node.name().value() == "input" {
            apply_input(node.children().unwrap(), &mut effective);
        }
    }
    effective
}

fn test_directory(name: &str) -> PathBuf {
    let sequence = CANDIDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rmac-input-{name}-{}-{sequence}",
        std::process::id()
    ))
}

#[test]
fn reads_known_values_and_explicit_false_flags() {
    let settings = effective(CONFIG).settings;
    assert_eq!(settings.keyboard.repeat_delay_ms, 450);
    assert_eq!(settings.keyboard.repeat_rate, 32);
    assert!(!settings.keyboard.numlock);
    assert!(settings.mouse.natural_scroll);
    assert_eq!(settings.mouse.accel_profile, AccelProfile::Flat);
    assert_eq!(settings.mouse.accel_speed, -0.25);
    assert!(settings.mouse.middle_emulation);
    assert!(settings.touchpad.tap_to_click);
    assert!(!settings.touchpad.disable_while_typing);
    assert!(settings.touchpad.pointer.middle_emulation);
    assert_eq!(settings.touchpad.pointer.accel_speed, 0.2);
}

#[test]
fn keyboard_merges_but_pointing_sections_replace() {
    let mut effective = effective(CONFIG);
    let later = KdlDocument::parse_v1(
        r#"
input {
    keyboard { repeat-rate 40; numlock; }
    mouse { accel-speed 0.5; }
}
"#,
    )
    .unwrap();
    apply_input(
        later.get("input").unwrap().children().unwrap(),
        &mut effective,
    );
    assert_eq!(effective.settings.keyboard.repeat_delay_ms, 450);
    assert_eq!(effective.settings.keyboard.repeat_rate, 40);
    assert!(effective.settings.keyboard.numlock);
    assert_eq!(effective.settings.mouse.accel_speed, 0.5);
    assert!(!effective.settings.mouse.natural_scroll);
    assert!(!effective.settings.mouse.middle_emulation);
}

#[test]
fn managed_override_preserves_unknown_pointer_nodes_and_explicit_off() {
    let effective = effective(CONFIG);
    let authority = Authority {
        main_path: PathBuf::from("/config.kdl"),
        main_source: String::new(),
        managed_path: PathBuf::from("/.rmac-input.kdl"),
        managed_source: None,
        has_managed_include: false,
        safe_to_write: true,
        detail: None,
        effective: effective.clone(),
        files: vec![ConfigFile {
            path: PathBuf::from("/config.kdl"),
            source: String::new(),
        }],
        missing_optional_files: Vec::new(),
    };
    let mut requested = effective.settings.clone();
    requested.keyboard.repeat_rate = 40;
    requested.mouse.natural_scroll = false;
    let managed = update_managed_source(&authority, &requested).unwrap();
    assert!(managed.contains("scroll-method \"on-button-down\""));
    assert!(managed.contains("accel-profile \"flat\""));
    assert!(managed.contains("numlock false"));
    assert!(!managed.contains("#false"));

    let managed =
        parse_managed_document(&managed).unwrap_or_else(|error| panic!("{error}\n{managed}"));
    let mut reread = effective;
    apply_input(
        managed.get("input").unwrap().children().unwrap(),
        &mut reread,
    );
    assert_eq!(reread.settings, requested);
}

#[test]
fn include_graph_is_recursive_positional_and_bounded() {
    let directory = test_directory("includes");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
            directory.join("base.kdl"),
            "input {\n    keyboard {\n        repeat-delay 300\n        numlock\n    }\n    mouse {\n        natural-scroll\n    }\n}\n",
        )
        .unwrap();
    std::fs::write(
            directory.join("nested.kdl"),
            "include \"base.kdl\"\ninput {\n    keyboard {\n        repeat-rate 40\n        numlock false\n    }\n    mouse {\n        accel-speed 0.5\n    }\n}\n",
        )
        .unwrap();
    std::fs::write(
        directory.join("config.kdl"),
        "include optional=true \"missing.kdl\"\ninclude \"nested.kdl\"\n",
    )
    .unwrap();
    let mut graph = GraphState::default();
    traverse_config(&directory.join("config.kdl"), 0, &mut graph).unwrap();
    assert_eq!(graph.files.len(), 3);
    assert_eq!(graph.effective.settings.keyboard.repeat_delay_ms, 300);
    assert_eq!(graph.effective.settings.keyboard.repeat_rate, 40);
    assert!(!graph.effective.settings.keyboard.numlock);
    assert_eq!(graph.effective.settings.mouse.accel_speed, 0.5);
    assert!(!graph.effective.settings.mouse.natural_scroll);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn recursive_include_is_rejected() {
    let directory = test_directory("cycle");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("a.kdl"), "include \"b.kdl\"\n").unwrap();
    std::fs::write(directory.join("b.kdl"), "include \"a.kdl\"\n").unwrap();
    let mut graph = GraphState::default();
    assert!(traverse_config(&directory.join("a.kdl"), 0, &mut graph).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn empty_xkb_follows_localed_but_explicit_empty_options_do_not() {
    let empty = effective("input { keyboard { xkb {}; }; }\n");
    assert!(empty.settings.keyboard.xkb_override.is_none());
    let explicit = effective(
        r#"
input {
    keyboard {
        xkb {
            options ""
        }
    }
}
"#,
    );
    assert_eq!(
        explicit.settings.keyboard.xkb_override.unwrap().options,
        Some(String::new())
    );
}

#[test]
fn rejects_out_of_range_values() {
    let mut settings = InputSettings::default();
    settings.mouse.accel_speed = 1.1;
    assert!(validate_settings(&settings).is_err());
    settings.mouse.accel_speed = 0.0;
    settings.keyboard.repeat_rate = 0;
    assert!(validate_settings(&settings).is_err());
}

#[test]
fn managed_file_rejects_unrelated_top_level_content() {
    let foreign = format!("{MANAGED_HEADER}\nspawn \"private-command\"\n");
    assert!(parse_managed_document(&foreign).is_err());
    let duplicate = format!("{MANAGED_HEADER}\ninput {{ keyboard {{}}; keyboard {{}}; }}\n");
    assert!(parse_managed_document(&duplicate).is_err());
    let unsupported_keyboard = format!("{MANAGED_HEADER}\ninput {{ keyboard {{ xkb {{}}; }}; }}\n");
    assert!(parse_managed_document(&unsupported_keyboard).is_err());
    let imitated_header = format!("{MANAGED_HEADER} but not really\n");
    assert!(parse_managed_document(&imitated_header).is_err());
}

#[test]
fn include_nodes_reject_children_and_track_missing_optional_files() {
    let source = Path::new("/config/config.kdl");
    let document = KdlDocument::parse_v1("include \"child.kdl\" { input {}; }\n").unwrap();
    assert!(include_path(&document.nodes()[0], source).is_err());

    let directory = test_directory("optional");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("config.kdl"),
        "include optional=true \"missing.kdl\"\n",
    )
    .unwrap();
    let mut graph = GraphState::default();
    traverse_config(&directory.join("config.kdl"), 0, &mut graph).unwrap();
    assert_eq!(graph.missing_optional_files.len(), 1);
    assert_eq!(
        graph.missing_optional_files[0],
        directory.join("missing.kdl")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn concurrent_include_changes_are_rejected_before_save() {
    let directory = test_directory("concurrency");
    std::fs::create_dir_all(&directory).unwrap();
    let main_path = directory.join("config.kdl");
    let included_path = directory.join("included.kdl");
    let main_source = "include \"included.kdl\"\n";
    let included_source = "input { keyboard { repeat-rate 30; }; }\n";
    std::fs::write(&main_path, main_source).unwrap();
    std::fs::write(&included_path, included_source).unwrap();
    let authority = Authority {
        main_path: main_path.clone(),
        main_source: main_source.into(),
        managed_path: directory.join(MANAGED_CONFIG_NAME),
        managed_source: None,
        has_managed_include: false,
        safe_to_write: true,
        detail: None,
        effective: EffectiveConfig::default(),
        files: vec![
            ConfigFile {
                path: main_path,
                source: main_source.into(),
            },
            ConfigFile {
                path: included_path.clone(),
                source: included_source.into(),
            },
        ],
        missing_optional_files: Vec::new(),
    };
    assert!(ensure_authority_unchanged(&authority).is_ok());
    std::fs::write(&included_path, "input { keyboard { repeat-rate 31; }; }\n").unwrap();
    assert!(ensure_authority_unchanged(&authority).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn managed_include_requires_one_exact_final_main_reference() {
    let directory = test_directory("managed-include");
    std::fs::create_dir_all(&directory).unwrap();
    let main_path = directory.join("config.kdl");
    let managed_path = directory.join(MANAGED_CONFIG_NAME);
    std::fs::write(
        &managed_path,
        format!("{MANAGED_HEADER}\ninput {{ keyboard {{ repeat-rate 30; }}; }}\n"),
    )
    .unwrap();
    let location = ConfigLocation {
        path: main_path.clone(),
        writable_user_config: true,
    };

    std::fs::write(&main_path, "include \"./.rmac-input.kdl\"\n").unwrap();
    let aliased = load_authority(&location).unwrap();
    assert!(!aliased.safe_to_write);

    std::fs::write(&main_path, "include \".rmac-input.kdl\"\n").unwrap();
    let exact = load_authority(&location).unwrap();
    assert!(exact.safe_to_write);
    assert!(exact.has_managed_include);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn helper_output_is_drained_without_unbounded_retention() {
    let (captured, truncated) = drain_bounded(std::io::Cursor::new(vec![b'x'; 32]), 8).unwrap();
    assert_eq!(captured, vec![b'x'; 8]);
    assert!(truncated);
}

#[test]
fn device_classification_prefers_udev_capabilities_without_serial_data() {
    assert_eq!(
        classify_device("Generic Device", "E:ID_INPUT=1\nE:ID_INPUT_TOUCHPAD=1\n"),
        DeviceKind::Touchpad
    );
    assert_eq!(classify_device("USB Keyboard", ""), DeviceKind::Keyboard);
    assert_eq!(
        classify_device("Unknown", "E:ID_INPUT_MOUSE=1\n"),
        DeviceKind::Mouse
    );
}

#[test]
fn config_reload_events_require_an_explicit_success_state() {
    assert_eq!(
        config_load_failed(&serde_json::json!({"ConfigLoaded": {"failed": false}})),
        Some(false)
    );
    assert_eq!(
        config_load_failed(&serde_json::json!({"ConfigLoaded": {"failed": true}})),
        Some(true)
    );
    assert_eq!(
        config_load_failed(&serde_json::json!({"ConfigLoaded": {}})),
        None
    );
}

#[cfg(unix)]
#[test]
fn reload_witness_reads_bounded_success_and_failure_events() {
    let (client, mut server) = std::os::unix::net::UnixStream::pair().unwrap();
    server
            .write_all(
                b"{\"Ok\":\"Handled\"}\n{\"ConfigLoaded\":{\"failed\":false}}\n{\"ConfigLoaded\":{\"failed\":true}}\n",
            )
            .unwrap();
    let mut reader = std::io::BufReader::new(client);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    assert!(read_reload_json_line(&mut reader, deadline)
        .unwrap()
        .get("Ok")
        .is_some());
    assert!(!next_config_load(&mut reader, deadline).unwrap());
    assert!(next_config_load(&mut reader, deadline).unwrap());
}
