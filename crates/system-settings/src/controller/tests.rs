//! Focused controller boundary and mutation-isolation contracts.

use super::{
    audio_change_needs_followup, audio_stream_snapshot_is_current,
    bluetooth_stream_snapshot_is_current, composite_wallpaper_pixel,
    gtk_text_stream_snapshot_is_current, input_stream_snapshot_is_current,
    locale_stream_snapshot_is_current, login_items_stream_snapshot_is_current,
    network_stream_snapshot_is_current, power_change_needs_followup,
    power_stream_snapshot_is_current, privacy_stream_snapshot_is_current, render_wallpaper_preview,
    shortcut_configuration_available, storage_stream_snapshot_is_current,
    system_info_stream_snapshot_is_current, theme_stream_snapshot_is_current,
    time_stream_snapshot_is_current, update_stream_snapshot_is_current,
    vpn_stream_snapshot_is_current, wallpaper_selection, wifi_stream_snapshot_is_current,
    DockChange, ShellSettingsMutation, SpotlightAuthority, SpotlightChange, WallpaperChange,
    WallpaperTarget,
};

#[test]
fn input_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(input_stream_snapshot_is_current(4, 4, false, false));
    assert!(!input_stream_snapshot_is_current(3, 4, false, false));
    assert!(!input_stream_snapshot_is_current(4, 4, true, false));
    assert!(!input_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn system_info_stream_snapshots_cannot_cross_hostname_transactions() {
    assert!(system_info_stream_snapshot_is_current(4, 4, false, false));
    assert!(!system_info_stream_snapshot_is_current(3, 4, false, false));
    assert!(!system_info_stream_snapshot_is_current(4, 4, true, false));
    assert!(!system_info_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn update_stream_snapshots_cannot_cross_install_transactions() {
    assert!(update_stream_snapshot_is_current(4, 4, false, false));
    assert!(!update_stream_snapshot_is_current(3, 4, false, false));
    assert!(!update_stream_snapshot_is_current(4, 4, true, false));
    assert!(!update_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn storage_stream_snapshots_cannot_cross_manual_refreshes() {
    assert!(storage_stream_snapshot_is_current(4, 4, false, false));
    assert!(!storage_stream_snapshot_is_current(3, 4, false, false));
    assert!(!storage_stream_snapshot_is_current(4, 4, true, false));
    assert!(!storage_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn time_stream_snapshots_cannot_cross_clock_transactions() {
    assert!(time_stream_snapshot_is_current(4, 4, false, false));
    assert!(!time_stream_snapshot_is_current(3, 4, false, false));
    assert!(!time_stream_snapshot_is_current(4, 4, true, false));
    assert!(!time_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn locale_stream_snapshots_cannot_cross_locale_transactions() {
    assert!(locale_stream_snapshot_is_current(4, 4, false, false));
    assert!(!locale_stream_snapshot_is_current(3, 4, false, false));
    assert!(!locale_stream_snapshot_is_current(4, 4, true, false));
    assert!(!locale_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn login_item_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(login_items_stream_snapshot_is_current(4, 4, false, false));
    assert!(!login_items_stream_snapshot_is_current(3, 4, false, false));
    assert!(!login_items_stream_snapshot_is_current(4, 4, true, false));
    assert!(!login_items_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn gtk_text_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(gtk_text_stream_snapshot_is_current(4, 4, false, false));
    assert!(!gtk_text_stream_snapshot_is_current(3, 4, false, false));
    assert!(!gtk_text_stream_snapshot_is_current(4, 4, true, false));
    assert!(!gtk_text_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn theme_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(theme_stream_snapshot_is_current(4, 4, false, false));
    assert!(!theme_stream_snapshot_is_current(3, 4, false, false));
    assert!(!theme_stream_snapshot_is_current(4, 4, true, false));
    assert!(!theme_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn privacy_stream_snapshots_cannot_cross_reset_generations() {
    assert!(privacy_stream_snapshot_is_current(4, 4, false, false));
    assert!(!privacy_stream_snapshot_is_current(3, 4, false, false));
    assert!(!privacy_stream_snapshot_is_current(4, 4, true, false));
    assert!(!privacy_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn wifi_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(wifi_stream_snapshot_is_current(7, 7, false, false));
    assert!(!wifi_stream_snapshot_is_current(6, 7, false, false));
    assert!(!wifi_stream_snapshot_is_current(7, 7, true, false));
    assert!(!wifi_stream_snapshot_is_current(7, 7, false, true));
}

#[test]
fn bluetooth_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(bluetooth_stream_snapshot_is_current(11, 11, false, false));
    assert!(!bluetooth_stream_snapshot_is_current(10, 11, false, false));
    assert!(!bluetooth_stream_snapshot_is_current(11, 11, true, false));
    assert!(!bluetooth_stream_snapshot_is_current(11, 11, false, true));
}

#[test]
fn network_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(network_stream_snapshot_is_current(5, 5, false, false));
    assert!(!network_stream_snapshot_is_current(4, 5, false, false));
    assert!(!network_stream_snapshot_is_current(5, 5, true, false));
    assert!(!network_stream_snapshot_is_current(5, 5, false, true));
}

#[test]
fn vpn_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(vpn_stream_snapshot_is_current(3, 3, false, false));
    assert!(!vpn_stream_snapshot_is_current(2, 3, false, false));
    assert!(!vpn_stream_snapshot_is_current(3, 3, true, false));
    assert!(!vpn_stream_snapshot_is_current(3, 3, false, true));
}

#[test]
fn power_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(power_stream_snapshot_is_current(9, 9, false, false));
    assert!(!power_stream_snapshot_is_current(8, 9, false, false));
    assert!(!power_stream_snapshot_is_current(9, 9, true, false));
    assert!(!power_stream_snapshot_is_current(9, 9, false, true));
}

#[test]
fn audio_stream_snapshots_cannot_cross_mutation_generations() {
    assert!(audio_stream_snapshot_is_current(4, 4, false, false));
    assert!(!audio_stream_snapshot_is_current(3, 4, false, false));
    assert!(!audio_stream_snapshot_is_current(4, 4, true, false));
    assert!(!audio_stream_snapshot_is_current(4, 4, false, true));
}

#[test]
fn audio_changes_retain_recovery_without_duplicating_initial_load() {
    assert!(!audio_change_needs_followup(false, true, false));
    assert!(audio_change_needs_followup(false, true, true));
    assert!(audio_change_needs_followup(true, false, false));
    assert!(!audio_change_needs_followup(false, false, true));
}

#[test]
fn power_changes_retain_recovery_without_duplicating_initial_load() {
    assert!(!power_change_needs_followup(false, true, false));
    assert!(power_change_needs_followup(false, true, true));
    assert!(power_change_needs_followup(true, false, false));
    assert!(!power_change_needs_followup(false, false, true));
}

#[test]
fn dock_changes_touch_only_the_selected_policy() {
    let original = rmac_shell_settings::DockSettings::default();

    let mut dock = original.clone();
    DockChange::Placement(rmac_shell_settings::DockPlacement::Left).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            placement: rmac_shell_settings::DockPlacement::Left,
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::Outputs(rmac_shell_settings::OutputScope::Named("DP-1".into())).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            outputs: rmac_shell_settings::OutputScope::Named("DP-1".into()),
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::Autohide(true).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            autohide: true,
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::Magnification(false).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            magnification: false,
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::MagnificationScale(2.0).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            magnification_scale: 2.0,
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::ReserveSpace(false).apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            reserve_space: false,
            ..original.clone()
        }
    );

    let mut dock = original.clone();
    DockChange::RepeatedClick(rmac_shell_settings::RepeatedClickBehavior::DoNothing)
        .apply(&mut dock);
    assert_eq!(
        dock,
        rmac_shell_settings::DockSettings {
            repeated_click: rmac_shell_settings::RepeatedClickBehavior::DoNothing,
            ..original
        }
    );
}

#[test]
fn dock_mutation_preserves_unrelated_shell_settings() {
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.clock.show_seconds = true;
    settings.spotlight.include_removable_mounts = true;

    ShellSettingsMutation::Change(DockChange::Autohide(true)).apply(&mut settings);

    assert!(settings.dock.autohide);
    assert!(settings.clock.show_seconds);
    assert!(settings.spotlight.include_removable_mounts);
}

#[test]
fn spotlight_provider_changes_are_scoped_and_elide_default_policy() {
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.autohide = true;
    let original_wallpaper = settings.wallpaper.clone();
    let provider = rmac_launcher_providers::FILES_PROVIDER.to_string();

    SpotlightChange::ProviderPrivateContent {
        id: provider.clone(),
        allowed: true,
    }
    .apply(&mut settings);
    let policy = settings
        .providers
        .get(&rmac_shell_settings::ProviderId(provider.clone()))
        .unwrap();
    assert!(policy.enabled);
    assert!(policy.allow_private_content);
    assert!(!policy.allow_network);
    assert!(settings.dock.autohide);
    assert_eq!(settings.wallpaper, original_wallpaper);

    SpotlightChange::ProviderPrivateContent {
        id: provider.clone(),
        allowed: false,
    }
    .apply(&mut settings);
    assert!(!settings
        .providers
        .contains_key(&rmac_shell_settings::ProviderId(provider)));
}

#[test]
fn currency_rates_need_their_own_network_permission() {
    let mut settings = rmac_shell_settings::ShellSettings::default();
    let currency = rmac_launcher_providers::CURRENCY_PROVIDER.to_string();
    SpotlightChange::ProviderNetwork {
        id: currency.clone(),
        allowed: true,
    }
    .apply(&mut settings);
    let policy = settings
        .providers
        .get(&rmac_shell_settings::ProviderId(currency.clone()))
        .unwrap();
    assert!(policy.enabled && policy.allow_network && !policy.allow_private_content);
    assert_eq!(settings.providers.len(), 1);

    SpotlightChange::ProviderNetwork {
        id: currency.clone(),
        allowed: false,
    }
    .apply(&mut settings);
    assert!(!settings
        .providers
        .contains_key(&rmac_shell_settings::ProviderId(currency)));
}

#[test]
fn spotlight_configuration_requires_the_live_version_two_portal() {
    assert!(shortcut_configuration_available(Some(
        &rmac_shortcuts::BackendStatus::Portal {
            version: 2,
            can_configure: true,
        }
    )));
    assert!(!shortcut_configuration_available(Some(
        &rmac_shortcuts::BackendStatus::Portal {
            version: 1,
            can_configure: true,
        }
    )));
    assert!(!shortcut_configuration_available(Some(
        &rmac_shortcuts::BackendStatus::Portal {
            version: 2,
            can_configure: false,
        }
    )));
    assert!(!shortcut_configuration_available(Some(
        &rmac_shortcuts::BackendStatus::FallbackRequired {
            reason: "portal unavailable".into(),
        }
    )));
    assert!(!shortcut_configuration_available(None));
}

#[test]
fn spotlight_scope_and_rollback_preserve_unrelated_shell_settings() {
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.reserve_space = false;
    settings.wallpaper.default.source = Some("builtin:rmac-aurora".into());
    let previous = SpotlightAuthority::from_settings(&settings);

    ShellSettingsMutation::Spotlight(SpotlightChange::IncludeRemovableMounts(true))
        .apply(&mut settings);
    ShellSettingsMutation::Spotlight(SpotlightChange::AddExclusion("/home/test/Private".into()))
        .apply(&mut settings);
    ShellSettingsMutation::Spotlight(SpotlightChange::AddExclusion("/home/test/Private".into()))
        .apply(&mut settings);
    assert!(settings.spotlight.include_removable_mounts);
    assert_eq!(settings.spotlight.excluded_paths, ["/home/test/Private"]);
    assert!(!settings.dock.reserve_space);
    assert_eq!(
        settings.wallpaper.default.source.as_deref(),
        Some("builtin:rmac-aurora")
    );

    ShellSettingsMutation::Spotlight(SpotlightChange::RemoveExclusion(
        "/home/test/Private".into(),
    ))
    .apply(&mut settings);
    assert!(settings.spotlight.excluded_paths.is_empty());

    ShellSettingsMutation::RestoreSpotlight(previous).apply(&mut settings);
    assert_eq!(
        settings.spotlight,
        rmac_shell_settings::SpotlightSettings::default()
    );
    assert!(settings.providers.is_empty());
    assert!(!settings.dock.reserve_space);
    assert_eq!(
        settings.wallpaper.default.source.as_deref(),
        Some("builtin:rmac-aurora")
    );
}

#[test]
fn wallpaper_output_changes_clone_the_default_and_preserve_other_settings() {
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.dock.autohide = true;
    settings.wallpaper.default.source = Some("builtin:rmac-aurora".into());
    let original_default = settings.wallpaper.default.clone();

    ShellSettingsMutation::Wallpaper {
        target: WallpaperTarget::Output("DP-1".into()),
        change: WallpaperChange::Fit(rmac_shell_settings::WallpaperFit::Center),
    }
    .apply(&mut settings);

    assert_eq!(settings.wallpaper.default, original_default);
    assert_eq!(
        settings.wallpaper.per_output.get("DP-1"),
        Some(&rmac_shell_settings::WallpaperSelection {
            source: Some("builtin:rmac-aurora".into()),
            fit: rmac_shell_settings::WallpaperFit::Center,
        })
    );
    assert!(settings.dock.autohide);

    ShellSettingsMutation::Wallpaper {
        target: WallpaperTarget::Output("DP-1".into()),
        change: WallpaperChange::UseDefault,
    }
    .apply(&mut settings);
    assert!(!settings.wallpaper.per_output.contains_key("DP-1"));
}

#[test]
fn wallpaper_selection_reports_inheritance_without_inventing_an_override() {
    let wallpaper = rmac_shell_settings::WallpaperSettings::default();
    let (selection, owns_selection) =
        wallpaper_selection(&wallpaper, &WallpaperTarget::Output("HDMI-A-1".into()));
    assert_eq!(selection, wallpaper.default);
    assert!(!owns_selection);
    assert!(wallpaper.per_output.is_empty());
}

#[test]
fn original_wallpaper_preview_uses_the_bounded_renderer() {
    // Aurora is drawn without packaged files, so this runs from a checkout.
    let aurora = rmac_shell_settings::WallpaperSelection {
        source: Some("builtin:rmac-aurora".into()),
        ..Default::default()
    };
    for dark in [true, false] {
        let preview = render_wallpaper_preview(&aurora, dark).unwrap();
        assert_eq!(preview.size(0).width.0, 480);
        assert_eq!(preview.size(0).height.0, 270);
        assert_eq!(preview.as_bytes(0).unwrap().len(), 480 * 270 * 4);
    }
}

#[test]
fn wallpaper_preview_alpha_compositing_does_not_overflow() {
    assert_eq!(
        composite_wallpaper_pixel([255, 64, 0, 128], [0, 0, 32, 255]),
        [128, 32, 15, 255]
    );
    assert_eq!(
        composite_wallpaper_pixel([1, 2, 3, 0], [20, 30, 40, 255]),
        [20, 30, 40, 255]
    );
}
