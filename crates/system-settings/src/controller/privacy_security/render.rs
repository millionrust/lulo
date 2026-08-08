//! Privacy & Security settings presentation.

use super::*;

mod portal;
mod security;
mod sources;

impl Settings {
    pub(in crate::controller) fn render_privacy_security(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        self.append_portal_permissions(view.clone(), &mut cards);
        self.append_security_coverage(view, &mut cards);
        self.append_application_sources(&mut cards);
        self.pane(cards)
    }
}
