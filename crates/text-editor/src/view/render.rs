mod alert;
mod chrome;
mod document_dialogs;
mod find;
mod rtf;
mod save_sheet;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, font, px, AccessibleAction, ClickEvent, Context, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled, StyledText, TextRun, UnderlineStyle, Window,
};
use gpui_component::{Icon, IconName, Size, StyledExt as _};
use rmac_ui::{mac, AccessibleTextInput as _, Button, SearchField, TextField};

use crate::{
    document, ActualSize, AlignCentre, AlignLeft, AlignRight, CheckDocumentNow, ClearRecentMenu,
    CloseAll, CloseBar, CloseWindow, CopyRuler, DecreaseFont, DuplicateDocument, EnterFullScreen,
    ExportPdf, FindNext, FindPrev, IncreaseFont, InsertLineBreak, InsertPageBreak,
    InsertParagraphBreak, JumpToSelection, MoveToFolder, NewFile, OpenFile, OpenPageSetup,
    OpenRecent0, OpenRecent1, OpenRecent2, OpenRecent3, OpenRecent4, OpenRecent5, OpenRecent6,
    OpenRecent7, OpenRecent8, OpenRecent9, OpenSpacing, PasteRuler, PreventEditing, PrintFile,
    QuitAndKeepWindows, RenameDocument, RevertToLastSaved, SaveFile, SaveFileAs, SaveGoToFolder,
    SelectLine, SetEncodingUtf16Be, SetEncodingUtf16Le, SetEncodingUtf8, SetEncodingUtf8Bom,
    SetLineEndingCr, SetLineEndingCrLf, SetLineEndingLf, ShowRuler, ShowSettings,
    ShowSpellingAndGrammar, ShowSubstitutions, StartSpeaking, StopSpeaking,
    ToggleCheckGrammarWithSpelling, ToggleCheckSpellingWhileTyping,
    ToggleCorrectSpellingAutomatically, ToggleDarkBackground, ToggleDataDetectors, ToggleFind,
    ToggleMono, ToggleReplace, ToggleRichText, ToggleSmartCopyPaste, ToggleSmartDashes,
    ToggleSmartLinks, ToggleSmartQuotes, ToggleTextReplacement, ToggleWrapToPage,
    TransformCapitalise, TransformLowercase, TransformUppercase, UseSelectionForFind, ZoomIn,
    ZoomOut,
};

use super::{
    responsive_layout::EditorLayout, ActiveAlert, AssistiveEdit, EditorView, ExternalChange,
    Pending, SaveLocation, CTX,
};

/// Text origin from the window's left edge (the Mac's caret sits at x 9).
const TEXT_INSET_X: f32 = 10.0;
/// Menlo 11 sets 13 pt lines in TextEdit.
const PLAIN_LINE_RATIO: f32 = 13.0 / 11.0;

/// NSTextView's dark-mode paper colour (measured): #1E1E1E.
fn dark_paper() -> gpui::Hsla {
    gpui::rgb(0x1e1e1e).into()
}

/// NSTextView's text background: dark paper in dark mode (measured), white
/// in light, untinted by the wallpaper unlike the title bar.
pub(super) fn text_background() -> gpui::Hsla {
    if mac::window().l < 0.5 {
        dark_paper()
    } else {
        gpui::rgb(0xffffff).into()
    }
}

impl EditorView {
    /// View ▸ Use Dark Background for Windows (TXT-MENU-082): this
    /// window's own paper colour, overriding the system appearance with
    /// the same dark paper `text_background()` already falls back to.
    pub(super) fn text_background(&self) -> gpui::Hsla {
        if self.dark_background {
            dark_paper()
        } else {
            text_background()
        }
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
        let page_width = f32::from(self.page_width_chars) * size * 0.596 + TEXT_INSET_X * 2.0;
        let wrap_to_page = self.wrap_to_page;
        let ruler = self.ruler;
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
            .on_action(cx.listener(|_, _: &ShowSettings, _, cx| {
                crate::settings_window::show(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenRecent0, window, cx| this.open_recent(0, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent1, window, cx| this.open_recent(1, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent2, window, cx| this.open_recent(2, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent3, window, cx| this.open_recent(3, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent4, window, cx| this.open_recent(4, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent5, window, cx| this.open_recent(5, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent6, window, cx| this.open_recent(6, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent7, window, cx| this.open_recent(7, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent8, window, cx| this.open_recent(8, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent9, window, cx| this.open_recent(9, window, cx)))
            .on_action(cx.listener(|this, _: &ClearRecentMenu, _, cx| this.clear_recent_documents(cx)))
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
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_next(cx)))
            .on_action(cx.listener(|this, _: &UseSelectionForFind, window, cx| {
                this.use_selection_for_find(window, cx)
            }))
            .on_action(cx.listener(|this, _: &JumpToSelection, window, cx| {
                this.jump_to_selection(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectLine, window, cx| {
                this.select_line(window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertLineBreak, window, cx| {
                this.insert_break("\u{2028}", window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertParagraphBreak, window, cx| {
                this.insert_break("\n", window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertPageBreak, window, cx| {
                this.insert_break("\u{000c}", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformUppercase, window, cx| {
                this.transform_selection(rmac_ui::TextTransformation::Uppercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformLowercase, window, cx| {
                this.transform_selection(rmac_ui::TextTransformation::Lowercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformCapitalise, window, cx| {
                this.transform_selection(rmac_ui::TextTransformation::Capitalise, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowSpellingAndGrammar, window, cx| {
                this.check_document_now(window, cx)
            }))
            .on_action(cx.listener(|this, _: &CheckDocumentNow, window, cx| {
                this.check_document_now(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleCheckSpellingWhileTyping, _, cx| {
                this.text_assist.check_spelling_while_typing =
                    !this.text_assist.check_spelling_while_typing;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleCheckSpellingWhileTyping",
                    this.text_assist.check_spelling_while_typing,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleCheckGrammarWithSpelling, _, cx| {
                this.text_assist.check_grammar_with_spelling =
                    !this.text_assist.check_grammar_with_spelling;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleCheckGrammarWithSpelling",
                    this.text_assist.check_grammar_with_spelling,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleCorrectSpellingAutomatically, _, cx| {
                this.text_assist.correct_spelling_automatically =
                    !this.text_assist.correct_spelling_automatically;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleCorrectSpellingAutomatically",
                    this.text_assist.correct_spelling_automatically,
                    cx,
                );
            }))
            .on_action(cx.listener(|_, _: &ShowSubstitutions, _, cx| {
                // No dedicated Substitutions panel exists yet; Settings…
                // is the nearest real destination (see docs/parity.md TE-07).
                crate::settings_window::show(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSmartCopyPaste, _, cx| {
                this.text_assist.smart_copy_paste = !this.text_assist.smart_copy_paste;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleSmartCopyPaste",
                    this.text_assist.smart_copy_paste,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleSmartQuotes, _, cx| {
                this.text_assist.smart_quotes = !this.text_assist.smart_quotes;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleSmartQuotes",
                    this.text_assist.smart_quotes,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleSmartDashes, _, cx| {
                this.text_assist.smart_dashes = !this.text_assist.smart_dashes;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleSmartDashes",
                    this.text_assist.smart_dashes,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleSmartLinks, _, cx| {
                this.text_assist.smart_links = !this.text_assist.smart_links;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleSmartLinks",
                    this.text_assist.smart_links,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleDataDetectors, _, cx| {
                this.data_detectors = !this.data_detectors;
                rmac_ui::set_menu_checked("text_editor::ToggleDataDetectors", this.data_detectors, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTextReplacement, _, cx| {
                this.text_assist.text_replacement = !this.text_assist.text_replacement;
                rmac_ui::set_menu_checked(
                    "text_editor::ToggleTextReplacement",
                    this.text_assist.text_replacement,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &StartSpeaking, _, cx| this.start_speaking(cx)))
            .on_action(cx.listener(|_, _: &StopSpeaking, _, _| rmac_ui::stop_speaking()))
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| {
                if matches!(this.alert, Some(ActiveAlert::ConfirmSave(_))) {
                    this.open_save_goto(window, cx);
                } else {
                    this.find_prev(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SaveGoToFolder, window, cx| {
                if matches!(this.alert, Some(ActiveAlert::ConfirmSave(_))) {
                    this.open_save_goto(window, cx);
                } else {
                    this.find_prev(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseBar, window, cx| this.close_bar(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
            .on_action(cx.listener(|this, _: &ToggleWrapToPage, _, cx| {
                this.wrap_to_page = !this.wrap_to_page;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PreventEditing, _, cx| {
                this.prevent_editing = !this.prevent_editing;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RenameDocument, window, cx| {
                this.rename_document(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &MoveToFolder, window, cx| this.move_to_folder(window, cx)),
            )
            .on_action(cx.listener(|this, _: &RevertToLastSaved, window, cx| {
                this.revert_to_last_saved(window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenPageSetup, _, cx| this.open_page_setup(cx)))
            .on_action(
                cx.listener(|this, _: &QuitAndKeepWindows, _, cx| this.quit_and_keep_windows(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleRichText, _, cx| this.toggle_rich_text(cx)))
            .on_action(cx.listener(|this, _: &AlignLeft, _, cx| {
                this.set_alignment(gpui::TextAlign::Left, cx)
            }))
            .on_action(cx.listener(|this, _: &AlignCentre, _, cx| {
                this.set_alignment(gpui::TextAlign::Center, cx)
            }))
            .on_action(cx.listener(|this, _: &AlignRight, _, cx| {
                this.set_alignment(gpui::TextAlign::Right, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowRuler, _, cx| this.toggle_show_ruler(cx)))
            .on_action(cx.listener(|this, _: &CopyRuler, _, cx| this.copy_ruler(cx)))
            .on_action(cx.listener(|this, _: &PasteRuler, _, cx| this.paste_ruler(cx)))
            .on_action(cx.listener(|this, _: &OpenSpacing, _, cx| this.open_spacing(cx)))
            .on_action(cx.listener(|this, _: &ToggleDarkBackground, _, cx| {
                this.toggle_dark_background(cx)
            }))
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
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.increase_font(cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.decrease_font(cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| this.actual_size(cx)))
            .on_action(cx.listener(|_, _: &EnterFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                this.guarded(Pending::Close, window, cx)
            }))
            .on_action(cx.listener(|_, _: &CloseAll, _, cx| {
                if super::startup::document_window_count() < 2 {
                    return;
                }
                // Dispatch after this action finishes: GPUI cannot update the
                // active window recursively while its render tree handles it.
                cx.defer(|cx| {
                    for handle in cx.windows() {
                        let _ = handle.update(cx, |_, window, cx| {
                            window.dispatch_action(Box::new(CloseWindow), cx);
                        });
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::SheetWhereDocuments, _, cx| { this.save_location = SaveLocation::Documents; this.save_custom_folder = None; cx.notify(); }))
            .on_action(cx.listener(|this, _: &crate::SheetWhereDesktop, _, cx| { this.save_location = SaveLocation::Desktop; this.save_custom_folder = None; cx.notify(); }))
            .on_action(cx.listener(|this, _: &crate::SheetWhereHome, _, cx| { this.save_location = SaveLocation::Home; this.save_custom_folder = None; cx.notify(); }))
            .on_action(cx.listener(|this, _: &crate::SheetWhereDownloads, _, cx| { this.save_location = SaveLocation::Downloads; this.save_custom_folder = None; cx.notify(); }))
            .on_action(cx.listener(|this, _: &crate::SheetEncodingUtf8, _, cx| { this.text_format.encoding = document::TextEncoding::Utf8; this.refresh_dirty_state(cx); }))
            .on_action(cx.listener(|this, _: &crate::SheetEncodingUtf8Bom, _, cx| { this.text_format.encoding = document::TextEncoding::Utf8Bom; this.refresh_dirty_state(cx); }))
            .on_action(cx.listener(|this, _: &crate::SheetEncodingUtf16Le, _, cx| { this.text_format.encoding = document::TextEncoding::Utf16Le; this.refresh_dirty_state(cx); }))
            .on_action(cx.listener(|this, _: &crate::SheetEncodingUtf16Be, _, cx| { this.text_format.encoding = document::TextEncoding::Utf16Be; this.refresh_dirty_state(cx); }))
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
            .when(self.find_open || self.select_line_open, |d| {
                d.child(self.render_find_bar(layout, cx))
            })
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
                    .bg(self.text_background())
                    .child(body)
                    .into_any_element()
            } else {
                // TextEdit's plain-text view: #1E1E1E edge to edge, the text
                // origin 10 from the left and flush with the top, Menlo 11 on
                // a 13 pt pitch (JetBrains Mono stands in for Menlo). The
                // wrapper names the document for assistive technologies and
                // accepts their SetValue / ReplaceSelectedText edits.
                let editable = !(recovery_loading
                    || self.print_busy
                    || self.file_busy
                    || self.prevent_editing);
                div()
                    .id("document-body")
                    .role(Role::MultilineTextInput)
                    .aria_label(filename.clone())
                    .when_some(accessible_value, |body, value| body.aria_value(value))
                    .accessible_text_input(&self.input, cx)
                    .flex_1()
                    .min_h(px(0.0))
                    .v_flex()
                    .bg(self.text_background())
                    .when(self.rich_text && self.show_ruler, |body| {
                        body.child(self.render_ruler_bar(cx))
                    })
                    .child(
                        TextField::new(&self.input)
                            .large()
                            .flex_1()
                            .min_h(px(0.0))
                            .appearance(false)
                            .disabled(!editable)
                            .font_family(font_family)
                            .text_size(px(size))
                            .text_align(ruler.alignment)
                            .line_height(px(line_height * ruler.line_spacing))
                            .pl(px(TEXT_INSET_X))
                            .pr(px(TEXT_INSET_X))
                            .pt(px(0.0))
                            .pb(px(0.0))
                            .when(wrap_to_page, |field| field.max_w(px(page_width)).mx_auto()),
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
            .when(self.save_goto_open, |d| {
                d.child(
                    rmac_ui::dialog("text-editor-save-goto", self.render_save_goto(cx))
                        .aria_label("Go to Folder")
                        .attached(),
                )
            })
            .when(self.rename_open, |d| {
                d.child(
                    rmac_ui::dialog("text-editor-rename", self.render_rename_dialog(cx))
                        .aria_label("Rename Document")
                        .attached(),
                )
            })
            .when(self.page_setup_open, |d| {
                d.child(
                    rmac_ui::dialog("text-editor-page-setup", self.render_page_setup_dialog(cx))
                        .aria_label("Page Setup")
                        .attached(),
                )
            })
            .when(self.spacing_open, |d| {
                d.child(
                    rmac_ui::dialog("text-editor-spacing", self.render_spacing_dialog(cx))
                        .aria_label("Spacing")
                        .attached(),
                )
            })
    }
}
