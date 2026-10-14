//! The windows Mail opens beside the viewer: File ▸ Open Messages shows a
//! message on its own, and View ▸ Message ▸ Raw Source its source.

use gpui::{
    div, px, App, AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_mail::Message;
use rmac_ui::{mac, Root};

use super::MailView;
use crate::CloseWindow;

fn open_window<V: Render + 'static>(
    title: String,
    width: f32,
    height: f32,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
    cx: &mut App,
) {
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::MAIL, width, height, cx);
    let opened = cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title(&title);
        let view = cx.new(|cx| build(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(error) => eprintln!("rmac-mail: could not open a window: {error}"),
    }
}

/// A window title for a message: its subject, as Mail names them.
fn title_for(subject: &str) -> String {
    let subject = subject.trim();
    if subject.is_empty() {
        "(No Subject)".to_owned()
    } else {
        subject.chars().take(120).collect()
    }
}

/// File ▸ Open Messages (⌘O): the message in a window of its own.
pub(super) fn open_message(message: Message, cx: &mut App) {
    let title = title_for(&message.subject);
    open_window(
        title,
        720.0,
        600.0,
        move |_, cx| MessageWindow {
            focus: cx.focus_handle(),
            message,
        },
        cx,
    );
}

/// View ▸ Message ▸ Raw Source (⌥⌘U).
pub(super) fn open_source(subject: &str, source: String, cx: &mut App) {
    let title = format!("Raw Source: {}", title_for(subject));
    open_window(
        title,
        720.0,
        600.0,
        move |_, cx| SourceWindow {
            focus: cx.focus_handle(),
            source: source.into(),
        },
        cx,
    );
}

struct MessageWindow {
    focus: FocusHandle,
    message: Message,
}

impl Render for MessageWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("mail-message-window")
            .role(Role::Document)
            .aria_label(format!("{}, {}", self.message.sender, self.message.subject))
            .track_focus(&self.focus)
            .key_context("MailMessage")
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .size_full()
            .overflow_y_scroll()
            .bg(mac::window())
            .px(px(24.0))
            .pt(px(44.0))
            .pb(px(20.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(MailView::header(&self.message))
            .child(MailView::rich_body(&self.message.body))
    }
}

struct SourceWindow {
    focus: FocusHandle,
    source: SharedString,
}

impl Render for SourceWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("mail-raw-source")
            .role(Role::Document)
            .aria_label("Raw Source")
            .track_focus(&self.focus)
            .key_context("MailMessage")
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .size_full()
            .overflow_y_scroll()
            .bg(mac::window())
            .px(px(16.0))
            .pt(px(44.0))
            .pb(px(16.0))
            .font_family(rmac_ui::MONO_FONT)
            .text_size(px(12.0))
            .text_color(mac::text())
            .child(self.source.clone())
    }
}
