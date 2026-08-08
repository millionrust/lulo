//! Text Editor toolbar and document-status chrome projection.

use super::*;

impl EditorView {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.filename();
        let dirty = self.dirty;
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .px_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("new", "")
                            .icon(Icon::new(IconName::File).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(self.file_busy || self.file_action_blocked())
                            .tooltip("New Window")
                            .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
                    )
                    .child(
                        Button::new("open", "")
                            .icon(Icon::new(IconName::FolderOpen).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(self.file_busy || self.file_action_blocked())
                            .tooltip("Open")
                            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
                    )
                    .child(
                        Button::new("find", "")
                            .icon(Icon::new(IconName::Search).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .selected(self.find_open)
                            .tooltip("Find")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_find(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(mac::MEDIUM)
                    .text_color(mac::text())
                    .child(div().min_w_0().truncate().child(title))
                    .when(dirty, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(mac::text_secondary())
                                .child("— Edited"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("mono", "Mono")
                            .ghost()
                            .with_size(Size::Small)
                            .selected(self.mono)
                            .tooltip("Monospace font")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mono(cx))),
                    )
                    .child(
                        Button::new("font-dec", "")
                            .icon(Icon::new(IconName::Minus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Smaller text")
                            .on_click(cx.listener(|this, _, _, cx| this.decrease_font(cx))),
                    )
                    .child(
                        Button::new("font-inc", "")
                            .icon(Icon::new(IconName::Plus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Larger text")
                            .on_click(cx.listener(|this, _, _, cx| this.increase_font(cx))),
                    )
                    .when(cfg!(target_os = "linux"), |actions| {
                        actions.child(
                            Button::new("print", "Print…")
                                .ghost()
                                .with_size(Size::Small)
                                .busy(self.print_busy)
                                .disabled(!can_begin_print(
                                    self.file_busy,
                                    self.print_busy,
                                    self.recovery_loading,
                                    self.alert.is_some(),
                                    self.rtf_runs.is_some(),
                                ))
                                .tooltip("Print Document")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.print_document(window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("save", "Save")
                            .primary()
                            .with_size(Size::Small)
                            .busy(self.file_busy)
                            .disabled(
                                self.file_busy
                                    || self.rtf_runs.is_some()
                                    || self.file_action_blocked(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            );
        rmac_ui::toolbar(row)
    }

    pub(super) fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let st = self.input.read(cx);
        let pos = st.cursor_position();
        let value = st.value();
        let chars = value.chars().count();
        let words = value.split_whitespace().count();

        let cell = |s: String| {
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(mac::text_secondary())
                .child(s)
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(24.0))
            .px_3()
            .border_t_1()
            .border_color(mac::separator())
            .bg(mac::chrome())
            .child(cell(format!(
                "Ln {}, Col {}",
                pos.line + 1,
                pos.character + 1
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new(
                            "document-format",
                            self.text_format.status_against(self.saved_format),
                        )
                        .ghost()
                        .xsmall()
                        .disabled(
                            self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked(),
                        )
                        .tooltip("Text encoding and line endings")
                        .dropdown_menu({
                            let format = self.text_format;
                            move |menu, _, _| {
                                menu.menu_with_check(
                                    "UTF-8",
                                    format.encoding == document::TextEncoding::Utf8,
                                    Box::new(SetEncodingUtf8),
                                )
                                .menu_with_check(
                                    "UTF-8 with BOM",
                                    format.encoding == document::TextEncoding::Utf8Bom,
                                    Box::new(SetEncodingUtf8Bom),
                                )
                                .menu_with_check(
                                    "UTF-16 Little Endian",
                                    format.encoding == document::TextEncoding::Utf16Le,
                                    Box::new(SetEncodingUtf16Le),
                                )
                                .menu_with_check(
                                    "UTF-16 Big Endian",
                                    format.encoding == document::TextEncoding::Utf16Be,
                                    Box::new(SetEncodingUtf16Be),
                                )
                                .separator()
                                .menu_with_check(
                                    "Unix (LF)",
                                    format.save_line_ending == document::LineEnding::Lf,
                                    Box::new(SetLineEndingLf),
                                )
                                .menu_with_check(
                                    "Windows (CRLF)",
                                    format.save_line_ending == document::LineEnding::CrLf,
                                    Box::new(SetLineEndingCrLf),
                                )
                                .menu_with_check(
                                    "Classic Mac (CR)",
                                    format.save_line_ending == document::LineEnding::Cr,
                                    Box::new(SetLineEndingCr),
                                )
                            }
                        }),
                    )
                    .child(cell(format!(
                        "{} {}",
                        words,
                        if words == 1 { "word" } else { "words" }
                    )))
                    .child(cell(format!(
                        "{} {}",
                        chars,
                        if chars == 1 { "char" } else { "chars" }
                    ))),
            )
    }
}
