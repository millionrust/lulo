//! Platform Lab root presentation.

use super::*;

impl Render for PlatformLab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("PlatformLab")
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .on_action(cx.listener(|this, _: &CopyProbe, _, cx| this.copy_probe(cx)))
            .on_action(cx.listener(|this, _: &ReadProbe, _, cx| this.read_probe(cx)))
            .on_action(cx.listener(|this, _: &OpenProbe, window, cx| this.open_probe(window, cx)))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("rmac Platform Lab — GPUI 0.2.2"))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .gap_5()
                    .p_5()
                    .overflow_hidden()
                    .child(self.render_probe_panel(cx))
                    .child(self.render_capabilities(cx)),
            )
            .child(
                div()
                    .h(px(34.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_t_1()
                    .border_color(mac::separator())
                    .bg(mac::chrome())
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} / {} • {}",
                        std::env::consts::OS,
                        std::env::consts::ARCH,
                        self.event
                    )),
            )
    }
}

pub(super) fn section_title(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(15.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text())
        .child(text)
}

pub(super) fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(12.0))
        .font_weight(mac::MEDIUM)
        .text_color(mac::text_secondary())
        .child(text)
}

pub(super) fn value(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(11.0))
        .text_color(mac::text_tertiary())
        .child(text.into())
}
