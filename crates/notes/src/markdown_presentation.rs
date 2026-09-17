//! Read-only Markdown preview projection for Notes.

use gpui::{
    div, font, prelude::FluentBuilder as _, px, AnyElement, InteractiveElement as _, IntoElement,
    ParentElement, StatefulInteractiveElement as _, StrikethroughStyle, Styled, StyledText,
    TextRun,
};
use rmac_notes_storage::{MarkdownPreviewBlock, MarkdownPreviewBlockKind, MarkdownPreviewDocument};
use rmac_ui::{mac, StyledExt as _};

use super::centered_state;

pub(super) fn render_markdown_document(document: &MarkdownPreviewDocument) -> AnyElement {
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
                    .rounded(px(mac::radius_control()))
                    .bg(mac::warning_background())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::warning_text())
                    .child(
                        "Preview stopped at its safety limit. The saved note remains complete in Edit mode.",
                    ),
            )
        })
        .children(document.blocks().iter().map(render_markdown_block))
        .into_any_element()
}

fn render_markdown_block(block: &MarkdownPreviewBlock) -> AnyElement {
    if matches!(block.kind(), MarkdownPreviewBlockKind::ThematicBreak) {
        return div()
            .h(px(1.0))
            .w_full()
            .my_2()
            .bg(mac::separator())
            .into_any_element();
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
        let range = run.range();
        text_runs.push(TextRun {
            len: range.len(),
            font: text_font,
            color: if style.inert_placeholder {
                mac::text_secondary()
            } else if style.link_label {
                mac::notes_accent()
            } else {
                mac::text()
            },
            background_color: style.code.then(mac::control_fill),
            underline: None,
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
        MarkdownPreviewBlockKind::Paragraph => content.into_any_element(),
        MarkdownPreviewBlockKind::Heading(depth) => content
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
            .font_weight(mac::BOLD)
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
        } => {
            let marker = checked.map_or_else(
                || ordered_index.map_or_else(|| "•".to_string(), |index| format!("{index}.")),
                |checked| if checked { "☑".into() } else { "☐".into() },
            );
            div()
                .pl(px(f32::from(depth) * 18.0))
                .flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .w(px(24.0))
                        .text_color(mac::text_secondary())
                        .child(marker),
                )
                .child(div().flex_1().child(content))
                .into_any_element()
        }
        MarkdownPreviewBlockKind::CodeBlock => div()
            .p_3()
            .rounded(px(mac::radius_control()))
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
            .rounded(px(mac::radius_control()))
            .bg(mac::control_fill())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(mac::text_secondary())
            .child(content)
            .into_any_element(),
        MarkdownPreviewBlockKind::ThematicBreak => unreachable!("handled above"),
    }
}
