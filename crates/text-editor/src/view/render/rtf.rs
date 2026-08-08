//! Text Editor bounded read-only formatted RTF preview projection.

use super::*;

impl EditorView {
    pub(super) fn render_rtf_preview(
        &self,
        layout: EditorLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .when(layout.compact, |banner| {
                banner.v_flex().items_start().gap_2()
            })
            .when(!layout.compact, |banner| {
                banner.h_flex().items_center().justify_between()
            })
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
            .px(px(layout.content_padding))
            .py(px(20.0))
            .text_size(px(size))
            .line_height(px(size * 1.5))
            .child(banner)
            .child(StyledText::new(text).with_runs(text_runs))
    }
}
