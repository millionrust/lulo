//! Touch probe (docs/decisions/0028-touch-input.md): a full-screen GPUI
//! window whose regions log what the Wayland backend turned each touch into.
//! `scripts/behavior/run_touch_input.py` injects touches into a private
//! headless compositor and reads the log back. Never part of the product.
//!
//! Regions, in window coordinates (the window fills the screen):
//! - `tap` (40,40 200x100): clicks and right-clicks.
//! - `small` (120,200 16x16): a small button whose hit box grows on touch.
//! - `slider` (40,300 240x40): marked `touch_drag`, logs presses and moves.
//! - `list` (320,40 400x600): a scrolling list whose rows have `on_drag`.
//! - `pinch` (760,40 400x400): pinches and right-clicks.
//!
//! Each log line is `<ms> <event> key=value...`, in `RMAC_TOUCH_PROBE_LOG`.

use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

use gpui::{
    div, prelude::*, px, rgb, App, Bounds, ClickEvent, Context, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point, ScrollHandle, ScrollWheelEvent,
    Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;
use rmac_shell_ui::touch;

struct Log {
    started: Instant,
    path: Option<String>,
}

impl Log {
    fn write(&self, line: &str) {
        let line = format!(
            "{} {line} touch={}\n",
            self.started.elapsed().as_millis(),
            gpui::last_input_was_touch()
        );
        match &self.path {
            Some(path) => {
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                    let _ = file.write_all(line.as_bytes());
                }
            }
            None => print!("{line}"),
        }
    }
}

struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(40.0)).bg(rgb(0x4299e1))
    }
}

struct TouchProbe {
    log: std::rc::Rc<Log>,
    scroll: ScrollHandle,
    sliding: bool,
    first_frame: bool,
}

fn x(position: Point<Pixels>) -> f32 {
    f32::from(position.x)
}

impl Render for TouchProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.first_frame {
            self.first_frame = false;
            let size = window.viewport_size();
            self.log.write(&format!(
                "ready width={} height={}",
                f32::from(size.width),
                f32::from(size.height)
            ));
        }
        let log = self.log.clone();
        let slop = touch::touch_slop();
        let rows = (0..50).map(|row| {
            let log = log.clone();
            div()
                .id(("row", row))
                .h(px(60.0))
                .px_4()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(rgb(0x2d3748))
                .on_drag(row, move |row: &usize, _, _, cx| {
                    log.write(&format!("drag-start row={row}"));
                    cx.new(|_| DragPreview)
                })
                .child(format!("Row {row}"))
        });
        div()
            .id("touch-probe")
            .size_full()
            .relative()
            .bg(rgb(0x171923))
            .text_color(rgb(0xf7fafc))
            .child(
                div()
                    .id("tap")
                    .absolute()
                    .left(px(40.0))
                    .top(px(40.0))
                    .w(px(200.0))
                    .h(px(100.0))
                    .bg(rgb(0x3182ce))
                    .on_click({
                        let log = log.clone();
                        move |event: &ClickEvent, _, _| {
                            log.write(&format!(
                                "click target=tap button=left count={}",
                                event.click_count()
                            ))
                        }
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let log = log.clone();
                        move |_: &MouseDownEvent, _, _| log.write("right target=tap")
                    })
                    .child("Tap"),
            )
            .child(
                touch::hit_size(
                    div()
                        .id("small")
                        .absolute()
                        .left(px(120.0))
                        .top(px(200.0))
                        .flex()
                        .items_center()
                        .justify_center(),
                    16.0,
                    16.0,
                    slop,
                )
                .on_click({
                    let log = log.clone();
                    move |_: &ClickEvent, _, _| log.write(&format!("click target=small slop={slop}"))
                })
                .child(div().size(px(16.0)).rounded_full().bg(rgb(0xe53e3e))),
            )
            .child(
                div()
                    .id("slider")
                    .absolute()
                    .left(px(40.0))
                    .top(px(300.0))
                    .w(px(240.0))
                    .h(px(40.0))
                    .bg(rgb(0x4a5568))
                    .touch_drag()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.sliding = true;
                            this.log.write(&format!("slider-down x={}", x(event.position)));
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                        if this.sliding && event.pressed_button == Some(MouseButton::Left) {
                            this.log.write(&format!("slider-move x={}", x(event.position)));
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, _| {
                            if this.sliding {
                                this.sliding = false;
                                this.log.write(&format!("slider-up x={}", x(event.position)));
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .id("list")
                    .absolute()
                    .left(px(320.0))
                    .top(px(40.0))
                    .w(px(400.0))
                    .h(px(600.0))
                    .bg(rgb(0x1a202c))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, _| {
                        let delta = event.delta.pixel_delta(px(20.0));
                        this.log.write(&format!(
                            "scroll target=list phase={:?} dx={} dy={} offset={}",
                            event.touch_phase,
                            f32::from(delta.x),
                            f32::from(delta.y),
                            -f32::from(this.scroll.offset().y)
                        ));
                    }))
                    .on_mouse_down(MouseButton::Right, {
                        let log = log.clone();
                        move |_: &MouseDownEvent, _, _| log.write("right target=list")
                    })
                    .children(rows),
            )
            .child(
                div()
                    .id("pinch")
                    .absolute()
                    .left(px(760.0))
                    .top(px(40.0))
                    .w(px(400.0))
                    .h(px(400.0))
                    .bg(rgb(0x2f855a))
                    .on_pinch({
                        let log = log.clone();
                        move |event: &PinchEvent, _, _| {
                            log.write(&format!(
                                "pinch target=pinch phase={:?} delta={:.4}",
                                event.phase, event.delta
                            ))
                        }
                    })
                    .on_mouse_down(MouseButton::Right, {
                        let log = log.clone();
                        move |_: &MouseDownEvent, _, _| log.write("right target=pinch")
                    })
                    .child("Pinch"),
            )
    }
}

fn main() {
    let log = std::rc::Rc::new(Log {
        started: Instant::now(),
        path: std::env::var("RMAC_TOUCH_PROBE_LOG").ok(),
    });
    application().run(move |cx: &mut App| {
        let bounds = cx
            .primary_display()
            .map(|display| display.bounds())
            .unwrap_or_else(|| Bounds::centered(None, gpui::size(px(1280.0), px(800.0)), cx));
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Fullscreen(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Touch probe".into()),
                    ..Default::default()
                }),
                app_id: Some("dev.rmac.TouchProbe".into()),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| TouchProbe {
                    log,
                    scroll: ScrollHandle::new(),
                    sliding: false,
                    first_frame: true,
                })
            },
        )
        .expect("open the touch probe window");
        cx.activate(true);
    });
}
