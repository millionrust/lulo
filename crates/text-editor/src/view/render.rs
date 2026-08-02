use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, font, px, Context, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, StyledText, TextRun, UnderlineStyle,
    Window,
};
use gpui_component::{Icon, IconName, Size, StyledExt as _};
use rmac_ui::{mac, Button, SearchField, TextField};

use crate::{
    document, CloseBar, CloseWindow, DecreaseFont, FindNext, FindPrev, IncreaseFont, NewFile,
    OpenFile, PrintFile, SaveFile, SaveFileAs, SetEncodingUtf16Be, SetEncodingUtf16Le,
    SetEncodingUtf8, SetEncodingUtf8Bom, SetLineEndingCr, SetLineEndingCrLf, SetLineEndingLf,
    ToggleFind, ToggleMono, ToggleReplace,
};

use super::{can_begin_print, ActiveAlert, EditorView, ExternalChange, Pending, CTX};

impl EditorView {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(mac::MEDIUM)
                    .text_color(mac::text())
                    .child(title)
                    .when(dirty, |d| {
                        d.child(
                            div()
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

    fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .w(px(220.0))
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
            .px(px(48.0))
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
                            .w(px(220.0))
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

    /// The read-only formatted RTF preview: a banner plus styled text built from
    /// the parsed runs (weight / italic / underline / color preserved).
    fn render_rtf_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = if self.mono {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        };
        let size = self.font_size;
        let runs = self.rtf_runs.as_deref().unwrap_or(&[]);

        let mut text = String::new();
        let mut text_runs: Vec<TextRun> = Vec::new();
        for r in runs {
            if r.text.is_empty() {
                continue;
            }
            let family = r.family.clone().unwrap_or_else(|| base.to_string());
            let mut f = font(family);
            if r.bold {
                f = f.bold();
            }
            if r.italic {
                f = f.italic();
            }
            let color = r
                .color
                .map(|(rr, gg, bb)| {
                    gpui::rgb(((rr as u32) << 16) | ((gg as u32) << 8) | bb as u32).into()
                })
                .unwrap_or_else(mac::text);
            text_runs.push(TextRun {
                len: r.text.len(),
                font: f,
                color,
                background_color: None,
                underline: r.underline.then(|| UnderlineStyle {
                    thickness: px(1.0),
                    color: None,
                    wavy: false,
                }),
                strikethrough: None,
            });
            text.push_str(&r.text);
        }

        let banner = div()
            .flex_none()
            .h_flex()
            .items_center()
            .justify_between()
            .mb_4()
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .bg(mac::chrome())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child("Read-only RTF preview — formatting shown as in the document."),
            )
            .child(
                Button::new("edit-plain", "Edit as Plain Text")
                    .small()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.edit_as_plain_text(window, cx)),
                    ),
            );

        div()
            .id("rtf-preview")
            .flex_1()
            .overflow_y_scroll()
            .px(px(48.0))
            .py(px(20.0))
            .text_size(px(size))
            .line_height(px(size * 1.5))
            .child(banner)
            .child(StyledText::new(text).with_runs(text_runs))
    }

    /// Bottom status bar: live cursor line:column (1-based) on the left, and the
    /// document's word + character counts on the right — like a real editor.
    fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
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

    /// Build the shared modal alert for the current `ActiveAlert`.
    fn render_alert(&self, alert: ActiveAlert, cx: &mut Context<Self>) -> impl IntoElement {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
        let (title, message, buttons): (&str, String, Vec<gpui::AnyElement>) = match alert {
            ActiveAlert::Recover(prompt) => (
                "Recover unsaved changes?",
                format!(
                    "An autosaved draft for “{}” was found.{}",
                    prompt.document_label,
                    if prompt.additional_drafts == 0 {
                        String::new()
                    } else {
                        format!(
                            " {} additional {} will remain available for a later launch.",
                            prompt.additional_drafts,
                            if prompt.additional_drafts == 1 {
                                "draft"
                            } else {
                                "drafts"
                            }
                        )
                    }
                ),
                vec![
                    rmac_ui::dialog_button("alert-discard", "Discard", Normal)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-restore", "Restore", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmSave(_) => (
                "Do you want to save the changes you made?",
                "Your changes will be lost if you don't save them.".into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-dontsave", "Don't Save", Destructive)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save", "Save", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Conflict => (
                "The document changed in another application.",
                "Text Editor did not overwrite the external version. Reload discards this local buffer, Save a Copy preserves it at a new location, and Overwrite requires a fresh review plus another exact preflight."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-reload", "Discard & Reload", Destructive)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.reload_conflicting_document(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save-copy", "Save a Copy…", Primary)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_conflicting_copy(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-review-overwrite",
                        "Overwrite Anyway…",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.review_conflict_overwrite(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmOverwrite { .. } => (
                "Overwrite the external document?",
                "Text Editor reread the complete external revision. Overwrite will run a second exact preflight and stop if the document changes again. This cannot preserve the external edits."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-overwrite", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-confirm-overwrite",
                        "Overwrite Anyway",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.alert_confirm(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::Error { title, message } => (
                title,
                message,
                vec![rmac_ui::dialog_button("alert-ok", "OK", Primary)
                    .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                    .into_any_element()],
            ),
        };
        rmac_ui::alert(title, message, buttons)
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font_family = if self.mono {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        };
        let size = self.font_size;
        let recovery_loading = self.recovery_loading;
        let recovery_error = self.recovery_error.clone();
        let status_notice = self.status_notice.clone();
        let external_change = self.external_change;
        let document_watch_warning = self.document_watch_warning;

        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context(CTX)
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| this.save_as(window, cx)))
            .on_action(
                cx.listener(|this, _: &PrintFile, window, cx| this.print_document(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFind, window, cx| this.toggle_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleReplace, window, cx| this.toggle_replace(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_next(window, cx)))
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| this.find_prev(window, cx)))
            .on_action(cx.listener(|this, _: &CloseBar, _, cx| this.close_bar(cx)))
            .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
            .on_action(cx.listener(|this, _: &SetEncodingUtf8, _, cx| {
                this.set_encoding(document::TextEncoding::Utf8, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf8Bom, _, cx| {
                this.set_encoding(document::TextEncoding::Utf8Bom, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf16Le, _, cx| {
                this.set_encoding(document::TextEncoding::Utf16Le, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf16Be, _, cx| {
                this.set_encoding(document::TextEncoding::Utf16Be, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingLf, _, cx| {
                this.set_line_ending(document::LineEnding::Lf, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingCrLf, _, cx| {
                this.set_line_ending(document::LineEnding::CrLf, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingCr, _, cx| {
                this.set_line_ending(document::LineEnding::Cr, cx)
            }))
            .on_action(cx.listener(|this, _: &IncreaseFont, _, cx| this.increase_font(cx)))
            .on_action(cx.listener(|this, _: &DecreaseFont, _, cx| this.decrease_font(cx)))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                this.guarded(Pending::Close, window, cx)
            }))
            // The custom red traffic light dispatches RequestClose — route it
            // through the same unsaved-changes guard so closes aren't silent.
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.guarded(Pending::Close, window, cx)
            }))
            .bg(mac::window())
            .text_color(mac::text())
            .child(self.render_toolbar(cx))
            .when(recovery_loading, |editor| {
                editor.child(
                    div()
                        .id("recovery-loading")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child("Checking for unsaved drafts…"),
                )
            })
            .when_some(recovery_error, |editor, message| {
                editor.child(
                    div()
                        .id("recovery-error")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::error_background())
                        .border_b_1()
                        .border_color(mac::error_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::danger())
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("dismiss-recovery-error", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.recovery_error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(status_notice, |editor, message| {
                editor.child(
                    div()
                        .id("recovery-notice")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("dismiss-status-notice", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.status_notice = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(external_change, |editor, change| {
                let message = match change {
                    ExternalChange::Modified => {
                        "This document changed outside Text Editor. Your buffer was not replaced."
                    }
                    ExternalChange::Missing => {
                        "This document was moved or deleted outside Text Editor. Your buffer remains open."
                    }
                    ExternalChange::Unreadable => {
                        "Text Editor can no longer verify the external document. Your buffer remains open."
                    }
                };
                editor.child(
                    div()
                        .id("external-change")
                        .h(px(40.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::warning_background())
                        .border_b_1()
                        .border_color(mac::warning_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text())
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("external-change-review", "Review…")
                                .with_size(Size::Small)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_external_conflict(cx);
                                })),
                        ),
                )
            })
            .when(document_watch_warning && external_change.is_none(), |editor| {
                editor.child(
                    div()
                        .id("document-watch-warning")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Live document monitoring is unavailable. Saves still recheck the complete file before writing.",
                        ),
                )
            })
            .when(self.find_open, |d| d.child(self.render_find_bar(cx)))
            .child(if self.rtf_runs.is_some() {
                self.render_rtf_preview(cx).into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(48.0))
                    .py(px(20.0))
                    .font_family(font_family)
                    .text_size(px(size))
                    .line_height(px(size * 1.5))
                    .child(
                        TextField::new(&self.input)
                            .h_full()
                            .appearance(false)
                            .disabled(recovery_loading || self.print_busy),
                    )
                    .into_any_element()
            })
            .when(self.rtf_runs.is_none(), |d| {
                d.child(self.render_status_bar(cx))
            })
            .when_some(self.alert.clone(), |d, alert| {
                d.child(self.render_alert(alert, cx))
            })
    }
}
