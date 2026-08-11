//! Focused Dock model and surface contracts.

use std::path::Path;

use super::*;

fn application(id: &str, name: &str) -> rmac_apps::Application {
    rmac_apps::Application {
        id: id.into(),
        name: name.into(),
        generic_name: None,
        keywords: Vec::new(),
        source: PathBuf::from(format!("/apps/{id}")),
        icon: Some(PathBuf::from(format!("/icons/{id}.svg"))),
        categories: Vec::new(),
        mime_types: Vec::new(),
        launch: rmac_apps::LaunchSpec::Command {
            program: id.trim_end_matches(".desktop").into(),
            args: Vec::new(),
            working_dir: None,
            terminal: false,
        },
        actions: Vec::new(),
    }
}

fn window(
    id: u64,
    app_id: &str,
    focused: bool,
    urgent: bool,
    seconds: u64,
) -> rmac_compositor::Window {
    rmac_compositor::Window {
        id: rmac_compositor::WindowId(id),
        title: Some(format!("Window {id}")),
        app_id: Some(app_id.into()),
        pid: None,
        workspace: None,
        focused,
        floating: false,
        urgent,
        focus_timestamp: Some(rmac_compositor::Timestamp {
            seconds,
            nanoseconds: 0,
        }),
        layout: rmac_compositor::WindowLayout::default(),
    }
}

#[test]
fn pinned_order_leads_and_running_windows_group_by_desktop_identity() {
    let catalog = [
        application("finder.desktop", "Finder"),
        application("terminal.desktop", "Terminal"),
    ];
    let compositor = rmac_compositor::Snapshot {
        windows: vec![
            window(1, "terminal", false, false, 20),
            window(2, "terminal.desktop", true, true, 30),
            window(3, "org.example.music", false, false, 40),
        ],
        ..Default::default()
    };
    let model = Model::build(
        &[
            rmac_shell_settings::AppId("finder.desktop".into()),
            rmac_shell_settings::AppId("terminal.desktop".into()),
        ],
        &Default::default(),
        &catalog,
        &compositor,
    );
    assert_eq!(
        model
            .items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["Finder", "Terminal", "Music"]
    );
    assert!(!model.items[0].running);
    assert_eq!(model.items[1].windows.len(), 2);
    assert!(model.items[1].active);
    assert!(model.items[1].urgent);
    assert!(!model.items[2].launchable);
}

#[test]
fn unpinned_running_apps_do_not_reorder_when_focus_changes() {
    let catalog = [
        application("alacritty.desktop", "Alacritty"),
        application("firefox.desktop", "Firefox"),
    ];
    let order = |windows| {
        Model::build(
            &[],
            &Default::default(),
            &catalog,
            &rmac_compositor::Snapshot {
                windows,
                ..Default::default()
            },
        )
        .items
        .into_iter()
        .map(|item| item.name)
        .collect::<Vec<_>>()
    };

    let alacritty_focused = order(vec![
        window(1, "alacritty", true, false, 20),
        window(2, "firefox", false, false, 10),
    ]);
    let firefox_focused = order(vec![
        window(1, "alacritty", false, false, 20),
        window(2, "firefox", true, false, 30),
    ]);

    assert_eq!(alacritty_focused, ["Alacritty", "Firefox"]);
    assert_eq!(firefox_focused, alacritty_focused);
}

#[test]
fn click_launches_or_focuses_without_optimistic_state() {
    let catalog = [application("finder.desktop", "Finder")];
    let pinned = [rmac_shell_settings::AppId("finder.desktop".into())];
    let model = Model::build(&pinned, &Default::default(), &catalog, &Default::default());
    assert!(matches!(
        model.activate("finder.desktop"),
        Activation::Launch { .. }
    ));

    let compositor = rmac_compositor::Snapshot {
        windows: vec![window(7, "finder", false, false, 8)],
        ..Default::default()
    };
    let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
    assert_eq!(
        model.activate("finder.desktop"),
        Activation::FocusWindow(rmac_compositor::WindowId(7))
    );
}

#[test]
fn repeated_click_cycles_recent_windows_and_single_window_is_a_noop() {
    let catalog = [application("terminal.desktop", "Terminal")];
    let pinned = [rmac_shell_settings::AppId("terminal.desktop".into())];
    let compositor = rmac_compositor::Snapshot {
        windows: vec![
            window(1, "terminal", false, false, 10),
            window(2, "terminal", true, false, 20),
            window(3, "terminal", false, false, 30),
        ],
        ..Default::default()
    };
    let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
    assert_eq!(
        model.activate("terminal"),
        Activation::FocusWindow(rmac_compositor::WindowId(3))
    );

    let compositor = rmac_compositor::Snapshot {
        windows: vec![window(2, "terminal", true, false, 20)],
        ..Default::default()
    };
    let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
    assert_eq!(model.activate("terminal"), Activation::NoAction);
}

#[test]
fn unsupported_hide_and_missing_pinned_app_are_truthful() {
    let settings = rmac_shell_settings::DockSettings {
        repeated_click: rmac_shell_settings::RepeatedClickBehavior::HideApplication,
        ..Default::default()
    };
    let compositor = rmac_compositor::Snapshot {
        windows: vec![window(1, "missing", true, false, 1)],
        ..Default::default()
    };
    let model = Model::build(
        &[rmac_shell_settings::AppId("missing.desktop".into())],
        &settings,
        &[],
        &compositor,
    );
    assert!(!model.items[0].launchable);
    assert!(matches!(
        model.activate("missing"),
        Activation::Unavailable { .. }
    ));

    let model = Model::build(
        &[rmac_shell_settings::AppId("gone.desktop".into())],
        &Default::default(),
        &[],
        &Default::default(),
    );
    assert!(matches!(
        model.activate("gone.desktop"),
        Activation::Unavailable { .. }
    ));
}

fn output_with_geometry(
    id: &str,
    enabled: bool,
    width: f64,
    height: f64,
    scale: f64,
) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: String::new(),
        model: String::new(),
        serial: None,
        physical_size_mm: None,
        modes: Vec::new(),
        current_mode: enabled.then_some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: enabled.then_some(rmac_compositor::LogicalOutput {
            position: Default::default(),
            size: rmac_compositor::LogicalSize { width, height },
            scale,
            transform: "normal".into(),
        }),
    }
}

fn output(id: &str, enabled: bool) -> rmac_compositor::Output {
    output_with_geometry(id, enabled, 1920.0, 1080.0, 1.0)
}

#[test]
fn output_scope_never_invents_a_primary_or_disabled_surface() {
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![output("eDP-1", true), output("HDMI-A-1", false)],
        ..Default::default()
    };
    assert_eq!(
        surface_outputs(&compositor, &rmac_shell_settings::OutputScope::All, None,),
        [rmac_compositor::OutputId::from("eDP-1")]
    );
    assert!(surface_outputs(
        &compositor,
        &rmac_shell_settings::OutputScope::Primary,
        None,
    )
    .is_empty());
    assert!(surface_outputs(
        &compositor,
        &rmac_shell_settings::OutputScope::Named("HDMI-A-1".into()),
        None,
    )
    .is_empty());
    assert_eq!(
        surface_outputs(
            &compositor,
            &rmac_shell_settings::OutputScope::Primary,
            Some(&rmac_compositor::OutputId::from("eDP-1")),
        ),
        [rmac_compositor::OutputId::from("eDP-1")]
    );
}

#[test]
fn surface_plan_is_sorted_scaled_and_rejects_unrenderable_outputs() {
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![
            output_with_geometry("eDP-1", true, 1512.0, 982.0, 2.0),
            output_with_geometry("DP-2", true, 2560.0, 1440.0, 1.25),
            output_with_geometry("DP-1", true, 0.0, 1440.0, 1.0),
            output_with_geometry("HDMI-A-1", true, 1920.0, 1080.0, f64::NAN),
        ],
        ..Default::default()
    };
    let bottom = surface_descriptions(
        &compositor,
        &rmac_shell_settings::DockSettings::default(),
        None,
        false,
    )
    .expect("default surface policy is valid");

    assert_eq!(
        bottom
            .iter()
            .map(|surface| surface.output.0.as_str())
            .collect::<Vec<_>>(),
        ["DP-2", "eDP-1"]
    );
    assert_eq!(bottom[0].output_axis_length, 2560.0);
    assert_eq!(bottom[0].output_scale, 1.25);
    assert_eq!(bottom[1].output_axis_length, 1512.0);
    assert_eq!(bottom[1].output_scale, 2.0);

    let settings = rmac_shell_settings::DockSettings {
        placement: rmac_shell_settings::DockPlacement::Left,
        ..Default::default()
    };
    let side = surface_descriptions(&compositor, &settings, None, false)
        .expect("side surface policy is valid");
    assert_eq!(side[0].output_axis_length, 1440.0);
    assert_eq!(side[1].output_axis_length, 982.0);
    assert_eq!(
        surface_outputs(&compositor, &settings.outputs, None).len(),
        2
    );
}

#[test]
fn surface_plan_keeps_reservation_stable_across_magnification_and_autohide() {
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![output("eDP-1", true)],
        overview_visible: true,
        ..Default::default()
    };
    let settings = rmac_shell_settings::DockSettings {
        autohide: true,
        magnification: true,
        magnification_scale: 1.5,
        reserve_space: true,
        ..Default::default()
    };
    let surface = surface_descriptions(&compositor, &settings, None, false)
        .expect("surface policy is valid")
        .remove(0);

    assert_eq!(surface.base_thickness, 64.0);
    assert_eq!(surface.maximum_thickness, 88.0);
    assert_eq!(surface.exclusive_zone, 64.0);
    assert_eq!(surface.reveal_edge_thickness, HIDDEN_EDGE_THICKNESS);
    assert!(!surface.keyboard_interactive);
    assert!(surface.autohide);
    assert!(surface.overview_visible);
    assert!(surface.magnification_enabled);
    assert!(surface.animate);

    let layout = motion::magnified_layout(
        3,
        Some(24.0),
        surface.magnification_enabled,
        false,
        surface.magnification,
    )
    .expect("surface-provided magnification is valid");
    assert!(layout.items[0].size > surface.magnification.icon_size);
    assert_eq!(surface.exclusive_zone, surface.base_thickness);

    let no_reservation = surface_descriptions(
        &compositor,
        &rmac_shell_settings::DockSettings {
            reserve_space: false,
            ..settings
        },
        None,
        false,
    )
    .expect("surface policy is valid")
    .remove(0);
    assert_eq!(no_reservation.exclusive_zone, 0.0);
    assert_eq!(no_reservation.reveal_edge_thickness, HIDDEN_EDGE_THICKNESS);
}

#[test]
fn reduced_motion_disables_dock_scaling_and_animation() {
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![output("eDP-1", true)],
        ..Default::default()
    };
    let surface = surface_descriptions(
        &compositor,
        &rmac_shell_settings::DockSettings::default(),
        None,
        true,
    )
    .expect("surface policy is valid")
    .remove(0);

    assert!(!surface.magnification_enabled);
    assert!(!surface.animate);
    assert_eq!(surface.maximum_thickness, surface.base_thickness);
}

#[test]
fn invalid_magnification_policy_fails_before_any_surface_is_described() {
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![output("eDP-1", true)],
        ..Default::default()
    };
    let invalid = rmac_shell_settings::DockSettings {
        magnification_scale: f32::NAN,
        ..Default::default()
    };
    assert_eq!(
        surface_descriptions(&compositor, &invalid, None, false),
        Err(motion::ConfigError::NonFinite)
    );
}

#[test]
fn context_menu_exposes_real_windows_and_hides_unverifiable_process_actions() {
    let catalog = [application("terminal.desktop", "Terminal")];
    let pinned = [rmac_shell_settings::AppId("terminal.desktop".into())];
    let compositor = rmac_compositor::Snapshot {
        windows: vec![
            window(1, "terminal", true, false, 20),
            window(2, "terminal", false, true, 10),
        ],
        ..Default::default()
    };
    let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
    let menu = model.context_menu("terminal").expect("Dock item exists");
    assert_eq!(menu.application_name, "Terminal");
    assert!(menu.open.is_none());
    assert!(menu.application_commands.is_empty());
    assert_eq!(menu.windows.len(), 2);
    assert!(menu.windows[0].focused);
    assert!(matches!(
        menu.windows[0].close,
        ContextAction::CloseWindow {
            window: rmac_compositor::WindowId(1),
            ..
        }
    ));
    assert_eq!(
        menu.pin,
        PinCommand::Unpin {
            app_id: "terminal.desktop".into()
        }
    );
    assert!(matches!(
        menu.show_in_finder,
        Some(ContextAction::RevealApplication { .. })
    ));
    assert!(menu.quit.is_none());
    assert!(menu.force_quit.is_none());
}

#[test]
fn context_menu_quit_actions_require_and_revalidate_exact_live_processes() {
    let catalog = [application("terminal.desktop", "Terminal")];
    let pinned = [rmac_shell_settings::AppId("terminal.desktop".into())];
    let mut first = window(1, "terminal", true, false, 20);
    first.pid = Some(4242);
    let mut second = window(2, "terminal", false, false, 10);
    second.pid = Some(4242);
    let model = Model::build(
        &pinned,
        &Default::default(),
        &catalog,
        &rmac_compositor::Snapshot {
            windows: vec![first, second],
            ..Default::default()
        },
    );
    let menu = model.context_menu("terminal").unwrap();
    let quit = menu.quit.clone().expect("authoritative PID exposes Quit");
    assert_eq!(
        quit,
        ContextAction::TerminateApplication {
            app_id: "terminal.desktop".into(),
            pids: vec![4242],
            kind: TerminationKind::Quit,
        }
    );
    assert!(matches!(
        menu.force_quit,
        Some(ContextAction::TerminateApplication {
            kind: TerminationKind::ForceQuit,
            ..
        })
    ));
    assert!(model.authorizes_context_action(&quit));

    let mut replaced = window(1, "terminal", true, false, 30);
    replaced.pid = Some(4343);
    let changed = Model::build(
        &pinned,
        &Default::default(),
        &catalog,
        &rmac_compositor::Snapshot {
            windows: vec![replaced],
            ..Default::default()
        },
    );
    assert!(!changed.authorizes_context_action(&quit));
}

#[test]
fn reveal_action_keeps_catalog_source_private_and_revalidates_it() {
    let catalog = [application("terminal.desktop", "Terminal")];
    let model = Model::build(
        &[rmac_shell_settings::AppId("terminal.desktop".into())],
        &Default::default(),
        &catalog,
        &Default::default(),
    );
    let action = model
        .context_menu("terminal")
        .and_then(|menu| menu.show_in_finder)
        .unwrap();
    assert!(model.authorizes_context_action(&action));
    let debug = format!("{action:?}");
    assert!(debug.contains("<private>"));
    assert!(!debug.contains("/apps/"));
}

fn places(downloads: &str, downloads_exists: bool, trash_count: usize) -> rmac_places::Snapshot {
    rmac_places::Snapshot {
        home: rmac_places::Place {
            path: PathBuf::from("/home/alex"),
            exists: true,
        },
        downloads: rmac_places::Place {
            path: PathBuf::from(downloads),
            exists: downloads_exists,
        },
        downloads_configured: true,
        trash: rmac_places::TrashSnapshot {
            available: true,
            empty: trash_count == 0,
            item_count: trash_count,
        },
    }
}

#[test]
fn places_project_after_applications_without_becoming_pins() {
    let places = places("/home/alex/Transfers", true, 7);
    let model = Model::build_with_places(
        &[rmac_shell_settings::AppId("finder.desktop".into())],
        &Default::default(),
        &[application("finder.desktop", "Finder")],
        &Default::default(),
        &places,
    );

    assert_eq!(model.items.len(), 1);
    assert_eq!(model.items[0].id, "finder.desktop");
    assert_eq!(
        model
            .special_items
            .iter()
            .map(|item| (item.kind, item.name, item.available, item.item_count))
            .collect::<Vec<_>>(),
        [
            (SpecialItemKind::Files, "Files", true, None),
            (SpecialItemKind::Downloads, "Downloads", true, None),
            (SpecialItemKind::Trash, "Trash", true, Some(7)),
        ]
    );
    assert!(matches!(
        model.activate_special(SpecialItemKind::Downloads),
        SpecialActivation::OpenDirectory {
            kind: SpecialItemKind::Downloads,
            path
        } if path == Path::new("/home/alex/Transfers")
    ));
}

#[test]
fn disabled_or_missing_places_remain_visible_and_truthfully_unavailable() {
    let mut places = places("/home/alex", true, 0);
    places.home.exists = false;
    let model =
        Model::build_with_places(&[], &Default::default(), &[], &Default::default(), &places);

    assert!(!model.special_items[0].available);
    assert!(matches!(
        model.activate_special(SpecialItemKind::Files),
        SpecialActivation::Unavailable {
            kind: SpecialItemKind::Files,
            ..
        }
    ));
    assert!(!model.special_items[1].available);
    assert!(matches!(
        model.activate_special(SpecialItemKind::Downloads),
        SpecialActivation::Unavailable {
            kind: SpecialItemKind::Downloads,
            ..
        }
    ));
    assert_eq!(model.special_items[2].item_count, Some(0));
}

#[test]
fn special_activation_debug_never_discloses_private_paths() {
    let activation = SpecialActivation::OpenDirectory {
        kind: SpecialItemKind::Files,
        path: PathBuf::from("/home/alex/Private/Tax"),
    };
    let debug = format!("{activation:?}");
    assert!(debug.contains("<private>"));
    assert!(!debug.contains("alex"));
    assert!(!debug.contains("Tax"));
}

#[test]
fn only_authoritatively_nonempty_trash_offers_destructive_review() {
    let model = Model::build_with_places(
        &[],
        &Default::default(),
        &[],
        &Default::default(),
        &places("/home/alex/Downloads", true, 4),
    );
    assert_eq!(
        model
            .special_context_menu(SpecialItemKind::Trash)
            .expect("Trash menu")
            .empty_trash,
        Some(SpecialContextAction::EmptyTrash {
            expected_item_count: 4
        })
    );
    assert!(model
        .special_context_menu(SpecialItemKind::Files)
        .expect("Files menu")
        .empty_trash
        .is_none());

    let empty = Model::build_with_places(
        &[],
        &Default::default(),
        &[],
        &Default::default(),
        &places("/home/alex/Downloads", true, 0),
    );
    assert!(empty
        .special_context_menu(SpecialItemKind::Trash)
        .expect("Trash menu")
        .empty_trash
        .is_none());
}

#[test]
fn context_menu_offers_pin_for_an_unpinned_running_app() {
    let catalog = [application("music.desktop", "Music")];
    let compositor = rmac_compositor::Snapshot {
        windows: vec![window(4, "music", false, false, 1)],
        ..Default::default()
    };
    let model = Model::build(&[], &Default::default(), &catalog, &compositor);
    let menu = model
        .context_menu("music.desktop")
        .expect("running item exists");
    assert_eq!(
        menu.pin,
        PinCommand::Pin {
            app_id: "music.desktop".into()
        }
    );
}

#[test]
fn pin_mutations_are_idempotent_bounded_and_preserve_exact_ids() {
    let pinned = vec![
        rmac_shell_settings::AppId("finder.desktop".into()),
        rmac_shell_settings::AppId("terminal.desktop".into()),
    ];
    assert_eq!(
        apply_pin_command(
            &pinned,
            &PinCommand::Pin {
                app_id: "Finder".into()
            }
        )
        .expect("duplicate pin is a no-op"),
        pinned
    );
    let moved = apply_pin_command(
        &pinned,
        &PinCommand::Move {
            app_id: "terminal".into(),
            direction: MoveDirection::Left,
        },
    )
    .expect("pinned app moves");
    assert_eq!(moved[0].0, "terminal.desktop");
    assert_eq!(moved[1].0, "finder.desktop");
    let bounded = apply_pin_command(
        &moved,
        &PinCommand::Move {
            app_id: "terminal.desktop".into(),
            direction: MoveDirection::Left,
        },
    )
    .expect("edge move is a no-op");
    assert_eq!(bounded, moved);
    let unpinned = apply_pin_command(
        &bounded,
        &PinCommand::Unpin {
            app_id: "finder".into(),
        },
    )
    .expect("pin is removed");
    assert_eq!(
        unpinned,
        [rmac_shell_settings::AppId("terminal.desktop".into())]
    );
}

#[test]
fn reorder_rejects_an_app_that_is_not_pinned() {
    let error = apply_pin_command(
        &[],
        &PinCommand::Move {
            app_id: "terminal.desktop".into(),
            direction: MoveDirection::Right,
        },
    )
    .expect_err("missing pin cannot move");
    assert_eq!(
        error,
        PinError::NotPinned {
            app_id: "terminal.desktop".into()
        }
    );
}

#[test]
fn drag_reorder_moves_to_a_bounded_persisted_index() {
    let pinned = vec![
        rmac_shell_settings::AppId("finder.desktop".into()),
        rmac_shell_settings::AppId("terminal.desktop".into()),
        rmac_shell_settings::AppId("notes.desktop".into()),
    ];
    let moved = apply_pin_command(
        &pinned,
        &PinCommand::MoveTo {
            app_id: "finder".into(),
            index: usize::MAX,
        },
    )
    .expect("drag target is bounded");
    assert_eq!(
        moved
            .iter()
            .map(|app_id| app_id.0.as_str())
            .collect::<Vec<_>>(),
        ["terminal.desktop", "notes.desktop", "finder.desktop"]
    );
}
