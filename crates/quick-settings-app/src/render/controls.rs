//! Control Center Display and Sound slider modules and error banners.

use gpui::{ClickEvent, MouseDownEvent};
use rmac_quick_settings::detail::{Detail, Module};
use rmac_quick_settings::{Control, SoundValue, Tile};

use super::*;

/// The accessory circle at the right end of a slider module.
const ACCESSORY: f32 = 26.0;
const ACCESSORY_CENTRE: (f32, f32) = (265.0, 42.0);
/// Slider tracks are centred 42 below the module top.
const TRACK_CENTRE: f32 = 42.0;
/// Resting track thickness. The Mac's measures ≈4.5 pt (2× capture,
/// 2026-10-08); 5 keeps the filled part legible on a 1× low-DPI panel.
pub(super) const TRACK_HEIGHT: f32 = 5.0;
/// Track thickness while the pointer is over the slider or it is held
/// (CC-13): macOS 26 thickens it and shows its knob.
pub(super) const TRACK_HEIGHT_BULGED: f32 = 8.0;
/// The knob: a white capsule with a soft shadow, ≈17 × 14 pt on the Mac.
const KNOB_WIDTH: f32 = 18.0;
const KNOB_HEIGHT: f32 = 14.0;
/// How far a held slider's module grows on every side, at the end of its
/// spring (it briefly overshoots, then settles here until release).
const PRESS_GROW: f32 = 3.0;
/// The › beside a hovered Display or Sound title, which opens its view.
const CHEVRON: (&str, f32, f32) = ("cc/chevron.svg", 7.0, 12.0);
const CHEVRON_CENTRE: (f32, f32) = (276.0, 18.0);

/// Knob shadow: black 30 %, 0.5 down, 2.5 blur.
fn knob_shadow() -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow::new(px(0.0), px(0.5), color(0x0000_004d)).blur_radius(px(2.5))]
}

/// A slider's track, filled part and knob, `width` long, laid out in the
/// 24 pt hit box 8 in from its left edge. `bulge` (0..=1) thickens the
/// track; `knob` (0..=1) fades the knob in.
pub(super) fn slider_bar(width: f32, value: u8, fill: Hsla, bulge: f32, knob: f32) -> Div {
    let track_height = rmac_ui::slider_bulge_lerp(TRACK_HEIGHT, TRACK_HEIGHT_BULGED, bulge);
    let fraction = f32::from(value.min(100)) / 100.0;
    let filled = if value == 0 {
        0.0
    } else {
        (width * fraction).max(track_height)
    };
    let knob_centre = (width * fraction).clamp(KNOB_WIDTH / 2.0, width - KNOB_WIDTH / 2.0);
    div()
        .absolute()
        .left(px(8.0))
        .top_0()
        .w(px(width))
        .h(px(24.0))
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(12.0 - track_height / 2.0))
                .w(px(width))
                .h(px(track_height))
                .rounded_full()
                .overflow_hidden()
                .bg(slider_track())
                .child(div().h_full().w(px(filled)).rounded_full().bg(fill)),
        )
        .when(knob > 0.0, |bar| {
            bar.child(
                div()
                    .absolute()
                    .left(px(knob_centre - KNOB_WIDTH / 2.0))
                    .top(px(12.0 - KNOB_HEIGHT / 2.0))
                    .w(px(KNOB_WIDTH))
                    .h(px(KNOB_HEIGHT))
                    .rounded_full()
                    .bg(mac::white())
                    .shadow(knob_shadow())
                    .opacity(knob.clamp(0.0, 1.0)),
            )
        })
}

impl QuickSettingsView {
    /// Title, track and end glyphs shared by Display and Sound.
    #[allow(clippy::too_many_arguments)]
    fn slider_module(
        &self,
        kind: SliderKind,
        y: f32,
        title: &'static str,
        value: u8,
        fill: Hsla,
        enabled: bool,
        start: (&'static str, f32, f32, f32),
        end: (&'static str, f32, f32, f32),
        accessory: (&'static str, f32, f32, &'static str),
        cx: &Context<Self>,
    ) -> AnyElement {
        let (left, width) = track(kind);
        let (accessory_path, accessory_width, accessory_height, pane) = accessory;
        let id = match kind {
            SliderKind::Brightness | SliderKind::DetailBrightness => "display-slider",
            SliderKind::Volume | SliderKind::DetailVolume => "sound-slider",
        };
        let module_kind = if kind == SliderKind::Brightness {
            Module::Display
        } else {
            Module::Sound
        };
        let detail = if kind == SliderKind::Brightness {
            Detail::Display
        } else {
            Detail::Sound
        };
        let ring = self.ring(module_kind);
        // CC-13: the track thickens and the knob appears while hovered or
        // held, within the hit box so neighbouring controls never shift;
        // holding it also springs the whole module a little larger.
        let bulge = self.slider_bulge(kind);
        let grow = PRESS_GROW * self.slider_press(kind).max(0.0);
        let increment_view = cx.entity().downgrade();
        let decrement_view = cx.entity().downgrade();
        let set_value_view = cx.entity().downgrade();
        let step = 5;
        let slider_focus = if kind == SliderKind::Brightness {
            self.display_slider_focus.clone()
        } else {
            self.sound_slider_focus.clone()
        };
        let title_focus = if kind == SliderKind::Brightness {
            self.display_title_focus.clone()
        } else {
            self.sound_title_focus.clone()
        };
        // The title and the module's empty space open the detail view, as
        // on the Mac. Drawn first, so the slider and the accessory above
        // it keep their own presses.
        let title_hit = div()
            .id(SharedString::from(format!("{id}-title")))
            .role(Role::Button)
            .aria_label(format!("{title} details"))
            .track_focus(&title_focus.tab_stop(true).tab_index(0))
            .on_a11y_action(AccessibleAction::Click, {
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.open_detail(detail, window, cx));
                }
            })
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .rounded(px(layout::MODULE_RADIUS as f32))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.set_module_hovered(module_kind, *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_detail(detail, window, cx)
            }));
        let hit = div()
            .id(id)
            .role(Role::Slider)
            .aria_label(title)
            .aria_numeric_value(f64::from(value))
            .aria_numeric_value_step(5.0)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(100.0)
            // A real Tab stop unconditionally (ACC orca audit: the Sound
            // slider was pointer-only whenever its backend reported
            // unavailable, and Display's own arrow keys had no effect
            // because the window's capture-phase key_down always claimed
            // left/right for the module grid's separate mouse/arrow roving
            // state (`module_focus`), which real Tab focus never updated).
            // `key_down` (view.rs) now checks this handle directly, so
            // Left/Right nudge whichever slider Tab actually reached. Only
            // the value-changing actions below stay gated on `enabled`,
            // matching the mouse-drag handler further down.
            .track_focus(&slider_focus.tab_stop(true).tab_index(0))
            .when(enabled, |hit| {
                hit.on_a11y_action(AccessibleAction::Increment, move |_, _, cx| {
                    let _ = increment_view.update(cx, |this, cx| {
                        this.slide(kind, value.saturating_add(step).min(100), cx)
                    });
                })
                .on_a11y_action(AccessibleAction::Decrement, move |_, _, cx| {
                    let _ = decrement_view.update(cx, |this, cx| {
                        this.slide(kind, value.saturating_sub(step), cx)
                    });
                })
                .on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
                    if let Some(accesskit::ActionData::NumericValue(requested)) = data {
                        let value = requested.round().clamp(0.0, 100.0) as u8;
                        let _ = set_value_view.update(cx, |this, cx| this.slide(kind, value, cx));
                    }
                })
            })
            .absolute()
            .left(px(left - 8.0))
            .top(px(TRACK_CENTRE - 12.0))
            .w(px(width + 16.0))
            .h(px(24.0))
            .when(enabled, |hit| {
                hit.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        // A press on the track is not a click on the title.
                        cx.stop_propagation();
                        this.dragging = Some(kind);
                        this.sync_slider_bulge(kind, cx);
                        let value = super::slider_value(kind, f32::from(event.position.x));
                        this.slide(kind, value, cx);
                    }),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.set_slider_hovered(kind, *hovered, cx);
                }))
            })
            .child(slider_bar(width, value, fill, bulge, bulge));
        let accessory = div()
            .id(pane)
            .role(Role::Button)
            .aria_label(if kind == SliderKind::Brightness {
                "Display Options"
            } else {
                "Sound Outputs"
            })
            .focusable()
            .tab_stop(true)
            .on_a11y_action(AccessibleAction::Click, {
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.open_detail(detail, window, cx));
                }
            })
            .absolute()
            .left(px(ACCESSORY_CENTRE.0 - ACCESSORY / 2.0))
            .top(px(ACCESSORY_CENTRE.1 - ACCESSORY / 2.0))
            .size(px(ACCESSORY))
            .rounded_full()
            .bg(circle_off())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                // The monitor button opens the Display view and Sound's
                // output button lists the outputs, as on the Mac.
                this.open_detail(detail, window, cx)
            }))
            // Its press is not a click on the title behind it. Registered
            // after `on_click`, so this element's own click still sees it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(glyph_at(
                accessory_path,
                ACCESSORY / 2.0,
                ACCESSORY / 2.0,
                accessory_width,
                accessory_height,
                ink(),
            ));
        let glyph = |(path, centre, width, height): (&'static str, f32, f32, f32)| {
            glyph_at(path, centre, TRACK_CENTRE, width, height, ink())
        };
        let chevron = self.hovered_module == Some(module_kind) || ring;
        let radius = layout::MODULE_RADIUS as f32;
        module(
            -grow,
            y - grow,
            GRID_WIDTH + 2.0 * grow,
            CELL + 2.0 * grow,
            radius + grow,
        )
        .when(ring, |module| module.shadow(mac::focus_ring_shadow()))
        .when(!enabled, |module| module.opacity(0.5))
        .child(
            // The content keeps the resting coordinates while the glass
            // behind it grows.
            div()
                .absolute()
                .left(px(grow))
                .top(px(grow))
                .w(px(GRID_WIDTH))
                .h(px(CELL))
                .child(title_hit)
                .child(text_at(16.5, 23.0, 13.0, mac::BOLD, title_text(), title))
                .when(chevron, |content| {
                    content.child(glyph_at(
                        CHEVRON.0,
                        CHEVRON_CENTRE.0,
                        CHEVRON_CENTRE.1,
                        CHEVRON.1,
                        CHEVRON.2,
                        subtitle_text(),
                    ))
                })
                .child(glyph(start))
                .child(glyph(end))
                .child(hit)
                .child(accessory),
        )
        .into_any_element()
    }

    pub(super) fn display_module(&self, y: f32, level: u8, cx: &Context<Self>) -> AnyElement {
        self.slider_module(
            SliderKind::Brightness,
            y,
            "Display",
            level,
            mac::white(),
            true,
            ("cc/sun-min.svg", 22.75, 14.5, 14.5),
            ("cc/sun-max.svg", 233.5, 16.0, 16.0),
            ("cc/display.svg", 15.0, 13.0, "displays"),
            cx,
        )
    }

    pub(super) fn sound_module(
        &self,
        y: f32,
        sound: &Tile<SoundValue>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let volume = self.volume_preview.unwrap_or(sound.value.volume);
        // A muted output keeps its level; the fill dims like an unavailable
        // transport glyph until it is unmuted.
        let fill = if sound.value.muted {
            dim_glyph()
        } else {
            mac::white()
        };
        self.slider_module(
            SliderKind::Volume,
            y,
            "Sound",
            volume,
            fill,
            sound.available || !self.received_snapshot,
            ("cc/speaker.svg", 20.75, 8.5, 12.5),
            ("cc/speaker-wave.svg", 230.5, 20.0, 14.5),
            ("cc/airplay-audio.svg", 12.5, 12.5, "sound"),
            cx,
        )
    }

    /// One error banner above the modules; clicking it dismisses it.
    pub(super) fn banner(
        &self,
        index: usize,
        y: f32,
        control: Option<Control>,
        message: SharedString,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .id(("control-center-banner", index))
            .role(Role::Button)
            .aria_label(format!("Dismiss: {message}"))
            .focusable()
            .tab_stop(true)
            .on_a11y_action(AccessibleAction::Click, {
                let view = cx.entity().downgrade();
                move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| this.dismiss_error(control, cx));
                }
            })
            .absolute()
            .left_0()
            .top(px(y))
            .w(px(GRID_WIDTH))
            .h(px(layout::BANNER_HEIGHT as f32))
            .px(px(14.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(12.0))
            .bg(mac::error_background())
            .border_1()
            .border_color(mac::error_border())
            .text_size(rmac_ui::text_px(11.0))
            .text_color(mac::danger())
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.dismiss_error(control, cx)),
            )
            .child(div().flex_1().min_w_0().truncate().child(message))
            .child(
                div()
                    .flex_none()
                    .text_color(title_text())
                    .child(rmac_quick_settings::accessibility::DISMISS_LABEL),
            )
            .into_any_element()
    }
}
