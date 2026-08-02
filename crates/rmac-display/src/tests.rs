//! Focused display service contracts.

use super::*;

fn niri_snapshot(json: &str) -> Snapshot {
    Snapshot {
        available: true,
        can_configure: true,
        can_persist: true,
        mirror_supported: false,
        compositor: "niri".into(),
        graphics: None,
        outputs: parse_niri_outputs(json).unwrap(),
        persistence_detail: None,
    }
}

#[test]
fn niri_fixture_preserves_modes_layout_and_stable_name() {
    let outputs = parse_niri_outputs(
        r#"{
              "Dell Inc. U2723QE ABC": {
                "name":"DP-1", "make":"Dell Inc.", "model":"U2723QE", "serial":"ABC",
                "physical_size":[600,340],
                "modes":[
                  {"width":3840,"height":2160,"refresh_rate":60000,"is_preferred":true},
                  {"width":2560,"height":1440,"refresh_rate":59951,"is_preferred":false}
                ],
                "current_mode":0, "is_custom_mode":false,
                "vrr_supported":false, "vrr_enabled":false,
                "logical":{"x":0,"y":0,"width":1920,"height":1080,"scale":2.0,"transform":"Normal"}
              }
            }"#,
    )
    .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].id, "Dell Inc. U2723QE ABC");
    assert_eq!(outputs[0].connector, "DP-1");
    assert_eq!(outputs[0].current_mode().unwrap().width, 3840);
    assert_eq!(outputs[0].logical.as_ref().unwrap().scale, 2.0);
}

#[test]
fn unknown_transform_is_preserved_without_breaking_snapshot() {
    assert_eq!(parse_transform("Flipped90"), Transform::Flipped90);
    assert!(Transform::Flipped90.is_configurable());
    assert_eq!(
        parse_transform("future-transform"),
        Transform::Other("future-transform".to_string())
    );
    assert!(!parse_transform("future-transform").is_configurable());
}

#[test]
fn mode_and_scale_arguments_are_bounded_and_precise() {
    let mode = Mode {
        width: 2560,
        height: 1440,
        refresh_rate: 143_912,
        preferred: false,
    };
    assert_eq!(mode.niri_argument(), "2560x1440@143.912");
    assert!(validate_output_id("").is_err());
    assert!(validate_output_id("DP-1\noutput DP-2").is_err());
    let output = Output {
        id: "DP-1".into(),
        connector: "DP-1".into(),
        name: "Display".into(),
        serial: None,
        physical_size_mm: None,
        modes: vec![mode],
        current_mode: Some(0),
        logical: None,
        primary: true,
        detail: None,
    };
    assert!(set_scale(&output, 10.0).is_err());
}

#[test]
fn macos_fixture_preserves_main_display_and_graphics() {
    let (graphics, outputs) = parse_macos_displays(concat!(
        "Graphics/Displays:\n\n",
        "      Chipset Model: Apple M3 Pro\n",
        "        Built-in Liquid Retina XDR Display:\n",
        "          Display Type: Built-In Liquid Retina XDR Display\n",
        "          Resolution: 3456 x 2234 Retina\n",
        "          Main Display: Yes",
    ));
    assert_eq!(graphics.as_deref(), Some("Apple M3 Pro"));
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0].primary);
    assert_eq!(outputs[0].current_mode().unwrap().height, 2234);
}

#[test]
fn errors_keep_operation_context() {
    let error = Error::new("read niri displays", "socket unavailable");
    assert_eq!(
        error.to_string(),
        "could not read niri displays: socket unavailable"
    );
}

#[test]
fn helper_output_is_drained_without_retaining_unbounded_bytes() {
    let input = vec![b'x'; 32];
    let (captured, truncated) = drain_bounded(std::io::Cursor::new(input), 8).unwrap();
    assert_eq!(captured, vec![b'x'; 8]);
    assert!(truncated);
}

#[test]
fn complete_layout_requires_exact_non_overlapping_enabled_outputs() {
    let snapshot = niri_snapshot(
        r#"{
              "Display A 1": {
                "name":"DP-1", "make":"Display", "model":"A", "serial":"1",
                "modes":[{"width":1920,"height":1080,"refresh_rate":60000,"is_preferred":true}],
                "current_mode":0,
                "logical":{"x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"transform":"Normal"}
              },
              "Display B 2": {
                "name":"DP-2", "make":"Display", "model":"B", "serial":"2",
                "modes":[{"width":2560,"height":1440,"refresh_rate":59951,"is_preferred":true}],
                "current_mode":0,
                "logical":{"x":1920,"y":0,"width":1707,"height":960,"scale":1.5,"transform":"Normal"}
              }
            }"#,
    );
    let layout = current_layout(&snapshot, "Display B 2").unwrap();
    assert_eq!(layout.outputs.len(), 2);
    assert_eq!(layout.primary, "Display B 2");

    let mut missing = layout.clone();
    missing.outputs.pop();
    assert!(validate_layout(&missing, &snapshot).is_err());

    let mut stale = layout.clone();
    stale.outputs[0].mode.refresh_rate = 60_001;
    assert!(validate_layout(&stale, &snapshot).is_err());

    let mut overlap = layout.clone();
    overlap.outputs[1].x = 100;
    assert!(rectangles_overlap(&overlap.outputs[0], &overlap.outputs[1]));
}

#[test]
fn snapshot_restore_requires_the_same_enabled_hardware_identities() {
    let expected = niri_snapshot(
        r#"{
              "Display A 1": {
                "name":"DP-1", "make":"Display", "model":"A", "serial":"1",
                "modes":[{"width":1920,"height":1080,"refresh_rate":60000,"is_preferred":true}],
                "current_mode":0,
                "logical":{"x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"transform":"Normal"}
              }
            }"#,
    );
    let mut changed = expected.clone();
    assert!(same_enabled_output_identities(&expected, &changed));
    assert!(same_complete_layout(&expected, &changed));

    changed.outputs[0].serial = Some("replacement".into());
    assert!(!same_enabled_output_identities(&expected, &changed));
    changed.outputs[0].serial = Some("1".into());
    changed.outputs[0].logical.as_mut().unwrap().x = 40;
    assert!(!same_complete_layout(&expected, &changed));
    changed.outputs[0].logical.as_mut().unwrap().x = 0;
    changed.outputs[0].logical = None;
    assert!(!same_enabled_output_identities(&expected, &changed));
}

#[test]
fn managed_layout_preserves_offline_outputs_and_has_one_main_display() {
    assert_eq!(MANAGED_CONFIG_NAME, ".rmac-displays.kdl");
    let existing = format!(
            "{MANAGED_HEADER}\noutput \"Offline Panel 9\" {{\n    mode \"1280x720@60.000\"\n    scale 1.0\n    transform \"normal\"\n    position x=0 y=0\n    focus-at-startup\n}}\n"
        );
    let layout = Layout {
        primary: "Display B 2".into(),
        outputs: vec![
            OutputConfiguration {
                id: "Display A 1".into(),
                mode: Mode {
                    width: 1920,
                    height: 1080,
                    refresh_rate: 60_000,
                    preferred: true,
                },
                scale: 1.0,
                transform: Transform::Normal,
                x: 0,
                y: 0,
                logical_width: 1920,
                logical_height: 1080,
            },
            OutputConfiguration {
                id: "Display B 2".into(),
                mode: Mode {
                    width: 2560,
                    height: 1440,
                    refresh_rate: 59_951,
                    preferred: true,
                },
                scale: 1.5,
                transform: Transform::Rotate90,
                x: 1920,
                y: 0,
                logical_width: 960,
                logical_height: 1707,
            },
        ],
    };
    let saved = update_managed_source(Some(&existing), &layout).unwrap();
    let document = parse_managed_document(&saved).unwrap();
    assert_eq!(document.nodes().len(), 3);
    assert_eq!(
        parse_managed_primary(&saved).unwrap().as_deref(),
        Some("Display B 2")
    );
    assert!(saved.contains("Offline Panel 9"));
    assert!(saved.contains("2560x1440@59.951"));
    assert!(saved.contains("transform \"90\""));
}

#[test]
fn managed_layout_serializes_plain_strings_as_niri_kdl_v1() {
    let layout = Layout {
        primary: "eDP-1".into(),
        outputs: vec![OutputConfiguration {
            id: "eDP-1".into(),
            mode: Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60_000,
                preferred: true,
            },
            scale: 1.0,
            transform: Transform::Normal,
            x: 0,
            y: 0,
            logical_width: 1920,
            logical_height: 1080,
        }],
    };

    let saved = update_managed_source(None, &layout).unwrap();
    assert!(saved.contains("output \"eDP-1\""));
    assert!(saved.contains("mode \"1920x1080@60.000\""));
    assert!(saved.contains("transform \"normal\""));
    assert!(!saved.contains("#true"));
    assert!(!saved.contains("#false"));
}

#[test]
fn managed_layout_rejects_foreign_content_and_ambiguous_primary() {
    let foreign = format!("{MANAGED_HEADER}\nspawn \"private-command\"\n");
    assert!(parse_managed_document(&foreign).is_err());
    let ambiguous = format!(
            "{MANAGED_HEADER}\noutput \"A\" {{ focus-at-startup }}\noutput \"B\" {{ focus-at-startup }}\n"
        );
    assert!(parse_managed_primary(&ambiguous).is_err());
}
