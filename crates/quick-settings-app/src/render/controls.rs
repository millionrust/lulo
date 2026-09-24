//! Control Center Display and Sound slider modules and error banners.

use gpui::{ClickEvent, MouseDownEvent};
use rmac_quick_settings::detail::{Detail, Module};
use rmac_quick_settings::{Control, SoundValue, Tile};

use super::*;

/// The accessory circle at the right end of a slider module.
const ACCESSORY: f32 = 26.0;
const ACCESSORY_CENTRE: (f32, f32) = (265.0, 42.0);
/// Slider tracks are 4 thick and centred 42 below the module top.
const TRACK_HEIGHT: f32 = 4.0;
const TRACK_CENTRE: f32 = 42.0;

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
        let filled = width * f32::from(value.min(100)) / 100.0;
        let id = match kind {
            SliderKind::Brightness => "display-slider",
            SliderKind::Volume | SliderKind::DetailVolume => "sound-slider",
        };
        let module_kind = if kind == SliderKind::Brightness {
            Module::Display
        } else {
            Module::Sound
        };
        let ring = self.ring(module_kind);
        let hit = div()
            .id(id)
            .absolute()
            .left(px(left - 8.0))
            .top(px(TRACK_CENTRE - 12.0))
            .w(px(width + 16.0))
            .h(px(24.0))
            .when(enabled, |hit| {
                hit.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.dragging = Some(kind);
                        let value = super::slider_value(kind, f32::from(event.position.x));
                        this.slide(kind, value, cx);
                    }),
                )
            })
            .child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .top(px(12.0 - TRACK_HEIGHT / 2.0))
                    .w(px(width))
                    .h(px(TRACK_HEIGHT))
                    .rounded(px(TRACK_HEIGHT / 2.0))
                    .overflow_hidden()
                    .bg(slider_track())
                    .child(div().h_full().w(px(filled)).bg(fill)),
            );
        let accessory = div()
            .id(pane)
            .absolute()
            .left(px(ACCESSORY_CENTRE.0 - ACCESSORY / 2.0))
            .top(px(ACCESSORY_CENTRE.1 - ACCESSORY / 2.0))
            .size(px(ACCESSORY))
            .rounded_full()
            .bg(circle_off())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                // Sound's output button lists the outputs, as on the Mac.
                if kind == SliderKind::Brightness {
                    this.open_settings(Some(pane), window, cx)
                } else {
                    this.open_detail(Detail::Sound, cx)
                }
            }))
            .child(glyph_at(
                accessory_path,
                ACCESSORY / 2.0,
                ACCESSORY / 2.0,
                accessory_width,
                accessory_height,
                mac::white(),
            ));
        let glyph = |(path, centre, width, height): (&'static str, f32, f32, f32)| {
            glyph_at(path, centre, TRACK_CENTRE, width, height, mac::white())
        };
        module(0.0, y, GRID_WIDTH, CELL, layout::MODULE_RADIUS as f32)
            .when(ring, |module| module.shadow(mac::focus_ring_shadow()))
            .when(!enabled, |module| module.opacity(0.5))
            .child(text_at(16.5, 23.0, 13.0, mac::BOLD, title_text(), title))
            .child(glyph(start))
            .child(glyph(end))
            .child(hit)
            .child(accessory)
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
