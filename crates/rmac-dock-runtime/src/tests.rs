//! Focused Dock runtime orchestration contracts.

use std::path::PathBuf;

use super::*;

fn app(id: &str) -> rmac_apps::Application {
    rmac_apps::Application {
        id: id.into(),
        name: id.trim_end_matches(".desktop").into(),
        generic_name: None,
        keywords: Vec::new(),
        source: PathBuf::from(format!("/apps/{id}")),
        icon: None,
        categories: Vec::new(),
        mime_types: Vec::new(),
        launch: rmac_apps::LaunchSpec::Command {
            program: id.into(),
            args: Vec::new(),
            working_dir: None,
            terminal: false,
        },
        actions: Vec::new(),
    }
}

fn places_report(downloads: &str, trash_count: usize) -> rmac_places_system::Report {
    rmac_places_system::Report {
        snapshot: rmac_places::Snapshot {
            home: rmac_places::Place {
                path: PathBuf::from("/home/alex"),
                exists: true,
            },
            downloads: rmac_places::Place {
                path: PathBuf::from(downloads),
                exists: true,
            },
            downloads_configured: true,
            trash: rmac_places::TrashSnapshot {
                available: true,
                empty: trash_count == 0,
                item_count: trash_count,
            },
        },
        warnings: Vec::new(),
    }
}

#[test]
fn coordinator_waits_for_every_source_to_resolve() {
    let mut coordinator = Coordinator::default();
    assert!(!coordinator.ready());
    coordinator.apply_settings(Ok(Default::default()));
    coordinator.apply_catalog(Err("catalog unavailable".into()));
    assert!(!coordinator.ready());
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Disconnected,
    });
    assert!(!coordinator.ready());
    coordinator.apply_places(Err("places unavailable".into()));
    assert!(!coordinator.ready());
    coordinator.apply_appearance(Err("appearance unavailable".into()));
    assert!(coordinator.ready());
}

#[test]
fn primary_scope_waits_for_display_authority() {
    let mut coordinator = Coordinator::default();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.outputs = rmac_shell_settings::OutputScope::Primary;
    coordinator.apply_settings(Ok(settings.clone()));
    coordinator.apply_catalog(Ok(Vec::new()));
    coordinator.apply_places(Ok(places_report("/home/alex/Downloads", 0)));
    coordinator.apply_appearance(Ok(false));
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("eDP-1"), output("DP-1")],
    });

    assert!(!coordinator.ready());
    coordinator.apply_primary_output(Ok(Some(rmac_compositor::OutputId::from("DP-1"))));

    assert!(coordinator.ready());
    assert_eq!(
        coordinator.snapshot().outputs,
        [rmac_compositor::OutputId::from("DP-1")]
    );
}

#[test]
fn display_failure_retains_last_known_primary_output() {
    let mut coordinator = Coordinator::default();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.outputs = rmac_shell_settings::OutputScope::Primary;
    coordinator.apply_settings(Ok(settings));
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("eDP-1"), output("DP-1")],
    });
    coordinator.apply_primary_output(Ok(Some(rmac_compositor::OutputId::from("eDP-1"))));

    coordinator.apply_primary_output(Err("display service unavailable".into()));

    assert_eq!(
        coordinator.snapshot().outputs,
        [rmac_compositor::OutputId::from("eDP-1")]
    );
    assert!(matches!(
        coordinator.snapshot().health.displays,
        SourceHealth::Unavailable { .. }
    ));
}

#[test]
fn catalog_failure_retains_last_known_good_items() {
    let mut coordinator = Coordinator::default();
    let settings = rmac_shell_settings::ShellSettings {
        pinned_apps: vec![rmac_shell_settings::AppId("finder.desktop".into())],
        ..Default::default()
    };
    coordinator.apply_settings(Ok(settings));
    coordinator.apply_catalog(Ok(vec![app("finder.desktop")]));
    let item = coordinator.snapshot().model.items[0].clone();
    coordinator.apply_catalog(Err("filesystem watch failed".into()));
    assert_eq!(coordinator.snapshot().model.items[0], item);
    assert!(matches!(
        coordinator.snapshot().health.catalog,
        SourceHealth::Unavailable { .. }
    ));
}

#[test]
fn settings_and_compositor_changes_rebuild_the_authoritative_model() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_catalog(Ok(vec![app("terminal.desktop")]));
    let settings = rmac_shell_settings::ShellSettings {
        pinned_apps: vec![rmac_shell_settings::AppId("terminal.desktop".into())],
        ..Default::default()
    };
    coordinator.apply_settings(Ok(settings));
    assert!(!coordinator.snapshot().model.items[0].running);
    assert_eq!(
        coordinator.snapshot().content.applications[0].activity,
        rmac_dock::presentation::ActivityIndicator::None
    );

    coordinator.apply_compositor(rmac_compositor::Event::WindowsReplaced {
        windows: vec![rmac_compositor::Window {
            id: rmac_compositor::WindowId(5),
            title: Some("Terminal".into()),
            app_id: Some("terminal".into()),
            pid: None,
            workspace: None,
            focused: true,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: Default::default(),
        }],
    });
    coordinator.apply_compositor(rmac_compositor::Event::FocusChanged {
        focus: rmac_compositor::FocusState {
            target: Some(rmac_compositor::FocusTarget::Window(
                rmac_compositor::WindowId(5),
            )),
            window: Some(rmac_compositor::WindowId(5)),
            ..Default::default()
        },
    });
    assert!(coordinator.snapshot().model.items[0].running);
    assert!(coordinator.snapshot().model.items[0].active);
    assert_eq!(
        coordinator.snapshot().content.applications[0].activity,
        rmac_dock::presentation::ActivityIndicator::Active
    );
    assert_eq!(
        coordinator.snapshot().content.applications[0].accessible_label,
        "terminal, active, 1 window"
    );
}

#[test]
fn show_running_indicators_off_hides_the_dot_without_changing_running_state() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_catalog(Ok(vec![app("terminal.desktop")]));
    let mut settings = rmac_shell_settings::ShellSettings {
        pinned_apps: vec![rmac_shell_settings::AppId("terminal.desktop".into())],
        ..Default::default()
    };
    settings.dock.show_running_indicators = false;
    coordinator.apply_settings(Ok(settings));
    coordinator.apply_compositor(rmac_compositor::Event::WindowsReplaced {
        windows: vec![rmac_compositor::Window {
            id: rmac_compositor::WindowId(5),
            title: Some("Terminal".into()),
            app_id: Some("terminal".into()),
            pid: None,
            workspace: None,
            focused: true,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: Default::default(),
        }],
    });

    // The Dock still knows the app is running (grouping, badges, the Dock
    // menu's "Quit" and the accessible label all stay truthful); only the
    // visual dot is suppressed.
    assert!(coordinator.snapshot().model.items[0].running);
    assert_eq!(
        coordinator.snapshot().content.applications[0].activity,
        rmac_dock::presentation::ActivityIndicator::None
    );
    assert!(coordinator.snapshot().content.applications[0]
        .accessible_label
        .contains("running"));
}

fn quit_after_running(show_recent_apps: bool) -> Coordinator {
    let mut coordinator = Coordinator::default();
    coordinator.apply_catalog(Ok(vec![app("notes.desktop")]));
    coordinator.apply_places(Ok(places_report("/home/alex/Downloads", 0)));
    coordinator.apply_appearance(Ok(false));
    // An empty Dock (no default pinned apps) so items[0] is unambiguously
    // the one app this test runs and quits, not one of the profile's
    // deliberate first-party pinned apps (which happens to include Notes
    // under a different, unrelated app identity).
    let mut settings = rmac_shell_settings::ShellSettings {
        pinned_apps: Vec::new(),
        ..Default::default()
    };
    settings.dock.show_recent_apps = show_recent_apps;
    coordinator.apply_settings(Ok(settings));
    let window = rmac_compositor::Window {
        id: rmac_compositor::WindowId(9),
        title: Some("Notes".into()),
        app_id: Some("notes".into()),
        pid: None,
        workspace: None,
        focused: true,
        floating: false,
        urgent: false,
        focus_timestamp: None,
        layout: Default::default(),
    };
    coordinator.apply_compositor(rmac_compositor::Event::WindowsReplaced {
        windows: vec![window],
    });
    assert!(coordinator.ready());
    assert!(coordinator.snapshot().model.items[0].running);
    coordinator.apply_compositor(rmac_compositor::Event::WindowsReplaced {
        windows: Vec::new(),
    });
    coordinator
}

#[test]
fn show_recent_apps_off_drops_a_quit_application_immediately() {
    let coordinator = quit_after_running(false);
    assert!(coordinator
        .snapshot()
        .model
        .items
        .iter()
        .all(|item| item.id != "notes.desktop"));
}

#[test]
fn show_recent_apps_on_keeps_a_quit_application_in_the_recents_section() {
    let coordinator = quit_after_running(true);
    assert!(coordinator
        .snapshot()
        .model
        .items
        .iter()
        .any(|item| item.id == "notes.desktop" && !item.running));
}

fn output(id: &str) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: String::new(),
        model: String::new(),
        serial: None,
        physical_size_mm: None,
        modes: Vec::new(),
        current_mode: Some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: Some(rmac_compositor::LogicalOutput {
            position: Default::default(),
            size: rmac_compositor::LogicalSize {
                width: 1920.0,
                height: 1080.0,
            },
            scale: 1.0,
            transform: "normal".into(),
        }),
    }
}

#[test]
fn output_hotplug_changes_only_authoritative_surface_candidates() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("eDP-1")],
    });
    assert_eq!(
        coordinator.snapshot().outputs,
        [rmac_compositor::OutputId::from("eDP-1")]
    );
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: Vec::new(),
    });
    assert!(coordinator.snapshot().outputs.is_empty());
}

#[test]
fn renderer_policy_changes_request_a_frame_without_changing_dock_items() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("eDP-1")],
    });
    let before = coordinator.snapshot();

    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.placement = rmac_shell_settings::DockPlacement::Left;
    settings.dock.autohide = true;
    settings.dock.reserve_space = false;
    settings.dock.magnification_scale = 2.0;
    coordinator.apply_settings(Ok(settings.clone()));
    let after = coordinator.snapshot();

    assert_eq!(after.settings, settings.dock);
    assert_eq!(after.model, before.model);
    assert_eq!(after.outputs, before.outputs);
    assert_ne!(after.surface_plan, before.surface_plan);
    assert!(publication(Some(&before), after.clone()).visible);
    let surface = &after
        .surface_plan
        .expect("valid settings produce a surface plan")[0];
    assert_eq!(surface.output, rmac_compositor::OutputId::from("eDP-1"));
    assert_eq!(surface.placement, rmac_shell_settings::DockPlacement::Left);
    assert_eq!(surface.output_axis_length, 1080.0);
    assert_eq!(surface.exclusive_zone, 0.0);
    assert_eq!(
        surface.reveal_edge_thickness,
        rmac_dock::HIDDEN_EDGE_THICKNESS
    );
}

#[test]
fn overview_state_crosses_the_runtime_boundary_and_requests_a_frame() {
    let mut coordinator = Coordinator::default();
    let before = coordinator.snapshot();
    assert!(!before.overview_visible);

    coordinator.apply_compositor(rmac_compositor::Event::OverviewChanged { visible: true });
    let after = coordinator.snapshot();
    assert!(after.overview_visible);
    assert!(publication(Some(&before), after).visible);
}

#[test]
fn reduced_motion_is_authoritative_and_failure_retains_last_known_good() {
    let mut coordinator = Coordinator::default();
    let before = coordinator.snapshot();
    assert!(!before.reduced_motion);

    coordinator.apply_appearance(Ok(true));
    let reduced = coordinator.snapshot();
    assert!(reduced.reduced_motion);
    assert!(publication(Some(&before), reduced.clone()).visible);

    coordinator.apply_appearance(Err("private portal diagnostic".into()));
    let failed = coordinator.snapshot();
    assert!(failed.reduced_motion);
    assert!(!publication(Some(&reduced), failed.clone()).visible);
    assert!(matches!(
        failed.health.appearance,
        SourceHealth::Unavailable { .. }
    ));
    assert!(!format!("{:?}", failed.health).contains("private portal"));
}

#[test]
fn health_only_publication_does_not_request_a_dock_frame() {
    let previous = Snapshot::default();
    let mut next = previous.clone();
    next.health.catalog = SourceHealth::Unavailable {
        detail: "catalog watcher restarted".into(),
    };
    let update = publication(Some(&previous), next);
    assert!(!update.visible);
}

#[test]
fn place_changes_rebuild_special_items_and_failures_keep_last_known_good() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_places(Ok(places_report("/home/alex/Downloads", 2)));
    let before = coordinator.snapshot();
    assert_eq!(before.model.special_items[0].item_count, Some(2));

    coordinator.apply_places(Err("/home/alex/private trash failed".into()));
    let failed = coordinator.snapshot();
    assert_eq!(failed.model.special_items, before.model.special_items);
    assert!(matches!(
        failed.health.places,
        SourceHealth::Unavailable { .. }
    ));
    let debug = format!("{:?}", failed.health);
    assert!(!debug.contains("alex"));

    coordinator.apply_places(Ok(places_report("/home/alex/Transfers", 0)));
    let refreshed = coordinator.snapshot();
    assert_eq!(refreshed.model.special_items[0].item_count, Some(0));
    assert_ne!(refreshed.model.special_items, before.model.special_items);
    assert!(publication(Some(&before), refreshed).visible);
}

#[test]
fn snapshot_resolves_configured_stacks_against_the_live_filesystem() {
    // Regression: Coordinator::snapshot() only ever called
    // Model::build_with_places, so model.stacks stayed empty no matter what
    // dock_stacks held (§ folder/file stacks left of the Trash).
    let mut coordinator = Coordinator::default();
    coordinator.apply_places(Ok(places_report("/home/alex/Downloads", 0)));
    let settings = rmac_shell_settings::ShellSettings {
        dock_stacks: vec![
            rmac_shell_settings::DockStackEntry {
                kind: rmac_shell_settings::DockStackKind::Downloads,
                display_as: rmac_shell_settings::DockStackDisplayAs::default(),
                view_content_as: rmac_shell_settings::DockStackViewContentAs::default(),
                sort_by: rmac_shell_settings::DockStackSortBy::default(),
            },
            rmac_shell_settings::DockStackEntry {
                kind: rmac_shell_settings::DockStackKind::Path {
                    path: "/home/alex/does-not-exist".into(),
                },
                display_as: rmac_shell_settings::DockStackDisplayAs::default(),
                view_content_as: rmac_shell_settings::DockStackViewContentAs::default(),
                sort_by: rmac_shell_settings::DockStackSortBy::default(),
            },
        ],
        ..Default::default()
    };
    coordinator.apply_settings(Ok(settings));
    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.model.stacks.len(), 2);
    assert!(snapshot.model.stacks[0].available, "Downloads exists");
    assert!(
        !snapshot.model.stacks[1].available,
        "the configured path does not exist"
    );
}

#[test]
fn display_refresh_hints_are_narrow_and_forward_compatible() {
    assert!(compositor_event_affects_displays(
        &rmac_compositor::Event::OutputsReplaced {
            outputs: Vec::new(),
        }
    ));
    assert!(compositor_event_affects_displays(
        &rmac_compositor::Event::Unknown {
            source_kind: "ConfigLoaded".into(),
            payload: Default::default(),
        }
    ));
    assert!(!compositor_event_affects_displays(
        &rmac_compositor::Event::WindowsReplaced {
            windows: Vec::new(),
        }
    ));
}
