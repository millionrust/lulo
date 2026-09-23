//! System Settings Dock, Wallpaper, shortcut, and Spotlight control-row projection.

use super::*;

pub(in crate::controller) fn dock_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [DockOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = options
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, (option_label, change))| {
            let option_view = view.clone();
            choice(option_label, selected == Some(index), move |_, cx| {
                option_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(change.clone(), cx)
                });
            })
        })
        .collect();
    let current = popup_value(&choices, "Custom");
    popup_row(id, title, None, current, choices, enabled)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn dock_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: Option<SharedString>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> DockChange,
) -> AnyElement {
    let toggle_view = view.clone();
    let toggle = Toggle::new(id)
        .checked(checked)
        .disabled(!enabled)
        .on_click(move |value, _, cx| {
            toggle_view.update(cx, |settings, cx| {
                settings.apply_dock_change(change(*value), cx)
            });
        });
    row_base()
        .child(text_block(title.into(), subtitle))
        .child(toggle)
        .into_any_element()
}

/// The wallpaper's fit, labelled with the wallpaper's name as the Mac labels
/// its variant pop-up ("Tahoe · Automatic").
pub(in crate::controller) fn wallpaper_fit_row(
    view: Entity<Settings>,
    title: SharedString,
    selected: rmac_shell_settings::WallpaperFit,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = WALLPAPER_FIT_OPTIONS
        .iter()
        .copied()
        .map(|(label, fit)| {
            let fit_view = view.clone();
            choice(label, selected == fit, move |_, cx| {
                fit_view.update(cx, |settings, cx| {
                    let target = settings.wallpaper_target.clone();
                    settings.apply_wallpaper_change(target, WallpaperChange::Fit(fit), cx);
                });
            })
        })
        .collect();
    let current = popup_value(&choices, "Fill");
    popup_row("wallpaper-fit", title, None, current, choices, enabled)
}

pub(in crate::controller) fn shortcut_configuration_available(
    status: Option<&rmac_shortcuts::BackendStatus>,
) -> bool {
    matches!(
        status,
        Some(rmac_shortcuts::BackendStatus::Portal {
            version,
            can_configure: true,
        }) if *version >= rmac_shortcuts::PORTAL_CONFIGURE_VERSION
    )
}

/// A "Results from System" row: 20 pt icon, the provider name and a switch.
#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn spotlight_provider_row(
    view: Entity<Settings>,
    id: &'static str,
    icon: &'static str,
    color: Hsla,
    title: &'static str,
    checked: bool,
    enabled: bool,
) -> AnyElement {
    let toggle_view = view.clone();
    icon_row(tile(icon, color, style::ROW_ICON).into_any_element(), title)
        .child(
            Toggle::new(ElementId::from(SharedString::from(format!(
                "spotlight-provider-{id}"
            ))))
            .checked(checked)
            .disabled(!enabled)
            .on_click(move |value, _, cx| {
                toggle_view.update(cx, |settings, cx| {
                    settings.apply_spotlight_change(
                        SpotlightChange::ProviderEnabled {
                            id: id.into(),
                            enabled: *value,
                        },
                        cx,
                    )
                });
            }),
        )
        .into_any_element()
}
