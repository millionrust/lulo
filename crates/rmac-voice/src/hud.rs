//! The listening pill (design-lab/voice-pill.html).
//!
//! 1. The dispatcher's datagram on `shortcut-voice.sock` is the key going
//!    down (the dispatcher started this unit for it). The capture starts at
//!    once through `org.rmac.Voice1`, and the pill opens with the keyboard,
//!    so it hears ⌥ being let go: niri swallows the release of the key it
//!    bound (Space) but not of the modifier, which was down before the bind.
//! 2. ⌥ still down when the pill gets the keyboard: listen until it comes
//!    up. Already up (a quick tap), or no keyboard news at all: listen
//!    hands-free until the speech and a pause, or a second press.
//! 3. The transcript goes to Spotlight through [`handoff`]; Escape, or an
//!    empty transcript, ends without it.
//!
//! The meter redraws only when the service sends a level (about 20 times
//! a second, only while listening). Nothing polls; the process exits when
//! it is done, or 5 s after starting if no press arrives.

use std::borrow::Cow;
use std::time::Duration;

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::{
    div, point, prelude::*, px, size, svg, App, AssetSource, Bounds, Context, FocusHandle,
    KeyDownEvent, ModifiersChangedEvent, Role, SharedString, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions,
};
use rmac_intelligence::voice::client::{self, Control, Event, VoiceError};
use rmac_intelligence::voice::handoff;
use rmac_shortcuts::ShortcutId;

use crate::model::{message_for, KeyChange, Meter, Mode, BARS, NOTHING_HEARD};

const SHORTCUT: &str = "voice";
/// The window: room for the widest message, and the pill's shadow.
const WINDOW_WIDTH: f32 = 460.0;
const WINDOW_HEIGHT: f32 = 64.0;
/// The pill while listening (measured against the Mac's dictation pill).
const PILL_HEIGHT: f32 = 44.0;
const MIC_DIAMETER: f32 = 28.0;
const BAR_WIDTH: f32 = 3.0;
const BAR_GAP: f32 = 3.0;
const BAR_MAX: f32 = 20.0;
/// No ⌥ news this long after the pill opened: treat the press as a tap.
const KEYBOARD_WAIT: Duration = Duration::from_millis(700);
/// No press this long after starting: the dispatcher's datagram was lost.
const START_WAIT: Duration = Duration::from_secs(5);
/// How long a message stays up.
const MESSAGE_TIME: Duration = Duration::from_millis(2_200);

const MIC_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="2" width="4" height="7.5" rx="2" fill="currentColor"/><path d="M3.8 7.8a4.2 4.2 0 0 0 8.4 0M8 12v2"/></svg>"#;

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok((path == "voice/mic.svg").then_some(Cow::Borrowed(MIC_SVG)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(if "voice/mic.svg".starts_with(path) {
            vec!["voice/mic.svg".into()]
        } else {
            Vec::new()
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Listening,
    Transcribing,
    Message(&'static str),
}

struct Pill {
    phase: Phase,
    mode: Mode,
    meter: Meter,
    focus: FocusHandle,
    controls: async_channel::Sender<Control>,
    was_active: bool,
}

impl Pill {
    fn new(
        controls: async_channel::Sender<Control>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active && this.mode == Mode::Held {
                // The keyboard went elsewhere while the key was held: the
                // release can no longer be seen, so this counts as it.
                this.release(cx);
            }
        })
        .detach();
        // No word of the keyboard in time (it never came, or ⌥ was already
        // up): listen hands-free.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEYBOARD_WAIT).await;
            let _ = this.update(cx, |this, _| {
                if this.mode == Mode::Undecided {
                    this.mode = Mode::HandsFree;
                    rmac_ui::trace_mark("voice_hands_free");
                    let _ = this.controls.try_send(Control::EndOnPause);
                }
            });
        })
        .detach();
        Self {
            phase: Phase::Listening,
            mode: Mode::Undecided,
            meter: Meter::default(),
            focus,
            controls,
            was_active: false,
        }
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Listening {
            rmac_ui::trace_mark("voice_released");
            let _ = self.controls.try_send(Control::Stop);
            self.phase = Phase::Transcribing;
            self.mode = Mode::HandsFree;
            cx.notify();
        }
    }

    fn modifiers_changed(&mut self, alt: bool, cx: &mut Context<Self>) {
        let (mode, change) = self.mode.on_modifiers(alt);
        self.mode = mode;
        match change {
            KeyChange::Holding => rmac_ui::trace_mark("voice_holding"),
            KeyChange::Tapped => {
                rmac_ui::trace_mark("voice_hands_free");
                let _ = self.controls.try_send(Control::EndOnPause);
            }
            KeyChange::Released => self.release(cx),
            KeyChange::Nothing => {}
        }
    }

    /// A second press of the talk key: in hands-free mode it ends listening.
    fn pressed_again(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Held {
            self.release(cx);
        }
    }

    fn event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Listening => rmac_ui::trace_mark("voice_listening"),
            Event::Level(level) => {
                if self.phase == Phase::Listening {
                    self.meter.push(level);
                    cx.notify();
                }
            }
            Event::Transcribing => {
                if self.phase == Phase::Listening {
                    self.phase = Phase::Transcribing;
                    cx.notify();
                }
            }
        }
    }

    fn label(&self) -> &'static str {
        match &self.phase {
            Phase::Listening => "Listening…",
            Phase::Transcribing => "Working…",
            Phase::Message(message) => message,
        }
    }
}

impl Render for Pill {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rmac_ui::theme::current();
        let text = theme.colors.text.hsla();
        let secondary = theme.colors.text_secondary.hsla();
        let listening = self.phase == Phase::Listening;
        let message = matches!(self.phase, Phase::Message(_));
        let bars = self.meter.bars();
        let mut pill = div()
            .id("voice-pill")
            .role(Role::Status)
            .aria_label(self.label())
            .h(px(PILL_HEIGHT))
            .flex()
            .items_center()
            .gap(px(10.0))
            .pl(px(8.0))
            .pr(px(16.0))
            .rounded_full()
            .bg(rmac_ui::mac::material_hud())
            .shadow(rmac_ui::mac::menu_shadow());
        if !message {
            pill = pill
                .child(
                    div()
                        .size(px(MIC_DIAMETER))
                        .flex_none()
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(if listening {
                            rmac_ui::mac::system_orange()
                        } else {
                            rmac_ui::mac::system_gray()
                        })
                        .child(
                            svg()
                                .path("voice/mic.svg")
                                .size(px(15.0))
                                .text_color(rmac_ui::mac::white()),
                        ),
                )
                .child(
                    div()
                        .h(px(BAR_MAX))
                        .flex()
                        .items_center()
                        .gap(px(BAR_GAP))
                        .children(bars.iter().map(|height| {
                            let height = if listening { *height } else { 0.15 };
                            div()
                                .w(px(BAR_WIDTH))
                                .h(px((BAR_MAX * height).max(BAR_WIDTH)))
                                .rounded_full()
                                .bg(if listening { text } else { secondary })
                        })),
                );
        } else {
            pill = pill.pl(px(16.0));
        }
        let pill = pill.child(
            div()
                .text_size(px(13.0))
                .text_color(if message { text } else { secondary })
                .whitespace_nowrap()
                .child(self.label()),
        );
        div()
            .size_full()
            .flex()
            .justify_center()
            .items_start()
            .track_focus(&self.focus)
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.modifiers_changed(event.modifiers.alt, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    rmac_ui::trace_mark("voice_cancelled");
                    let _ = this.controls.try_send(Control::Cancel);
                    this.phase = Phase::Transcribing;
                    cx.notify();
                }
            }))
            .child(pill)
    }
}

fn window_options(cx: &App) -> WindowOptions {
    // Where Spotlight's bar opens (a fifth of the way down), so the
    // transcript appears in the same place the pill was.
    let margin_top = cx
        .primary_display()
        .map(|display| f32::from(display.bounds().size.height) * 0.2)
        .unwrap_or(160.0);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.0), px(0.0)),
            size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "rmac-voice".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP,
            margin: Some((px(margin_top), px(0.0), px(0.0), px(0.0))),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.rmac.Voice".into()),
        ..Default::default()
    }
}

/// Close the pill (if open) and leave.
fn finish(window: Option<gpui::AnyWindowHandle>, cx: &mut gpui::AsyncApp) {
    cx.update(|cx| {
        if let Some(window) = window {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
        cx.quit();
    });
}

/// Show `message` in the pill for a moment, then leave.
async fn show_message(
    pill: &gpui::WeakEntity<Pill>,
    window: Option<gpui::AnyWindowHandle>,
    message: &'static str,
    cx: &mut gpui::AsyncApp,
) {
    let _ = pill.update(cx, |pill, cx| {
        pill.phase = Phase::Message(message);
        cx.notify();
    });
    cx.background_executor().timer(MESSAGE_TIME).await;
    finish(window, cx);
}

/// One press: listen, then hand the transcript to Spotlight.
fn begin(presses: async_channel::Receiver<()>, cx: &mut App) {
    rmac_ui::trace_mark("voice_pressed");
    let (controls, control_rx) = async_channel::bounded(4);
    let (events_tx, events) = async_channel::bounded::<Event>(64);
    // The microphone first: everything else overlaps with it starting.
    let capture = cx.background_executor().spawn(async move {
        client::listen(false, control_rx, move |event| {
            let _ = events_tx.try_send(event);
        })
        .await
    });
    let mut pill_entity = None;
    let window = cx
        .open_window(window_options(cx), |window, cx| {
            window.set_window_title("Talk to Lulo");
            rmac_ui::prepare_surface_window(window, cx);
            let pill = cx.new(|cx| Pill::new(controls.clone(), window, cx));
            pill_entity = Some(pill.downgrade());
            cx.new(|cx| rmac_ui::shell_surface_root(pill, window, cx))
        })
        .ok()
        .map(|handle| -> gpui::AnyWindowHandle { handle.into() });
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
    let Some(pill) = pill_entity else {
        // No window: still never leave the microphone open.
        let _ = controls.try_send(Control::Cancel);
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let _ = capture.await;
            finish(None, cx);
        })
        .detach();
        return;
    };
    // Levels and states into the pill.
    let levels_pill = pill.clone();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(event) = events.recv().await {
            if levels_pill
                .update(cx, |pill, cx| pill.event(event, cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
    // Further presses of the key.
    let presses_pill = pill.clone();
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while presses.recv().await.is_ok() {
            if presses_pill
                .update(cx, |pill, cx| pill.pressed_again(cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
    // The outcome.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let outcome = capture.await;
        drop(controls);
        match outcome {
            Ok(transcript) if transcript.text.trim().is_empty() => {
                rmac_ui::trace_mark("voice_nothing_heard");
                show_message(&pill, window, NOTHING_HEARD, cx).await;
            }
            Ok(transcript) => {
                rmac_ui::trace_mark(&format!(
                    "voice_transcript:stop_to_text_ms={:.0}:audio_ms={:.0}:waited_for_model_ms={:.0}:model_load_ms={:.0}:start_to_audio_ms={:.0}",
                    transcript.timing.stop_to_text_ms,
                    transcript.timing.audio_ms,
                    transcript.timing.waited_for_model_ms,
                    transcript.timing.model_load_ms,
                    transcript.timing.start_to_audio_ms,
                ));
                let text = transcript.text;
                let handed = blocking::unblock(move || {
                    handoff::write(&text).map_err(|error| error.to_string())?;
                    rmac_shortcuts::dispatch(&ShortcutId("launcher".into()))
                        .map_err(|error| error.to_string())
                })
                .await;
                match handed {
                    Ok(()) => {
                        rmac_ui::trace_mark("voice_handed_to_spotlight");
                        finish(window, cx);
                    }
                    Err(error) => {
                        eprintln!("rmac-voice-hud: Spotlight did not open: {error}");
                        show_message(&pill, window, "Spotlight did not open.", cx).await;
                    }
                }
            }
            Err(error) => {
                if error != VoiceError::Cancelled {
                    eprintln!("rmac-voice-hud: {error}");
                }
                match message_for(&error) {
                    Some(message) => show_message(&pill, window, message, cx).await,
                    None => finish(window, cx),
                }
            }
        }
    })
    .detach();
}

pub fn run() {
    rmac_ui::application()
        .with_assets(rmac_ui::layered_assets(Assets))
        .with_quit_mode(gpui::QuitMode::Explicit)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            let (press_tx, press_rx) = async_channel::bounded(4);
            let (ready_tx, _ready_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_shortcuts::watch_dispatches_ready(
                        ShortcutId(SHORTCUT.into()),
                        press_tx,
                        ready_tx,
                    )
                    .await
                    {
                        eprintln!("rmac-voice-hud: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                let first = futures_util::FutureExt::fuse(press_rx.recv());
                let timeout = futures_util::FutureExt::fuse(
                    cx.background_executor().timer(START_WAIT),
                );
                futures_util::pin_mut!(first, timeout);
                let pressed = futures_util::select! {
                    press = first => press.is_ok(),
                    _ = timeout => false,
                };
                if !pressed {
                    finish(None, cx);
                    return;
                }
                let (presses_tx, presses) = async_channel::bounded(4);
                cx.update(|cx| begin(presses, cx));
                while let Ok(rmac_shortcuts::Event::Activated { .. }) = press_rx.recv().await {
                    if presses_tx.send(()).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
        });
}
