//! Shared display choice-row projection.

use super::*;

pub(super) fn display_choice_row(
    id: SharedString,
    title: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    disabled: bool,
) -> ListRow {
    let has_subtitle = subtitle.is_some();
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    let mut text = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(foreground)
            .child(title),
    );
    if let Some(subtitle) = subtitle {
        text = text.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary_foreground)
                .child(subtitle),
        );
    }
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .child(text)
        .when(selected, |row| {
            row.child(glyph("icons/check.svg", 14.0, on_accent()))
        });
    ListRow::new(ElementId::from(id), content)
        .selected(selected)
        .disabled(disabled)
        .h(px(if has_subtitle { 60.0 } else { 44.0 }))
        .px_3()
}
