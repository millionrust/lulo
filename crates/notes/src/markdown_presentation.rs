//! Read-only Markdown preview projection for Notes.

use gpui::{
    div, font, prelude::FluentBuilder as _, px, AnyElement, Context, FontFeatures,
    InteractiveElement as _, IntoElement, ParentElement, Role, StatefulInteractiveElement as _,
    StrikethroughStyle, Styled, StyledText, TextRun, Toggled, UnderlineStyle,
};
use rmac_notes_storage::{
    MarkdownPreviewBlock, MarkdownPreviewBlockKind, MarkdownPreviewDocument, TextAlign,
};
use rmac_ui::{mac, StyledExt as _};

use super::{centered_state, NotesView};

pub(super) fn render_markdown_document(
    document: &MarkdownPreviewDocument,
    light_background: bool,
    show_highlights: bool,
    chips: Option<&std::collections::BTreeMap<String, std::path::PathBuf>>,
    cx: &mut Context<NotesView>,
) -> AnyElement {
    if document.blocks().is_empty() {
        return centered_state(
            "Empty Note",
            "This note has no Markdown content to preview.",
        );
    }
    div()
        .id("markdown-preview")
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .px(px(44.0))
        .pt_2()
        .pb_6()
        .v_flex()
        .gap_3()
        .when(document.truncated(), |element| {
            element.child(
                div()
                    .p_3()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::warning_background())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::warning_text())
                    .child(
                        "Preview stopped at its safety limit. The saved note remains complete in Edit mode.",
                    ),
            )
        })
        .children(document.blocks().iter().map(|block| {
            render_markdown_block(block, light_background, show_highlights, chips, cx)
        }))
        .into_any_element()
}

/// Format ▸ Text ▸ Align Left/Centre/Align Right (NOT-MENU-042/043/045): a
/// paragraph or heading's own horizontal position, via flex `justify_*`
/// rather than a hard-coded margin. Left needs no wrapper (it is the
/// layout's own default), which also keeps the common case's element tree
/// no deeper than before this feature existed.
fn aligned(content: impl IntoElement, align: TextAlign) -> AnyElement {
    match align {
        TextAlign::Left => content.into_any_element(),
        TextAlign::Center => div()
            .w_full()
            .flex()
            .justify_center()
            .child(content)
            .into_any_element(),
        TextAlign::Right => div()
            .w_full()
            .flex()
            .justify_end()
            .child(content)
            .into_any_element(),
    }
}

fn render_markdown_block(
    block: &MarkdownPreviewBlock,
    light_background: bool,
    show_highlights: bool,
    chips: Option<&std::collections::BTreeMap<String, std::path::PathBuf>>,
    cx: &mut Context<NotesView>,
) -> AnyElement {
    let light = rmac_ui::theme::ThemeTokens::light_default().colors;
    if matches!(block.kind(), MarkdownPreviewBlockKind::ThematicBreak) {
        return div()
            .h(px(1.0))
            .w_full()
            .my_2()
            .bg(mac::separator())
            .into_any_element();
    }
    // Edit ▸ Attach File… (NOT-MENU-007): a `📎 filename` chip line with a
    // path this session still remembers renders as a clickable row instead
    // of plain text.
    if let MarkdownPreviewBlockKind::Paragraph(_) = block.kind() {
        if let Some(name) = block.text().strip_prefix("📎 ") {
            if let Some(path) = chips.and_then(|chips| chips.get(name)) {
                let path = path.clone();
                return div()
                    .id(gpui::SharedString::from(format!(
                        "notes-attachment-chip-{name}"
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::control_fill())
                    .cursor_pointer()
                    .child("📎")
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(mac::notes_accent())
                            .child(name.to_string()),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        NotesView::open_attachment_chip_path(path.clone(), cx);
                    }))
                    .into_any_element();
            }
        }
    }
    let mut text_runs = Vec::with_capacity(block.runs().len());
    for run in block.runs() {
        let style = run.style();
        let mut text_font = font(if style.code {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        });
        if style.bold {
            text_font = text_font.bold();
        }
        if style.italic {
            text_font = text_font.italic();
        }
        if style.superscript {
            text_font.features = FontFeatures(std::sync::Arc::new(vec![("sups".into(), 1)]));
        } else if style.subscript {
            text_font.features = FontFeatures(std::sync::Arc::new(vec![("subs".into(), 1)]));
        }
        let range = run.range();
        text_runs.push(TextRun {
            len: range.len(),
            font: text_font,
            color: if light_background && style.inert_placeholder {
                light.text_secondary.hsla()
            } else if light_background && style.link_label {
                light.accent.hsla()
            } else if light_background {
                light.text.hsla()
            } else if style.inert_placeholder {
                mac::text_secondary()
            } else if style.link_label {
                mac::notes_accent()
            } else {
                mac::text()
            },
            // Format ▸ Font ▸ Highlight (NOT-MENU-030): the Mac's default
            // highlight colour is the same yellow as Notes' own accent, so
            // the existing `notes_accent` design token is reused here
            // rather than a new hard-coded colour.
            background_color: if style.highlight && show_highlights {
                Some(mac::notes_accent().opacity(0.35))
            } else {
                style.code.then(|| {
                    if light_background {
                        light.control_fill.hsla()
                    } else {
                        mac::control_fill()
                    }
                })
            },
            underline: style.underline.then(|| UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: false,
            }),
            strikethrough: style.strikethrough.then(|| StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
        });
    }
    let styled = StyledText::new(block.text().to_string()).with_runs(text_runs);
    let content = div()
        .text_size(rmac_ui::text_px(16.0))
        .line_height(rmac_ui::text_px(24.0))
        .child(styled);
    match block.kind() {
        MarkdownPreviewBlockKind::Paragraph(align) => aligned(content, align).into_any_element(),
        MarkdownPreviewBlockKind::Heading(depth, align) => aligned(
            content
                .text_size(rmac_ui::text_px(match depth {
                    1 => 28.0,
                    2 => 23.0,
                    3 => 20.0,
                    _ => 17.0,
                }))
                .line_height(rmac_ui::text_px(match depth {
                    1 => 34.0,
                    2 => 29.0,
                    3 => 26.0,
                    _ => 23.0,
                }))
                .font_weight(mac::BOLD),
            align,
        )
        .into_any_element(),
        MarkdownPreviewBlockKind::BlockQuote => div()
            .pl_3()
            .border_l_2()
            .border_color(mac::separator())
            .text_color(mac::text_secondary())
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::ListItem {
            depth,
            ordered_index,
            checked,
            unordered_marker,
        } => {
            let source_range = block.source_range();
            let marker = match checked {
                // Notes draws checklist items as 18 pt circles, filled in
                // the Notes yellow with a check when done (S). Clicking the
                // circle flips the stored `- [ ] `/`- [x] ` marker back in
                // the note's Markdown body (NOTES-02); it is only
                // interactive when the parser could place its source range.
                Some(done) => {
                    let toggled = if done { Toggled::True } else { Toggled::False };
                    let element_key = source_range.as_ref().map_or(0, |range| range.start);
                    let mut circle = div()
                        .id(("notes-checklist-item", element_key))
                        .mt(px(3.0))
                        .size(px(18.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .role(Role::CheckBox)
                        .aria_toggled(toggled)
                        .aria_label(if done { "Done" } else { "Not done" })
                        .when(done, |circle| {
                            circle
                                .bg(mac::notes_accent())
                                .text_size(rmac_ui::text_px(12.0))
                                .font_weight(mac::BOLD)
                                .text_color(mac::black())
                                .child("✓")
                        })
                        .when(!done, |circle| {
                            circle.border_2().border_color(mac::text_tertiary())
                        });
                    if let Some(range) = source_range {
                        circle = circle.cursor_pointer();
                        circle = circle.on_click(cx.listener(move |view, _event, window, cx| {
                            view.toggle_checklist_range(range.clone(), window, cx);
                        }));
                    }
                    circle.into_any_element()
                }
                None => div()
                    .w(px(24.0))
                    .text_color(mac::text_secondary())
                    .child(ordered_index.map_or_else(
                        || {
                            if unordered_marker == Some('-') {
                                "–".to_string()
                            } else {
                                "•".to_string()
                            }
                        },
                        |index| format!("{index}."),
                    ))
                    .into_any_element(),
            };
            div()
                .pl(px(f32::from(depth) * 18.0))
                .flex()
                .items_start()
                .gap_2()
                .child(marker)
                .child(div().flex_1().child(content))
                .into_any_element()
        }
        MarkdownPreviewBlockKind::CodeBlock => div()
            .p_3()
            .rounded(px(rmac_ui::mac::radius_control()))
            .bg(mac::control_fill())
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::TableRow { header } => div()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(mac::separator())
            .when(header, |element| {
                element.bg(mac::control_fill()).font_weight(mac::BOLD)
            })
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::Footnote => div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(mac::text_secondary())
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::InertNotice => div()
            .p_3()
            .rounded(px(rmac_ui::mac::radius_control()))
            .bg(mac::control_fill())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(mac::text_secondary())
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::ThematicBreak => unreachable!("handled above"),
    }
}
