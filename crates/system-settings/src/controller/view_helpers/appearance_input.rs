//! System Settings theme, GTK text-scale, and input control-row projection.

use super::*;

pub(in crate::controller) type GtkTextScaleOption = (&'static str, f64);

pub(in crate::controller) const GTK_TEXT_SCALE_OPTIONS: [GtkTextScaleOption; 3] =
    [("Standard", 1.0), ("Large", 1.2), ("Extra Large", 1.3)];

pub(in crate::controller) fn theme_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [ThemeOption],
    selected: usize,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = options
        .iter()
        .copied()
        .enumerate()
        .map(|(index, (option_label, change))| {
            let option_view = view.clone();
            choice(option_label, selected == index, move |_, cx| {
                option_view.update(cx, |settings, cx| settings.apply_theme_change(change, cx));
            })
        })
        .collect();
    let current = popup_value(&choices, "Automatic");
    popup_row(id, title, None, current, choices, enabled)
}

pub(in crate::controller) fn gtk_text_scale_row(
    view: Entity<Settings>,
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = GTK_TEXT_SCALE_OPTIONS
        .iter()
        .copied()
        .enumerate()
        .map(|(index, (option_label, factor))| {
            let option_view = view.clone();
            choice(option_label, selected == Some(index), move |_, cx| {
                option_view.update(cx, |settings, cx| settings.set_gtk_text_scale(factor, cx));
            })
        })
        .collect();
    let current = popup_value(&choices, "Custom");
    popup_row(
        "gtk-text-scale",
        "GTK application text",
        None,
        current,
        choices,
        enabled,
    )
}

pub(in crate::controller) fn input_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [InputOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let choices: Vec<PopupChoice> = options
        .iter()
        .copied()
        .enumerate()
        .map(|(index, (option_label, change))| {
            let option_view = view.clone();
            choice(option_label, selected == Some(index), move |_, cx| {
                option_view.update(cx, |settings, cx| settings.apply_input_change(change, cx));
            })
        })
        .collect();
    let current = popup_value(&choices, "Custom");
    popup_row(id, title, None, current, choices, enabled)
}
