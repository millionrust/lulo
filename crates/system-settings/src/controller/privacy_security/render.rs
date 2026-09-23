//! Privacy & Security settings presentation, laid out like macOS 26: the
//! Privacy header card, the portal-backed Camera and Microphone rows, then
//! Security (design-lab/settings.html).

use super::*;

mod portal;
mod security;

impl Settings {
    pub(in crate::controller) fn render_privacy_security(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = vec![header_card(
            tile26("icons/shield.svg", accent()),
            "Privacy",
            "Control which applications can use your camera and microphone.",
            None,
        )];
        self.append_portal_permissions(view.clone(), &mut cards);
        self.append_security_coverage(view.clone(), &mut cards);
        let refresh_view = view;
        cards.push(footer_buttons(vec![push_button(
            "privacy-refresh",
            "Refresh",
        )
        .disabled(
            self.privacy_loading
                || self.privacy_busy.is_some()
                || self.privacy_stream_refreshing
                || self.security_coverage_loading,
        )
        .on_click(move |_, _, cx| {
            refresh_view.update(cx, |settings, cx| {
                settings.refresh_privacy(cx);
                settings.refresh_security_coverage(cx);
            });
        })
        .into_any_element()]));
        self.pane(cards)
    }
}
