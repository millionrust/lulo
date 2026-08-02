use super::*;
use crate::watch::selected_file_paths;

fn output(id: &str) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: "Test".into(),
        model: "Display".into(),
        serial: None,
        physical_size_mm: None,
        modes: vec![rmac_compositor::OutputMode {
            physical_size: rmac_compositor::PhysicalSize {
                width: 1920,
                height: 1080,
            },
            refresh_millihz: 60_000,
            preferred: true,
        }],
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

fn settings(source: &str) -> rmac_shell_settings::ShellSettings {
    rmac_shell_settings::ShellSettings {
        wallpaper: rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some(source.into()),
                fit: rmac_shell_settings::WallpaperFit::Fill,
            },
            per_output: Default::default(),
        },
        ..Default::default()
    }
}

#[test]
fn waits_for_both_authorities_and_builds_hotplug_plans() {
    let mut coordinator = Coordinator::default();
    assert!(!coordinator.ready());
    coordinator.apply_settings(Ok(settings("builtin:rmac-aurora")));
    assert!(!coordinator.ready());
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Connected,
    });
    assert!(coordinator.ready());
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("DP-2"), output("DP-1")],
    });
    assert_eq!(
        coordinator
            .snapshot()
            .plan
            .surfaces
            .iter()
            .map(|surface| surface.output.0.as_str())
            .collect::<Vec<_>>(),
        ["DP-1", "DP-2"]
    );
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("DP-1")],
    });
    assert_eq!(coordinator.snapshot().plan.surfaces.len(), 1);
}

#[test]
fn source_failures_retain_last_known_good_visible_plan() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_settings(Ok(settings("builtin:rmac-aurora")));
    coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("DP-1")],
    });
    let visible = coordinator.snapshot().plan;
    assert!(coordinator.apply_settings(Err("private settings path".into())));
    let failed = coordinator.snapshot();
    assert_eq!(failed.plan, visible);
    assert!(matches!(
        failed.health.settings,
        SourceHealth::Unavailable { .. }
    ));
    assert!(!format!("{:?}", failed.health).contains("private settings"));

    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Disconnected,
    });
    assert_eq!(coordinator.snapshot().plan, visible);
}

#[test]
fn update_debug_redacts_health_and_file_sources() {
    let plan = rmac_wallpaper::Plan {
        surfaces: vec![rmac_wallpaper::Surface {
            output: "DP-1".into(),
            logical_size: rmac_compositor::LogicalSize {
                width: 1.0,
                height: 1.0,
            },
            scale: 1.0,
            fit: rmac_shell_settings::WallpaperFit::Fill,
            source: rmac_wallpaper::Source::File("/home/alex/private.png".into()),
        }],
        issues: Vec::new(),
    };
    let update = Update::Render {
        rasterized: rmac_wallpaper_image::rasterize(&plan, &rmac_wallpaper_image::Cache::default()),
        plan,
        health: HealthSnapshot {
            compositor: SourceHealth::Unavailable {
                detail: "secret socket path".into(),
            },
            settings: SourceHealth::Healthy,
            files: SourceHealth::Healthy,
        },
    };
    let debug = format!("{update:?}");
    assert!(!debug.contains("alex"));
    assert!(!debug.contains("secret socket"));
    assert_eq!(
        selected_file_paths(match &update {
            Update::Render { plan, .. } => plan,
            Update::Health(_) => unreachable!(),
        })
        .len(),
        1
    );
}
