//! Archive Utility's two windows, at the Mac's measured sizes
//! (design-lab/archive.html): the 378 × 138 alert and the 404 × 88 progress
//! window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    div, img, prelude::FluentBuilder as _, px, rgb, svg, Context, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

pub const ALERT_SIZE: (f32, f32) = (378.0, 138.0);
pub const PROGRESS_SIZE: (f32, f32) = (404.0, 88.0);
const TITLE_BAR: f32 = 32.0;

const ALERT_FILL: u32 = 0x23242f;
const PROGRESS_FILL: u32 = 0x24212f;
const TITLE: u32 = 0x9fa0ab;
const MESSAGE: u32 = 0xdddedf;
const LIGHT_INACTIVE: u32 = 0x464752;
const LIGHT_YELLOW: u32 = 0xf2ca44;
const LIGHT_GREEN: u32 = 0x65c466;
const ACCENT: u32 = 0x3478f6;
const TRACK: u32 = 0x363340;
const STATUS: u32 = 0x9c9ba1;
const STOP: u32 = 0x9d9ba2;
const PROGRESS_TEXT: u32 = 0xdedddf;

/// Traffic lights at the measured centres (16,16) (39,16) (62,16). The
/// window's only action is OK, so close is drawn inactive as on the Mac.
fn traffic_lights() -> impl IntoElement {
    let light = |x: f32, colour: u32| {
        div()
            .absolute()
            .left(px(x - 7.0))
            .top(px(9.0))
            .w(px(14.0))
            .h(px(14.0))
            .rounded(px(7.0))
            .bg(rgb(colour))
    };
    div()
        .child(light(16.0, LIGHT_INACTIVE))
        .child(light(39.0, LIGHT_YELLOW))
        .child(light(62.0, LIGHT_GREEN))
}

/// The draggable title strip.
fn title_strip(title: Option<&'static str>) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(TITLE_BAR))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .child(traffic_lights())
        .when_some(title, |strip, title| {
            strip.child(
                div()
                    .absolute()
                    .left(px(82.0))
                    .top(px(8.0))
                    .h(px(16.0))
                    .line_height(px(16.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(rgb(TITLE))
                    .child(title),
            )
        })
}

pub struct AlertView {
    message: SharedString,
    pub focus: FocusHandle,
}

impl AlertView {
    pub fn new(message: SharedString, cx: &mut Context<Self>) -> Self {
        Self {
            message,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for AlertView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("archive-utility-alert")
            .key_context("ArchiveUtilityAlert")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "escape") {
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .relative()
            .bg(rgb(ALERT_FILL))
            .font_family(rmac_ui::UI_FONT)
            .child(title_strip(Some("Archive Utility")))
            .child(
                img("icons/archive-utility/caution.svg")
                    .absolute()
                    .left(px(19.0))
                    .top(px(47.0))
                    .w(px(39.0))
                    .h(px(39.0)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(63.0))
                    .top(px(49.0))
                    .w(px(294.0))
                    .line_height(px(16.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(MESSAGE))
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .id("archive-utility-ok")
                    .absolute()
                    .left(px(281.0))
                    .top(px(99.0))
                    .w(px(76.0))
                    .h(px(22.0))
                    .rounded(px(11.0))
                    .bg(rgb(ACCENT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(0xffffff))
                    .child("OK")
                    .on_click(|_, window, _| window.remove_window()),
            )
    }
}

pub struct ProgressView {
    pub label: SharedString,
    pub progress: rmac_archive::Progress,
    started: Instant,
    cancel: Arc<AtomicBool>,
}

impl ProgressView {
    pub fn new(label: SharedString, cancel: Arc<AtomicBool>, started: Instant) -> Self {
        Self {
            label,
            progress: rmac_archive::Progress::default(),
            started,
            cancel,
        }
    }
}

impl Render for ProgressView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fraction = if self.progress.total == 0 {
            0.0
        } else {
            (self.progress.done as f32 / self.progress.total as f32).clamp(0.0, 1.0)
        };
        div()
            .size_full()
            .relative()
            .bg(rgb(PROGRESS_FILL))
            .font_family(rmac_ui::UI_FONT)
            .child(title_strip(None))
            .child(
                svg()
                    .path("icons/archive-utility/archive.svg")
                    .absolute()
                    .left(px(17.0))
                    .top(px(39.0))
                    .w(px(34.0))
                    .h(px(34.0))
                    .text_color(rgb(0xe6e6ea)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(61.0))
                    .top(px(33.0))
                    .w(px(311.0))
                    .h(px(16.0))
                    .line_height(px(16.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(PROGRESS_TEXT))
                    .truncate()
                    .child(self.label.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(62.0))
                    .top(px(52.5))
                    .w(px(309.0))
                    .h(px(7.0))
                    .rounded(px(3.5))
                    .bg(rgb(TRACK))
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(px(309.0 * fraction))
                            .rounded(px(3.5))
                            .bg(rgb(ACCENT)),
                    ),
            )
            .child(
                div()
                    .id("archive-utility-stop")
                    .absolute()
                    .left(px(378.0))
                    .top(px(48.5))
                    .w(px(15.0))
                    .h(px(15.0))
                    .child(
                        svg()
                            .path("icons/archive-utility/stop.svg")
                            .w(px(15.0))
                            .h(px(15.0))
                            .text_color(rgb(STOP)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel.store(true, Ordering::Release);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .absolute()
                    .left(px(61.0))
                    .top(px(64.0))
                    .w(px(311.0))
                    .line_height(px(16.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rgb(STATUS))
                    .truncate()
                    .child(rmac_archive::progress_line(
                        self.progress,
                        self.started.elapsed(),
                    )),
            )
    }
}
