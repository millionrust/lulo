//! The document sheet is attached below the title bar, following the HTML mock
//! in design-lab/text-editor-save-sheet.html.

use super::*;
use gpui::prelude::FluentBuilder as _;
use gpui::{Animation, AnimationExt as _, AnyElement};
use rmac_ui::{DialogButtonKind, PopUpButton};
use std::time::Duration;

fn row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .h(px(38.0))
        .w_full()
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .w(px(140.0))
                .flex_none()
                .text_right()
                .text_size(rmac_ui::text_px(13.0))
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
}

fn encoding_label(encoding: document::TextEncoding) -> &'static str {
    match encoding {
        document::TextEncoding::Utf8 => "Unicode (UTF-8)",
        document::TextEncoding::Utf8Bom => "Unicode (UTF-8) with BOM",
        document::TextEncoding::Utf16Le => "Unicode (UTF-16 Little-Endian)",
        document::TextEncoding::Utf16Be => "Unicode (UTF-16 Big-Endian)",
    }
}

impl EditorView {
    pub(super) fn render_save_goto(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("save-goto-sheet")
            .w(px(460.0))
            .h(px(183.0))
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::sheet())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .occlude()
            .child(
                div()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(mac::BOLD)
                    .child("Go to Folder"),
            )
            .child(
                div()
                    .id("save-goto-path")
                    .role(Role::TextInput)
                    .aria_label("Go to Folder")
                    .accessible_text_input(&self.save_goto_input, cx)
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                cx.stop_propagation();
                                this.close_save_goto(window, cx);
                            }
                            "enter" => {
                                cx.stop_propagation();
                                this.commit_save_goto(window, cx);
                            }
                            _ => {}
                        }
                    }))
                    .child(TextField::new(&self.save_goto_input)),
            )
            .when(self.save_goto_error, |card| {
                card.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child("Choose an existing folder."),
                )
            })
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        rmac_ui::dialog_button(
                            "save-goto-cancel",
                            "Cancel",
                            DialogButtonKind::Normal,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_save_goto(window, cx)),
                        ),
                    )
                    .child(
                        rmac_ui::dialog_button("save-goto-go", "Go", DialogButtonKind::Primary)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.commit_save_goto(window, cx)
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_save_sheet(&self, cx: &mut Context<Self>) -> AnyElement {
        let closing = matches!(self.alert, Some(ActiveAlert::ConfirmSave(Some(_))));
        let where_label = self
            .save_custom_folder
            .as_ref()
            .and_then(|folder| folder.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.save_location.label().to_owned());
        let where_popup = PopUpButton::new("save-sheet-where", where_label)
            .form()
            .dropdown_menu(|menu, _, _| {
                menu.menu("Documents", Box::new(crate::SheetWhereDocuments))
                    .menu("Desktop", Box::new(crate::SheetWhereDesktop))
                    .menu("Home", Box::new(crate::SheetWhereHome))
                    .menu("Downloads", Box::new(crate::SheetWhereDownloads))
                    .separator()
                    .menu("Other…", Box::new(crate::SheetWhereOther))
            });
        let encoding_popup = PopUpButton::new(
            "save-sheet-encoding",
            encoding_label(self.text_format.encoding),
        )
        .form()
        .dropdown_menu(|menu, _, _| {
            menu.menu("Unicode (UTF-8)", Box::new(crate::SheetEncodingUtf8))
                .menu(
                    "Unicode (UTF-8) with BOM",
                    Box::new(crate::SheetEncodingUtf8Bom),
                )
                .menu(
                    "Unicode (UTF-16 Little-Endian)",
                    Box::new(crate::SheetEncodingUtf16Le),
                )
                .menu(
                    "Unicode (UTF-16 Big-Endian)",
                    Box::new(crate::SheetEncodingUtf16Be),
                )
        });
        let card = div()
            .id("save-sheet-card")
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| {
                this.open_save_goto(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveGoToFolder, window, cx| {
                this.open_save_goto(window, cx)
            }))
            .w(px(458.0))
            .h(px(367.0))
            .px(px(26.0))
            .pt(px(30.0))
            .pb(px(20.0))
            .flex()
            .flex_col()
            .rounded(px(mac::radius_card()))
            .bg(mac::sheet())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .relative()
            .occlude()
            .child(
                div()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(mac::BOLD)
                    .child(format!(
                        "Do you want to keep this new document “{}”?",
                        self.filename()
                    )),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .mb(px(22.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_secondary())
                    .child(
                        "You can choose to save your changes or delete this document immediately. You can’t undo this action.",
                    ),
            )
            .child(row(
                "Save As:",
                div()
                    .id("save-sheet-name")
                    .role(Role::TextInput)
                    .aria_label("Save As:")
                    .accessible_text_input(&self.save_name_input, cx)
                    .child(TextField::new(&self.save_name_input)),
            ))
            .child(row("Where:", where_popup))
            .child(row("Plain Text Encoding:", encoding_popup))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(closing, |row| {
                        row.child(
                            rmac_ui::dialog_button(
                                "save-sheet-delete",
                                "Delete",
                                DialogButtonKind::Destructive,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                            ),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        rmac_ui::dialog_button(
                            "save-sheet-cancel",
                            "Cancel",
                            DialogButtonKind::Normal,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx))),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "save-sheet-save",
                            "Save",
                            DialogButtonKind::Primary,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)),
                        ),
                    ),
            )
            .with_animation(
                "text-editor-save-sheet-slide",
                Animation::new(Duration::from_millis(180)),
                |sheet, progress| sheet.top(px(-32.0 * (1.0 - progress))),
            );
        let mut dialog = rmac_ui::dialog("text-editor-save-sheet", card);
        if self.save_goto_open {
            dialog = dialog.passive();
        }
        dialog
            .aria_label(format!(
                "Do you want to keep this new document “{}”?",
                self.filename()
            ))
            .attached()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if this.save_goto_open => {
                        cx.stop_propagation();
                        this.close_save_goto(window, cx);
                    }
                    "enter" if this.save_goto_open => {
                        cx.stop_propagation();
                        this.commit_save_goto(window, cx);
                    }
                    "escape" if matches!(this.alert, Some(ActiveAlert::ConfirmSave(_))) => {
                        cx.stop_propagation();
                        this.alert_cancel(cx);
                    }
                    "enter" if matches!(this.alert, Some(ActiveAlert::ConfirmSave(_))) => {
                        cx.stop_propagation();
                        this.alert_confirm(window, cx);
                    }
                    _ => {}
                }
            }))
            .into_any_element()
    }
}
