mod alert;
mod chrome;
mod find;
mod rtf;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, font, px, AccessibleAction, Context, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement as _, Styled, StyledText,
    TextRun, UnderlineStyle, Window,
};
use gpui_component::{Icon, IconName, Size, StyledExt as _};
use rmac_ui::{mac, Button, SearchField, TextField};

use crate::{
    document, CloseBar, CloseWindow, DecreaseFont, DuplicateDocument, ExportPdf, FindNext,
    FindPrev, IncreaseFont, NewFile, OpenFile, PrintFile, SaveFile, SaveFileAs, SetEncodingUtf16Be,
    SetEncodingUtf16Le, SetEncodingUtf8, SetEncodingUtf8Bom, SetLineEndingCr, SetLineEndingCrLf,
    SetLineEndingLf, ToggleFind, ToggleMono, ToggleReplace,
};

use super::{
    responsive_layout::EditorLayout, ActiveAlert, AssistiveEdit, EditorView, ExternalChange,
    Pending, CTX,
};

/// Text origin from the window's left edge (the Mac's caret sits at x 9).
const TEXT_INSET_X: f32 = 10.0;
/// Menlo 11 sets 13 pt lines in TextEdit.
const PLAIN_LINE_RATIO: f32 = 13.0 / 11.0;

/// NSTextView's text background: #1E1E1E in dark mode (measured), white in
/// light, untinted by the wallpaper unlike the title bar.
pub(super) fn text_background() -> gpui::Hsla {
    if mac::window().l < 0.5 {
        gpui::rgb(0x1e1e1e).into()
    } else {
        gpui::rgb(0xffffff).into()
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Text Editor's document is loaded before its window is even created
        // (see open_editor_window), so its first frame already is the real
        // content the performance harness should time launch-to-interactive
        // against. Safe to call every render; only the first call writes the
        // benchmark marker.
        rmac_ui::mark_content_ready(window);
        let layout = super::responsive_layout::editor_layout(f32::from(window.bounds().size.width));
        let filename = self.filename();
        let subject = if self.dirty {
            format!("{filename} — Edited")
        } else {
            filename.to_string()
        };
        let native_window_title = rmac_ui::native_window_title(&subject, "Text Editor");
        if self.native_window_title != native_window_title {
            window.set_window_title(&native_window_title);
            self.native_window_title = native_window_title;
        }
        if window.is_window_active() {
            self.publish_menu_state(window, cx);
        }
        let font_family = if self.mono {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        };
        let size = self.font_size;
        let line_height = (size * PLAIN_LINE_RATIO).round();
        let recovery_loading = self.recovery_loading;
        let recovery_error = self.recovery_error.clone();
        let status_notice = self.status_notice.clone();
        let external_change = self.external_change;
        let document_watch_warning = self.document_watch_warning;
        let accessible_value = self.accessible_document_value(cx);
        let long_line_body = if self.long_lines.is_some() && self.rtf_runs.is_none() {
            Some(
                self.render_long_line_view(
                    font_family,
                    size,
                    line_height,
                    TEXT_INSET_X,
                    window,
                    cx,
                )
                .into_any_element(),
            )
        } else {
            None
        };

        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context(CTX)
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &DuplicateDocument, window, cx| {
                this.duplicate_document(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ExportPdf, window, cx| this.export_pdf(window, cx)),
            )
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
            .child(self.render_title_bar(cx))
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
            .when(self.find_open, |d| d.child(self.render_find_bar(layout, cx)))
            .child(if self.rtf_runs.is_some() {
                self.render_rtf_preview(layout, cx).into_any_element()
            } else if let Some(body) = long_line_body {
                div()
                    .id("document-body")
                    .role(Role::Document)
                    .aria_label(filename.clone())
                    .when_some(accessible_value.clone(), |body, value| body.aria_value(value))
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .bg(text_background())
                    .child(body)
                    .into_any_element()
            } else {
                // TextEdit's plain-text view: #1E1E1E edge to edge, the text
                // origin 10 from the left and flush with the top, Menlo 11 on
                // a 13 pt pitch (JetBrains Mono stands in for Menlo). The
                // wrapper names the document for assistive technologies and
                // accepts their SetValue / ReplaceSelectedText edits.
                let editable = !(recovery_loading || self.print_busy);
                div()
                    .id("document-body")
                    .role(Role::MultilineTextInput)
                    .aria_label(filename.clone())
                    .when_some(accessible_value, |body, value| body.aria_value(value))
                    .flex_1()
                    .min_h(px(0.0))
                    .bg(text_background())
                    .child(
                        TextField::new(&self.input)
                            .large()
                            .h_full()
                            .appearance(false)
                            .disabled(!editable)
                            .font_family(font_family)
                            .text_size(px(size))
                            .line_height(px(line_height))
                            .pl(px(TEXT_INSET_X))
                            .pr(px(TEXT_INSET_X))
                            .pt(px(0.0))
                            .pb(px(0.0)),
                    )
                    .when(editable, |body| {
                        body.on_a11y_action(
                            AccessibleAction::SetValue,
                            self.assistive_edit_listener(AssistiveEdit::SetValue, cx),
                        )
                        .on_a11y_action(
                            AccessibleAction::ReplaceSelectedText,
                            self.assistive_edit_listener(AssistiveEdit::ReplaceSelection, cx),
                        )
                    })
                    .into_any_element()
            })
            .when_some(self.alert.clone(), |d, alert| {
                d.child(self.render_alert(alert, cx))
            })
    }
}
