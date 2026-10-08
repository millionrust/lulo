//! A small, one-time notice under the right end of the menu bar, as a Mac
//! notification banner sits: today only "Spotlight opens with …" when
//! another app holds Alt+Space. It closes on a click or after a while,
//! and its window is gone with it, so it costs nothing afterwards.

use std::time::Duration;

use gpui::{
    div, px, App, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Render, Role, SharedString, Styled as _, Window,
};
use rmac_ui::mac;
use windows::Win32::Foundation::RECT;

use super::{runtime, Surface};
use crate::win::{surface, trace, windows_list};

const WIDTH: f32 = 340.0;
const HEIGHT: f32 = 66.0;
/// Room around the banner for its shadow.
const MARGIN: f32 = 10.0;
const SHOWN_FOR: Duration = Duration::from_secs(12);

pub(crate) struct NoticeView {
    title: SharedString,
    detail: SharedString,
}

impl Render for NoticeView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p(px(MARGIN)).child(
            div()
                .id("lulo-notice")
                .role(Role::Alert)
                .aria_label(self.title.clone())
                .size_full()
                .px(px(14.0))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.0))
                .rounded(px(mac::radius_large_surface()))
                .bg(mac::menu_surface())
                .border_1()
                .border_color(mac::separator())
                .shadow(mac::menu_shadow())
                .text_color(mac::text())
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                    cx.defer(close);
                })
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(mac::SEMIBOLD)
                        .child(self.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(self.detail.clone()),
                ),
        )
    }
}

struct OpenNotice(Surface);

impl gpui::Global for OpenNotice {}

/// Show the notice; it closes itself after a while.
pub(crate) fn show(title: String, detail: String, cx: &mut App) {
    if cx.has_global::<OpenNotice>() {
        return;
    }
    let runtime = runtime(cx);
    let scale = runtime.scale;
    let bar = runtime.bar_rect;
    let (monitor, _) = surface::primary_monitor();
    let width = ((WIDTH + 2.0 * MARGIN) * scale).round() as i32;
    let height = ((HEIGHT + 2.0 * MARGIN) * scale).round() as i32;
    let right = monitor.right - (4.0 * scale).round() as i32;
    let top = bar.bottom.max(monitor.top);
    let rect = RECT {
        left: right - width,
        top,
        right,
        bottom: top + height,
    };
    let (title, detail) = (SharedString::from(title), SharedString::from(detail));
    let traced = title.clone();
    let Some(notice) = super::open_surface(super::logical(rect, scale), false, cx, move |_, _| {
        NoticeView { title, detail }
    }) else {
        return;
    };
    cx.set_global(OpenNotice(notice));
    super::later(cx, move || {
        surface::show_at(windows_list::handle(notice.hwnd), rect)
    });
    trace(|| format!("notice shown: {traced}"));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SHOWN_FOR).await;
        cx.update(close);
    })
    .detach();
}

/// Close the notice and drop its window.
pub(crate) fn close(cx: &mut App) {
    if !cx.has_global::<OpenNotice>() {
        return;
    }
    let OpenNotice(notice) = cx.remove_global::<OpenNotice>();
    let _ = notice
        .handle
        .update(cx, |_, window, _| window.remove_window());
    trace(|| "notice closed".into());
}
