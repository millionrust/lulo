//! Text Editor Find and Replace bar projection.

use super::*;

impl EditorView {
    pub(super) fn render_find_bar(
        &self,
        layout: EditorLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query_empty = self.find_input.read(cx).value().is_empty();
        let status: SharedString = if query_empty {
            "".into()
        } else if self.matches.is_empty() {
            "Not found".into()
        } else {
            format!("{} of {}", self.current + 1, self.matches.len()).into()
        };

        let find_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(layout.find_input_width))
                    .child(SearchField::new(&self.find_input).appearance(true)),
            )
            .child(
                Button::new("find-prev", "")
                    .icon(Icon::new(IconName::ChevronUp).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Previous match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_prev(window, cx))),
            )
            .child(
                Button::new("find-next", "")
                    .icon(Icon::new(IconName::ChevronDown).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Next match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_next(window, cx))),
            )
            .child(
                div()
                    .min_w(px(64.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(status),
            )
            .child(div().flex_1())
            .child(
                Button::new("find-close", "")
                    .icon(Icon::new(IconName::Close).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Done")
                    .on_click(cx.listener(|this, _, _, cx| this.close_bar(cx))),
            );

        let mut col = div()
            .v_flex()
            .gap_2()
            .w_full()
            .px(px(layout.content_padding))
            .py(px(8.0))
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            .child(find_row);

        if self.replace_mode {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(layout.find_input_width))
                            .child(TextField::new(&self.replace_input).appearance(true)),
                    )
                    .child(
                        Button::new("replace-one", "Replace")
                            .ghost()
                            .with_size(Size::Small)
                            .disabled(self.print_busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_current(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("replace-all", "Replace All")
                            .ghost()
                            .with_size(Size::Small)
                            .disabled(self.print_busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_all(window, cx)),
                            ),
                    ),
            );
        }

        col
    }
}
