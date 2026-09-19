use super::*;
use std::path::PathBuf;

fn host(available: bool) -> rmac_appearance::Snapshot {
    rmac_appearance::Snapshot {
        available,
        color_scheme: rmac_appearance::ColorScheme::PreferDark,
        capabilities: rmac_appearance::Capabilities {
            color_scheme: true,
            ..rmac_appearance::Capabilities::default()
        },
        ..rmac_appearance::Snapshot::default()
    }
}

fn theme(accent: rmac_theme::AccentPreference) -> rmac_theme::Snapshot {
    rmac_theme::Snapshot {
        preferences: rmac_theme::Preferences {
            color_scheme: rmac_theme::SchemePreference::Dark,
            accent_color: accent,
            contrast: rmac_theme::ContrastPreference::Higher,
            motion: rmac_theme::MotionPreferenceSetting::Reduced,
            text_scale: rmac_theme::TextScalePreference::Large,
            allow_wallpaper_tinting: true,
        },
        effective: rmac_appearance::ResolvedAppearance {
            color_scheme: rmac_appearance::ResolvedColorScheme::Dark,
            accent_color: rmac_appearance::AccentColor::new(
                19.0 / 255.0,
                114.0 / 255.0,
                249.0 / 255.0,
            )
            .unwrap(),
            contrast: rmac_appearance::Contrast::Higher,
            motion: rmac_appearance::MotionPreference::Reduced,
            text_scale: rmac_appearance::TextScale::Large,
        },
        path: PathBuf::from("/private/theme-8472.json"),
        recovered_from_last_good: false,
        detail: Some("Private recovery detail 8472".into()),
    }
}

#[test]
fn ready_projection_matches_visual_order_selection_and_typed_actions() {
    let host = host(true);
    let theme = theme(rmac_theme::AccentPreference::Custom(components_from_hex(
        0x1372f9,
    )));
    let snapshot = project_appearance(AppearanceInput {
        theme: Some(&theme),
        host: &host,
        loading: false,
        busy: false,
        refreshing: false,
        error: None,
    })
    .unwrap();

    assert_eq!(snapshot.state, PaneState::Ready);
    assert_eq!(snapshot.groups.len(), 4);
    assert_eq!(
        snapshot
            .groups
            .iter()
            .map(|group| group.kind)
            .collect::<Vec<_>>(),
        vec![
            ChoiceGroupKind::Scheme,
            ChoiceGroupKind::Accent,
            ChoiceGroupKind::Contrast,
            ChoiceGroupKind::Motion,
        ]
    );
    assert_eq!(snapshot.groups[1].value_text, "Blue");
    assert_eq!(
        snapshot.groups[1]
            .choices
            .iter()
            .filter(|choice| choice.selected)
            .map(|choice| choice.kind)
            .collect::<Vec<_>>(),
        vec![AppearanceAction::SetAccent(AccentChoice::Blue)]
    );
    assert_eq!(snapshot.keyboard_order.len(), 19);
    assert_eq!(snapshot.initial_focus.as_deref(), Some(REFRESH_ID));
    assert_eq!(
        snapshot.authority.as_ref().unwrap().effective_appearance,
        "Dark"
    );
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("/private"));
    assert!(!debug.contains("Private recovery"));
    assert!(!debug.contains("8472"));
}

#[test]
fn loading_busy_unavailable_and_custom_accent_states_are_truthful() {
    let unavailable_host = host(false);
    let loading = project_appearance(AppearanceInput {
        theme: None,
        host: &unavailable_host,
        loading: true,
        busy: false,
        refreshing: false,
        error: None,
    })
    .unwrap();
    assert_eq!(loading.state, PaneState::Loading);
    assert!(loading.keyboard_order.is_empty());
    assert_eq!(loading.announcements.len(), 2);

    let theme = theme(rmac_theme::AccentPreference::Custom([0.1, 0.2, 0.3]));
    let busy = project_appearance(AppearanceInput {
        theme: Some(&theme),
        host: &unavailable_host,
        loading: false,
        busy: true,
        refreshing: false,
        error: Some("Private mutation failure 8472"),
    })
    .unwrap();
    assert_eq!(busy.state, PaneState::Busy);
    assert_eq!(busy.groups[1].value_text, "Custom");
    assert!(busy.groups[1]
        .choices
        .iter()
        .all(|choice| !choice.selected && !choice.enabled));
    assert!(busy.keyboard_order.is_empty());
    assert_eq!(busy.announcements.len(), 3);
    assert!(!format!("{busy:?}").contains("Private mutation"));

    let unavailable = project_appearance(AppearanceInput {
        theme: None,
        host: &host(true),
        loading: false,
        busy: false,
        refreshing: false,
        error: None,
    })
    .unwrap();
    assert_eq!(unavailable.state, PaneState::Unavailable);
    assert_eq!(unavailable.keyboard_order, vec![REFRESH_ID.to_owned()]);
    assert_eq!(
        unavailable.announcements[0].politeness,
        LivePoliteness::Assertive
    );
}

#[test]
fn malformed_accent_and_oversized_error_fail_closed() {
    let host = host(true);
    let invalid = theme(rmac_theme::AccentPreference::Custom([f64::NAN, 0.0, 0.0]));
    assert_eq!(
        project_appearance(AppearanceInput {
            theme: Some(&invalid),
            host: &host,
            loading: false,
            busy: false,
            refreshing: false,
            error: None,
        }),
        Err(AccessibilityProjectionError::InvalidAccent)
    );
    let error = "x".repeat(MAX_TEXT_VALUE_BYTES + 1);
    assert_eq!(
        project_appearance(AppearanceInput {
            theme: None,
            host: &host,
            loading: false,
            busy: false,
            refreshing: false,
            error: Some(&error),
        }),
        Err(AccessibilityProjectionError::TextValueLimit)
    );
}
