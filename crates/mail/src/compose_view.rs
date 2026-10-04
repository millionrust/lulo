//! Compose window (MAIL-8 seam). MAIL-6 supplies the real compose window
//! (address completion, attachments, drafts, send); until it lands, this
//! is the minimal, additive stand-in that proves two MAIL-8 contracts end
//! to end: a `mailto:` launch opens a prefilled draft, and the account's
//! signature is inserted automatically (`rmac_mail::settings`). Layout
//! follows `design-lab/mail.html`'s compose mock (640 × 560, borderless
//! header fields on hairlines). Sending is MAIL-3/MAIL-6/MAIL-7's job, so
//! Send stays disabled here.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use rmac_mail_mime::Draft;
use rmac_ui::{mac, Root, StyledExt as _};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 560.0;

/// Opens a new Compose window for `draft`, with `account_address`'s
/// signature (Settings ▸ Signatures) appended to the body first. Mail
/// allows several Compose windows at once, as the Mac does, so this never
/// reuses an existing one.
pub fn show(draft: Draft, account_address: &str, cx: &mut App) {
    let (settings, _error) = rmac_mail::settings::load();
    let draft = settings.compose_draft_with_signature(draft, account_address);
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::MAIL, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let title = if draft.subject.is_empty() {
            "New Message".to_owned()
        } else {
            draft.subject.clone()
        };
        window.set_window_title(&title);
        let view = cx.new(|cx| ComposeView::new(draft.clone(), cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Err(error) = opened {
        eprintln!("rmac-mail: could not open Compose: {error}");
    }
}

struct ComposeView {
    focus: FocusHandle,
    draft: Draft,
}

impl ComposeView {
    fn new(draft: Draft, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            draft,
        }
    }

    fn field(&self, label: &'static str, value: &str) -> impl IntoElement {
        div()
            .h(px(34.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(mac::separator())
            .text_size(px(13.0))
            .child(div().text_color(mac::text_secondary()).child(label))
            .child(div().text_color(mac::text()).child(value.to_owned()))
    }
}

impl Render for ComposeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.draft.subject.is_empty() {
            "New Message".to_owned()
        } else {
            self.draft.subject.clone()
        };
        div()
            .size_full()
            .v_flex()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Mail")
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(13.0))
                    .child(title),
            ))
            .child(self.field("To:", &self.draft.to.join(", ")))
            .when(!self.draft.cc.is_empty(), |parent| {
                parent.child(self.field("Cc:", &self.draft.cc.join(", ")))
            })
            .child(self.field("Subject:", &self.draft.subject))
            .child(
                div()
                    .id("compose-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(16.0))
                    .py(px(12.0))
                    .text_size(px(13.0))
                    .text_color(mac::text())
                    .child(self.draft.text.clone()),
            )
    }
}
