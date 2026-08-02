use super::*;

impl FinderView {
    pub(in crate::view) fn render_quick_look(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let panel = self.quick_look.as_ref()?;
        let path = panel.paths.get(panel.current)?;
        let name = path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| root_volume_name().into());
        let count = panel.paths.len();
        let position = panel.current + 1;
        let can_previous = panel.current > 0;
        let can_next = position < count;

        let (preview, detail): (gpui::AnyElement, Option<String>) = if let Some(error) =
            &panel.error
        {
            (
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_8()
                    .child(
                        div()
                            .max_w(px(520.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rmac_ui::mac::error_border())
                            .bg(rmac_ui::mac::error_background())
                            .px_4()
                            .py_3()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(rmac_ui::mac::danger())
                            .child(error.clone()),
                    )
                    .into_any_element(),
                None,
            )
        } else {
            match panel.content.as_ref() {
                None => (
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child("Loading preview…")
                        .into_any_element(),
                    None,
                ),
                Some(quick_look::Content::Image { preview }) => (
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_5()
                        .child(
                            img(preview.clone())
                                .max_w(px(700.0))
                                .max_h(px(500.0))
                                .rounded(px(5.0)),
                        )
                        .into_any_element(),
                    Some("Image preview".into()),
                ),
                Some(quick_look::Content::Media { preview, kind }) => (
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_5()
                        .child(
                            img(preview.clone())
                                .max_w(px(700.0))
                                .max_h(px(500.0))
                                .rounded(px(5.0)),
                        )
                        .into_any_element(),
                    Some(
                        match kind {
                            rmac_thumbnails::MediaKind::Pdf => "PDF · first page",
                            rmac_thumbnails::MediaKind::Video => "Video · preview frame",
                            rmac_thumbnails::MediaKind::Audio => {
                                "Audio waveform · first 30 seconds"
                            }
                        }
                        .into(),
                    ),
                ),
                Some(quick_look::Content::MediaUnavailable { kind }) => (
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .px_8()
                        .child(icon("icons/file-fill.svg", 84.0, secondary()))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(15.0))
                                .text_color(label())
                                .child(format!("{} Preview Unavailable", kind.label())),
                        )
                        .child(
                            div()
                                .max_w(px(520.0))
                                .text_center()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(secondary())
                                .child(match kind {
                                    rmac_thumbnails::MediaKind::Pdf => {
                                        "Poppler is required to render PDF previews."
                                    }
                                    rmac_thumbnails::MediaKind::Video
                                    | rmac_thumbnails::MediaKind::Audio => {
                                        "FFmpeg is required to render media previews."
                                    }
                                }),
                        )
                        .into_any_element(),
                    Some(format!("{} preview capability unavailable", kind.label())),
                ),
                Some(quick_look::Content::Text { text, truncated }) => (
                    div()
                        .id("quick-look-text")
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .m_4()
                        .p_4()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(sep())
                        .bg(list_bg())
                        .font_family(rmac_ui::MONO_FONT)
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(label())
                        .child(text.clone())
                        .into_any_element(),
                    Some(if *truncated {
                        "Showing the first 64 KB of text".into()
                    } else {
                        "Text preview".into()
                    }),
                ),
                Some(quick_look::Content::Folder { items, truncated }) => (
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(icon("icons/folder-fill.svg", 96.0, accent()))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(15.0))
                                .text_color(label())
                                .child(if *truncated {
                                    format!("At least {items} items")
                                } else {
                                    format!("{items} item{}", if *items == 1 { "" } else { "s" })
                                }),
                        )
                        .into_any_element(),
                    Some("Folder summary".into()),
                ),
                Some(quick_look::Content::Link { target }) => (
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .px_8()
                        .child(icon("icons/file-fill.svg", 84.0, secondary()))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(secondary())
                                .child("Symbolic link to"),
                        )
                        .child(
                            div()
                                .max_w(px(560.0))
                                .font_family(rmac_ui::MONO_FONT)
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(label())
                                .child(target.clone()),
                        )
                        .into_any_element(),
                    Some("Link preview".into()),
                ),
                Some(quick_look::Content::Unsupported) => (
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(icon("icons/file-fill.svg", 84.0, secondary()))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(15.0))
                                .text_color(label())
                                .child("No Preview Available"),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(secondary())
                                .child("Press Return after closing Quick Look to open the item."),
                        )
                        .into_any_element(),
                    Some("Unsupported preview type".into()),
                ),
            }
        };

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rmac_ui::mac::scrim())
                .child(
                    div()
                        .w(px(780.0))
                        .h(px(620.0))
                        .v_flex()
                        .overflow_hidden()
                        .rounded(px(12.0))
                        .bg(rmac_ui::mac::raised())
                        .border_1()
                        .border_color(sep())
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(48.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .border_b_1()
                                .border_color(sep())
                                .child(
                                    div()
                                        .id("quick-look-close")
                                        .w(px(14.0))
                                        .h(px(14.0))
                                        .rounded_full()
                                        .bg(hsl(0xff5f57))
                                        .cursor_pointer()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.close_quick_look(cx)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .truncate()
                                        .text_center()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(name),
                                )
                                .when(count > 1, |element: Div| {
                                    element
                                        .child(
                                            div()
                                                .id("quick-look-previous")
                                                .w(px(28.0))
                                                .h(px(28.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(5.0))
                                                .when(can_previous, |button: Stateful<Div>| {
                                                    button
                                                        .cursor_pointer()
                                                        .hover(|hover| {
                                                            hover.bg(rmac_ui::mac::hover())
                                                        })
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.move_quick_look(-1, cx)
                                                        }))
                                                })
                                                .child(icon(
                                                    "icons/chevron-left.svg",
                                                    12.0,
                                                    if can_previous { label() } else { tertiary() },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("quick-look-next")
                                                .w(px(28.0))
                                                .h(px(28.0))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(5.0))
                                                .when(can_next, |button: Stateful<Div>| {
                                                    button
                                                        .cursor_pointer()
                                                        .hover(|hover| {
                                                            hover.bg(rmac_ui::mac::hover())
                                                        })
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.move_quick_look(1, cx)
                                                        }))
                                                })
                                                .child(icon(
                                                    "icons/chevron-right.svg",
                                                    12.0,
                                                    if can_next { label() } else { tertiary() },
                                                )),
                                        )
                                }),
                        )
                        .child(preview)
                        .child(
                            div()
                                .h(px(34.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .border_t_1()
                                .border_color(sep())
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(secondary())
                                .when(count > 1, |element: Div| {
                                    element.child(format!("{position} of {count}")).child("•")
                                })
                                .when_some(detail, |element, detail| element.child(detail)),
                        ),
                )
                .into_any_element(),
        )
    }
}
