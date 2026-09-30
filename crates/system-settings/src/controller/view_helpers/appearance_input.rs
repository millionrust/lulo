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

/// Appearance ▸ Text highlight colour: "Automatic" (follows the accent),
/// then the same 8 named swatches Theme ▸ Colour offers.
pub(in crate::controller) fn text_highlight_row(
    view: Entity<Settings>,
    current: rmac_theme::TextHighlightPreference,
    enabled: bool,
) -> AnyElement {
    let mut choices: Vec<PopupChoice> = Vec::with_capacity(ACCENTS.len() + 1);
    let automatic_view = view.clone();
    choices.push(choice(
        "Automatic",
        current == rmac_theme::TextHighlightPreference::Automatic,
        move |_, cx| {
            automatic_view.update(cx, |settings, cx| {
                settings.apply_theme_change(
                    ThemeChange::TextHighlight(rmac_theme::TextHighlightPreference::Automatic),
                    cx,
                )
            });
        },
    ));
    for (name, hex) in ACCENTS.iter().copied() {
        let preference = text_highlight_preference(hex);
        let option_view = view.clone();
        choices.push(choice(name, current == preference, move |_, cx| {
            option_view.update(cx, |settings, cx| {
                settings.apply_theme_change(ThemeChange::TextHighlight(preference), cx)
            });
        }));
    }
    let label = popup_value(&choices, "Automatic");
    popup_row(
        "theme-text-highlight",
        "Text highlight colour",
        None,
        label,
        choices,
        enabled,
    )
}

/// Appearance ▸ Show scroll bars: a Mac radio matrix, not a pop-up, since
/// the Mac itself lists three plain radio buttons rather than a menu.
pub(in crate::controller) fn scroll_bar_visibility_row(
    view: Entity<Settings>,
    selected: rmac_theme::ScrollBarVisibility,
    enabled: bool,
) -> AnyElement {
    use rmac_theme::ScrollBarVisibility;
    let options = [
        (
            "Automatically based on mouse or trackpad",
            ScrollBarVisibility::Automatic,
        ),
        ("When scrolling", ScrollBarVisibility::WhenScrolling),
        ("Always", ScrollBarVisibility::Always),
    ];
    let index = options
        .iter()
        .position(|(_, value)| *value == selected)
        .unwrap_or(0);
    let radio_view = view.clone();
    row_base()
        .v_flex()
        .items_start()
        .gap(px(8.0))
        .child(text_block("Show scroll bars".into(), None))
        .child(
            RadioGroup::new(
                "scroll-bar-visibility",
                options.iter().map(|(label, _)| *label),
            )
            .label("Show scroll bars")
            .selected(index)
            .disabled(!enabled)
            .on_change(move |index, _, cx| {
                if let Some((_, value)) = options.get(index).copied() {
                    radio_view.update(cx, |settings, cx| {
                        settings.apply_theme_change(ThemeChange::ScrollBarVisibility(value), cx)
                    });
                }
            }),
        )
        .into_any_element()
}

/// Appearance ▸ Click in the scroll bar to: a two-option radio matrix.
pub(in crate::controller) fn scroll_bar_click_row(
    view: Entity<Settings>,
    selected: rmac_theme::ScrollBarClickAction,
    enabled: bool,
) -> AnyElement {
    use rmac_theme::ScrollBarClickAction;
    let options = [
        (
            "Jump to the next page",
            ScrollBarClickAction::JumpToNextPage,
        ),
        (
            "Jump to the spot that's clicked",
            ScrollBarClickAction::JumpToSpot,
        ),
    ];
    let index = options
        .iter()
        .position(|(_, value)| *value == selected)
        .unwrap_or(0);
    let radio_view = view.clone();
    row_base()
        .v_flex()
        .items_start()
        .gap(px(8.0))
        .child(text_block("Click in the scroll bar to".into(), None))
        .child(
            RadioGroup::new("scroll-bar-click", options.iter().map(|(label, _)| *label))
                .label("Click in the scroll bar to")
                .selected(index)
                .disabled(!enabled)
                .on_change(move |index, _, cx| {
                    if let Some((_, value)) = options.get(index).copied() {
                        radio_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(ThemeChange::ScrollBarClick(value), cx)
                        });
                    }
                }),
        )
        .into_any_element()
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
