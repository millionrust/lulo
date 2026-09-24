//! Control Center detail views: nearby and saved Wi-Fi networks, paired
//! Bluetooth devices, and sound outputs, drawn in place of the grid at the
//! measurements in `design-lab/control-center.html`.

use gpui::{ClickEvent, MouseDownEvent};
use rmac_quick_settings::detail::{geometry as g, Panel, Part, Row, RowGlyph, Target};

use super::*;

const LOCK: (&str, f32, f32) = ("cc/lock.svg", 9.0, 12.0);
/// Knob of the title switch, inset 2 inside the 54 × 24 capsule.
const KNOB_WIDTH: f32 = 32.0;
const KNOB_HEIGHT: f32 = 20.0;
const KNOB_INSET: f32 = 2.0;
/// Detail slider track: 4 thick, centred in its 14 pt row.
const TRACK_HEIGHT: f32 = 4.0;

/// Separators read #48494D over the #383A3F panel: white ≈ 8 %.
fn separator() -> Hsla {
    color(0xffff_ff14)
}
/// Section headings, white ≈ 80 %.
fn heading_text() -> Hsla {
    color(0xffff_ffcc)
}
/// Row names, white ≈ 92 %.
fn row_text() -> Hsla {
    color(0xffff_ffeb)
}
/// Pointer-over row fill (S).
fn row_hover() -> Hsla {
    color(0xffff_ff1a)
}

/// One line of text in a box `height` tall, vertically centred.
fn line(x: f32, top: f32, height: f32, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(top))
        .h(px(height))
        .line_height(px(height))
        .text_size(rmac_ui::text_px(size))
        .font_weight(weight)
        .text_color(color)
        .whitespace_nowrap()
}

fn row_glyph(glyph: RowGlyph) -> (&'static str, f32, f32) {
    match glyph {
        RowGlyph::Wifi => ("cc/wifi.svg", 14.0, 10.0),
        RowGlyph::Bluetooth => ("cc/bluetooth.svg", 8.0, 12.5),
        RowGlyph::Speaker => ("cc/speaker.svg", 7.0, 10.0),
    }
}

impl QuickSettingsView {
    fn target_ring(&self, target: Target) -> bool {
        self.keyboard && self.detail_focus == Some(target)
    }

    fn detail_row(
        &self,
        index: usize,
        top: f32,
        target: Target,
        row: &Row,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (path, width, height) = row_glyph(row.glyph);
        let action = row.action.clone();
        let ring = self.target_ring(target);
        div()
            .id(("control-center-detail-row", index))
            .absolute()
            .left(px(g::INSET))
            .top(px(top))
            .w(px(g::ROW_WIDTH))
            .h(px(g::ROW_HEIGHT))
            .rounded(px(8.0))
            .when(ring, |row| row.shadow(mac::focus_ring_shadow()))
            .when_some(action, |element, action| {
                element
                    .hover(|style| style.bg(row_hover()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_row(action.clone(), window, cx)
                    }))
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((g::ROW_HEIGHT - g::CIRCLE) / 2.0))
                    .size(px(g::CIRCLE))
                    .rounded_full()
                    .bg(if row.on { mac::white() } else { circle_off() })
                    .child(glyph_at(
                        path,
                        g::CIRCLE / 2.0,
                        g::CIRCLE / 2.0,
                        width,
                        height,
                        if row.on { glyph_on() } else { mac::white() },
                    )),
            )
            .child(
                line(
                    g::NAME_LEFT,
                    0.0,
                    g::ROW_HEIGHT,
                    13.0,
                    mac::REGULAR,
                    row_text(),
                )
                .w(px(g::ROW_WIDTH - g::NAME_LEFT - 16.0))
                .truncate()
                .child(row.label.clone()),
            )
            .when(row.locked, |element| {
                element.child(glyph_at(
                    LOCK.0,
                    g::ROW_WIDTH - 2.0 - LOCK.1 / 2.0,
                    g::ROW_HEIGHT / 2.0,
                    LOCK.1,
                    LOCK.2,
                    subtitle_text(),
                ))
            })
            .into_any_element()
    }

    /// A 22 pt text row: the notice, the disclosure and the settings row.
    fn detail_item(
        &self,
        id: &'static str,
        top: f32,
        label: String,
        target: Target,
        cx: &Context<Self>,
    ) -> AnyElement {
        let ring = self.target_ring(target);
        div()
            .id(id)
            .absolute()
            .left(px(g::INSET))
            .top(px(top))
            .w(px(g::ROW_WIDTH))
            .h(px(g::ITEM_HEIGHT))
            .rounded(px(6.0))
            .hover(|style| style.bg(row_hover()))
            .when(ring, |item| item.shadow(mac::focus_ring_shadow()))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.detail_focus = Some(target);
                this.activate_detail_target(target, window, cx)
            }))
            .child(line(0.0, 0.0, g::ITEM_HEIGHT, 13.0, mac::REGULAR, row_text()).child(label))
            .into_any_element()
    }

    fn detail_switch(&self, on: bool, cx: &Context<Self>) -> AnyElement {
        let ring = self.target_ring(Target::Switch);
        let knob_left = if on {
            g::SWITCH_WIDTH - KNOB_INSET - KNOB_WIDTH
        } else {
            KNOB_INSET
        };
        div()
            .id("control-center-detail-switch")
            .absolute()
            .left(px(g::PANEL_WIDTH - g::INSET - g::SWITCH_WIDTH))
            .top(px(g::SWITCH_TOP))
            .w(px(g::SWITCH_WIDTH))
            .h(px(g::SWITCH_HEIGHT))
            .rounded_full()
            .bg(if on { mac::system_blue() } else { circle_off() })
            .when(ring, |switch| switch.shadow(mac::focus_ring_shadow()))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_detail_switch(cx)))
            .child(
                div()
                    .absolute()
                    .left(px(knob_left))
                    .top(px(KNOB_INSET))
                    .w(px(KNOB_WIDTH))
                    .h(px(KNOB_HEIGHT))
                    .rounded_full()
                    .bg(mac::white()),
            )
            .into_any_element()
    }

    fn detail_slider(&self, cx: &Context<Self>) -> AnyElement {
        let sound = self.state.view().sound;
        let volume = self.volume_preview.unwrap_or(sound.value.volume);
        let enabled = sound.available;
        let fill = if sound.value.muted {
            dim_glyph()
        } else {
            mac::white()
        };
        let centre = g::SLIDER_TOP + g::SLIDER_HEIGHT / 2.0;
        let filled = g::SLIDER_WIDTH * f32::from(volume.min(100)) / 100.0;
        let ring = self.target_ring(Target::Slider);
        layer()
            .child(glyph_at(
                "cc/speaker.svg",
                20.0,
                centre,
                8.5,
                12.5,
                mac::white(),
            ))
            .child(glyph_at(
                "cc/speaker-wave.svg",
                g::SLIDER_LEFT + g::SLIDER_WIDTH + 16.0,
                centre,
                20.0,
                14.5,
                mac::white(),
            ))
            .child(
                div()
                    .id("control-center-detail-slider")
                    .absolute()
                    .left(px(g::SLIDER_LEFT - 8.0))
                    .top(px(centre - 12.0))
                    .w(px(g::SLIDER_WIDTH + 16.0))
                    .h(px(24.0))
                    .rounded_full()
                    .when(ring, |hit| hit.shadow(mac::focus_ring_shadow()))
                    .when(enabled, |hit| {
                        hit.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                this.dragging = Some(SliderKind::DetailVolume);
                                let value = super::slider_value(
                                    SliderKind::DetailVolume,
                                    f32::from(event.position.x),
                                );
                                this.slide(SliderKind::DetailVolume, value, cx);
                            }),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .left(px(8.0))
                            .top(px(12.0 - TRACK_HEIGHT / 2.0))
                            .w(px(g::SLIDER_WIDTH))
                            .h(px(TRACK_HEIGHT))
                            .rounded(px(TRACK_HEIGHT / 2.0))
                            .overflow_hidden()
                            .bg(slider_track())
                            .child(div().h_full().w(px(filled)).bg(fill)),
                    ),
            )
            .when(!enabled, |slider| slider.opacity(0.5))
            .into_any_element()
    }

    /// The whole detail view, `top` below the surface edge.
    pub(super) fn detail_view(&self, panel: &Panel, top: f32, cx: &Context<Self>) -> AnyElement {
        let (placed, height) = panel.layout();
        let mut children: Vec<AnyElement> = vec![line(
            g::INSET,
            g::TITLE_TOP,
            g::TITLE_HEIGHT,
            13.0,
            mac::BOLD,
            title_text(),
        )
        .child(panel.detail.title())
        .into_any_element()];
        if let Some(on) = panel.switch {
            children.push(self.detail_switch(on, cx));
        }
        if let Some(notice) = panel.notice {
            children.push(self.detail_item(
                "control-center-detail-notice",
                g::NOTICE_TOP,
                notice.to_owned(),
                Target::Notice,
                cx,
            ));
        }
        if panel.slider {
            children.push(self.detail_slider(cx));
        }
        let mut headings = panel.sections.iter().map(|section| section.heading);
        for (index, part) in placed.iter().enumerate() {
            match (part.kind, part.target) {
                (Part::Separator, _) => children.push(
                    div()
                        .absolute()
                        .left(px(g::INSET))
                        .top(px(part.top))
                        .w(px(g::ROW_WIDTH))
                        .h(px(1.0))
                        .bg(separator())
                        .into_any_element(),
                ),
                (Part::Heading, _) => {
                    if let Some(heading) = headings.next() {
                        children.push(
                            line(
                                g::INSET,
                                part.top,
                                g::HEADING_HEIGHT,
                                11.0,
                                mac::SEMIBOLD,
                                heading_text(),
                            )
                            .child(heading)
                            .into_any_element(),
                        );
                    }
                }
                (Part::Row, Some(target)) => {
                    if let Some(row) = panel.row(target) {
                        children.push(self.detail_row(index, part.top, target, row, cx));
                    }
                }
                (Part::Item, Some(Target::Disclosure)) => {
                    let expanded = panel.disclosure == Some(true);
                    children.push(self.detail_item(
                        "control-center-detail-others",
                        part.top,
                        format!(
                            "Other Networks {}",
                            if expanded { "\u{2304}" } else { "\u{203a}" }
                        ),
                        Target::Disclosure,
                        cx,
                    ));
                }
                (Part::Item, Some(Target::Settings)) => {
                    children.push(self.detail_item(
                        "control-center-detail-settings",
                        part.top,
                        panel.detail.settings().0.to_owned(),
                        Target::Settings,
                        cx,
                    ));
                }
                (Part::Empty, _) => {
                    if let Some(empty) = panel.empty {
                        children.push(
                            line(
                                g::INSET,
                                part.top,
                                g::ITEM_HEIGHT,
                                13.0,
                                mac::REGULAR,
                                subtitle_text(),
                            )
                            .child(empty)
                            .into_any_element(),
                        );
                    }
                }
                _ => {}
            }
        }
        div()
            .absolute()
            .left(px(g::PANEL_LEFT))
            .top(px(top))
            .w(px(g::PANEL_WIDTH))
            .h(px(height))
            .children(children)
            .into_any_element()
    }
}
