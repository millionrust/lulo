use std::path::PathBuf;

use super::*;

fn output(id: &str, enabled: bool, scale: f64) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: "Test".into(),
        model: "Display".into(),
        serial: None,
        physical_size_mm: None,
        modes: vec![rmac_compositor::OutputMode {
            physical_size: rmac_compositor::PhysicalSize {
                width: 2560,
                height: 1440,
            },
            refresh_millihz: 60_000,
            preferred: true,
        }],
        current_mode: enabled.then_some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: enabled.then_some(rmac_compositor::LogicalOutput {
            position: Default::default(),
            size: rmac_compositor::LogicalSize {
                width: 1280.0,
                height: 720.0,
            },
            scale,
            transform: "normal".into(),
        }),
    }
}

#[test]
fn source_parser_accepts_original_builtin_and_safe_local_forms_only() {
    assert_eq!(parse_source(None), Ok(Source::BuiltIn(BuiltInId::Aurora)));
    assert_eq!(
        parse_source(Some("builtin:rmac-aurora")),
        Ok(Source::BuiltIn(BuiltInId::Aurora))
    );
    assert_eq!(
        parse_source(Some("file:///home/alex/Pictures/sky%20blue.png")),
        Ok(Source::File(PathBuf::from(
            "/home/alex/Pictures/sky blue.png"
        )))
    );
    assert!(matches!(
        parse_source(Some("https://example.com/wallpaper.jpg")),
        Err(SourceErrorKind::UnsupportedScheme)
    ));
    assert!(matches!(
        parse_source(Some("../secret.png")),
        Err(SourceErrorKind::RelativePath)
    ));
    let private = parse_source(Some("/home/alex/Private/wallpaper.png")).unwrap();
    assert!(!format!("{private:?}").contains("alex"));
    assert!(DEFAULT_BUILT_IN.metadata().attribution.contains("rmac"));
}

#[test]
fn plan_is_per_output_sorted_hotplug_safe_and_locally_fallbacks() {
    let settings = rmac_shell_settings::WallpaperSettings {
        default: rmac_shell_settings::WallpaperSelection {
            source: Some("builtin:rmac-aurora".into()),
            fit: rmac_shell_settings::WallpaperFit::Fill,
        },
        per_output: [(
            "DP-2".into(),
            rmac_shell_settings::WallpaperSelection {
                source: Some("https://invalid.example/image".into()),
                fit: rmac_shell_settings::WallpaperFit::Fit,
            },
        )]
        .into_iter()
        .collect(),
    };
    let initial = plan(
        &settings,
        &[
            output("DP-2", true, 2.0),
            output("DP-1", true, 1.0),
            output("DP-3", false, 1.0),
        ],
    );
    assert_eq!(
        initial
            .surfaces
            .iter()
            .map(|surface| surface.output.0.as_str())
            .collect::<Vec<_>>(),
        ["DP-1", "DP-2"]
    );
    assert_eq!(initial.issues.len(), 1);
    assert_eq!(initial.issues[0].output.0, "DP-2");
    assert_eq!(
        initial.surfaces[1].source,
        Source::BuiltIn(BuiltInId::Aurora)
    );
    assert_eq!(
        initial.surfaces[1].fit,
        rmac_shell_settings::WallpaperFit::Fit
    );

    let unplugged = plan(&settings, &[output("DP-1", true, 1.0)]);
    assert_eq!(unplugged.surfaces.len(), 1);
    let replugged = plan(&settings, &[output("DP-2", true, 2.0)]);
    assert_eq!(
        replugged.surfaces[0].fit,
        rmac_shell_settings::WallpaperFit::Fit
    );
}

#[test]
fn fit_geometry_covers_crop_letterbox_stretch_center_and_tile() {
    let image = rmac_compositor::PhysicalSize {
        width: 1000,
        height: 1000,
    };
    let viewport = rmac_compositor::LogicalSize {
        width: 1000.0,
        height: 500.0,
    };
    let fill = layout(
        rmac_shell_settings::WallpaperFit::Fill,
        image,
        viewport,
        1.0,
    )
    .unwrap();
    assert_eq!(
        fill.destination,
        Rect {
            x: 0.0,
            y: -250.0,
            width: 1000.0,
            height: 1000.0
        }
    );
    let fit = layout(rmac_shell_settings::WallpaperFit::Fit, image, viewport, 1.0).unwrap();
    assert_eq!(
        fit.destination,
        Rect {
            x: 250.0,
            y: 0.0,
            width: 500.0,
            height: 500.0
        }
    );
    let stretch = layout(
        rmac_shell_settings::WallpaperFit::Stretch,
        image,
        viewport,
        1.0,
    )
    .unwrap();
    assert_eq!(stretch.destination.width, viewport.width);
    assert_eq!(stretch.destination.height, viewport.height);
    assert_eq!(
        layout(
            rmac_shell_settings::WallpaperFit::Center,
            image,
            viewport,
            2.0
        )
        .unwrap()
        .destination,
        Rect {
            x: 250.0,
            y: 0.0,
            width: 500.0,
            height: 500.0
        }
    );
    assert!(
        layout(
            rmac_shell_settings::WallpaperFit::Tile,
            image,
            viewport,
            2.0
        )
        .unwrap()
        .tiled
    );
}

#[test]
fn every_built_in_round_trips_and_has_a_distinct_palette() {
    let mut ids = std::collections::BTreeSet::new();
    let mut palettes = std::collections::BTreeSet::new();
    for id in BuiltInId::ALL {
        assert_eq!(
            BuiltInId::parse(id.id()),
            Some(id),
            "{:?} did not round-trip",
            id
        );
        let metadata = id.metadata();
        assert!(!metadata.title.is_empty());
        assert!(ids.insert(metadata.id.id()));
        assert!(palettes.insert(metadata.palette));
        assert_ne!(
            metadata.palette, metadata.light_palette,
            "{:?} has no light pair",
            id
        );
        assert_eq!(metadata.palette_for(true), metadata.palette);
        assert_eq!(metadata.palette_for(false), metadata.light_palette);
    }
    assert_eq!(ids.len(), BuiltInId::ALL.len());
    assert_eq!(palettes.len(), BuiltInId::ALL.len());
}
