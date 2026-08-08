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
    let labels = options.iter().map(|(label, _)| *label).collect::<Vec<_>>();
    let control = Tabs::new(id, labels)
        .selected(selected)
        .disabled(!enabled)
        .on_change(move |index, _, cx| {
            if let Some((_, change)) = options.get(*index).copied() {
                view.update(cx, |settings, cx| settings.apply_theme_change(change, cx));
            }
        })
        .w(px(290.0));
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

pub(in crate::controller) fn gtk_text_scale_row(
    view: Entity<Settings>,
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, factor)) in GTK_TEXT_SCALE_OPTIONS.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("gtk-text-scale-{index}"))),
                option_label,
            )
            .flex_1()
            .h(px(26.0))
            .selected(selected == Some(index))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                option_view.update(cx, |settings, cx| settings.set_gtk_text_scale(factor, cx));
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child("GTK application text"),
        )
        .child(control)
        .into_any_element()
}

pub(in crate::controller) fn input_segment_row(
    view: Entity<Settings>,
    id: &'static str,
    title: &'static str,
    options: &'static [InputOption],
    selected: Option<usize>,
    enabled: bool,
) -> AnyElement {
    let mut control = div().flex().gap_1().w(px(290.0));
    for (index, (option_label, change)) in options.iter().copied().enumerate() {
        let option_view = view.clone();
        control = control.child(
            Button::new(
                ElementId::from(SharedString::from(format!("{id}-{index}"))),
                option_label,
            )
            .flex_1()
            .h(px(26.0))
            .selected(selected == Some(index))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                option_view.update(cx, |settings, cx| settings.apply_input_change(change, cx));
            }),
        );
    }
    row_base()
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(label())
                .child(title),
        )
        .child(control)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn input_switch_row(
    view: Entity<Settings>,
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> InputChange,
) -> AnyElement {
    let switch = Toggle::new(id)
        .checked(checked)
        .disabled(!enabled)
        .on_click(move |value, _, cx| {
            view.update(cx, |settings, cx| {
                settings.apply_input_change(change(*value), cx)
            });
        });
    row_base()
        .child(tile(icon, secondary(), 22.0))
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(switch)
        .into_any_element()
}
