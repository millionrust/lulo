//! Control Center detail views: nearby and saved Wi-Fi networks, paired
//! Bluetooth devices, the display's brightness and appearance, and sound
//! outputs, drawn in place of the grid at the measurements in
//! `design-lab/control-center.html`.

use gpui::{ClickEvent, MouseDownEvent};
use rmac_quick_settings::detail::{
    geometry as g, Detail, DisplayToggle, Panel, Part, Row, RowGlyph, Target, Toggle,
};

use super::*;

const LOCK: (&str, f32, f32) = ("cc/lock.svg", 9.0, 12.0);
/// Knob of the title switch, inset 2 inside the 54 × 24 capsule.
const KNOB_WIDTH: f32 = 32.0;
const KNOB_HEIGHT: f32 = 20.0;
const KNOB_INSET: f32 = 2.0;

/// Separators read #48494D over the #383A3F panel: white ≈ 8 %.
fn separator() -> Hsla {
    scheme(0xffff_ff14, 0x0000_0014)
}
/// Section headings, white ≈ 80 %.
fn heading_text() -> Hsla {
    scheme(0xffff_ffcc, 0x0000_00b3)
}
/// Row names, white ≈ 92 %.
fn row_text() -> Hsla {
    scheme(0xffff_ffeb, 0x0000_00d9)
}
/// An "on" round toggle is white with a near-black glyph, in both
/// appearances (Display ▸ Dark Mode, measured).
fn toggle_on_glyph() -> Hsla {
    color(0x1d1d_1fff)
}
/// Pointer-over row fill (S).
fn row_hover() -> Hsla {
    scheme(0xffff_ff1a, 0x0000_000f)
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
        let enabled = action.is_some();
        let ring = self.target_ring(target);
        let view = cx.entity().downgrade();
        div()
            .id(("control-center-detail-row", index))
            .role(Role::Button)
            .aria_label(row.label.clone())
            .aria_selected(row.on)
            .when(enabled, |row| row.focusable().tab_stop(true))
            .absolute()
            .left(px(g::INSET))
            .top(px(top))
            .w(px(g::ROW_WIDTH))
            .h(px(g::ROW_HEIGHT))
            .rounded(px(8.0))
            .when(ring, |row| row.shadow(mac::focus_ring_shadow()))
            .when_some(action, |element, action| {
                let a11y_action = action.clone();
                element
                    .hover(|style| style.bg(row_hover()))
                    .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                        let action = a11y_action.clone();
                        let _ = view.update(cx, |this, cx| this.run_row(action, window, cx));
                    })
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
                        if row.on { glyph_on() } else { ink() },
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
        let view = cx.entity().downgrade();
        let expanded = target == Target::Disclosure && self.others_expanded;
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .when(target == Target::Disclosure, |item| {
                item.aria_expanded(expanded)
            })
            .focusable()
            .tab_stop(true)
            .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.detail_focus = Some(target);
                    this.activate_detail_target(target, window, cx);
                });
            })
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
        let view = cx.entity().downgrade();
        let switch_name = match self.detail {
            Some(rmac_quick_settings::detail::Detail::Wifi) => "Wi-Fi",
            _ => "Bluetooth",
        };
        let state = self.state.view();
        let enabled = match self.detail {
            Some(rmac_quick_settings::detail::Detail::Wifi) => {
                state.wifi.available && !state.wifi.busy
            }
            Some(rmac_quick_settings::detail::Detail::Bluetooth) => {
                state.bluetooth.available && !state.bluetooth.busy
            }
            _ => false,
        };
        div()
            .id("control-center-detail-switch")
            .role(Role::Switch)
            .aria_label(switch_name)
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .when(enabled, |element| {
                element.focusable().tab_stop(true).on_a11y_action(
                    AccessibleAction::Click,
                    move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.toggle_detail_switch(cx));
                    },
                )
            })
            .absolute()
            .left(px(g::PANEL_WIDTH - g::INSET - g::SWITCH_WIDTH))
            .top(px(g::SWITCH_TOP))
            .w(px(g::SWITCH_WIDTH))
            .h(px(g::SWITCH_HEIGHT))
            .rounded_full()
            .bg(if on { mac::system_blue() } else { circle_off() })
            .when(ring, |switch| switch.shadow(mac::focus_ring_shadow()))
            .when(enabled, |element| {
                element.on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_detail_switch(cx)),
                )
            })
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

    /// The slider at the top of the Display or Sound view. Its knob always
    /// shows here, as on the Mac.
    fn detail_slider(&self, display: bool, cx: &Context<Self>) -> AnyElement {
        let (kind, value, fill, enabled, title) = if display {
            (
                SliderKind::DetailBrightness,
                self.brightness.unwrap_or(0),
                mac::white(),
                self.brightness.is_some(),
                "Display",
            )
        } else {
            let sound = self.state.view().sound;
            let fill = if sound.value.muted {
                dim_glyph()
            } else {
                mac::white()
            };
            (
                SliderKind::DetailVolume,
                self.volume_preview.unwrap_or(sound.value.volume),
                fill,
                sound.available,
                "Sound",
            )
        };
        let centre = g::SLIDER_TOP + g::SLIDER_HEIGHT / 2.0;
        let ring = self.target_ring(Target::Slider);
        let bulge = self.slider_bulge(kind);
        let increment_view = cx.entity().downgrade();
        let decrement_view = cx.entity().downgrade();
        let end_x = g::SLIDER_LEFT + g::SLIDER_WIDTH + 16.0;
        let (start, end) = if display {
            (
                glyph_at("cc/sun-min.svg", 20.0, centre, 14.5, 14.5, ink()),
                glyph_at("cc/sun-max.svg", end_x, centre, 16.0, 16.0, ink()),
            )
        } else {
            (
                glyph_at("cc/speaker.svg", 20.0, centre, 8.5, 12.5, ink()),
                glyph_at("cc/speaker-wave.svg", end_x, centre, 20.0, 14.5, ink()),
            )
        };
        layer()
            .child(start)
            .child(end)
            .child(
                div()
                    .id("control-center-detail-slider")
                    .role(Role::Slider)
                    .aria_label(title)
                    .aria_numeric_value(f64::from(value))
                    .aria_numeric_value_step(5.0)
                    .aria_min_numeric_value(0.0)
                    .aria_max_numeric_value(100.0)
                    .when(enabled, |hit| {
                        let set_value_view = cx.entity().downgrade();
                        hit.focusable()
                            .tab_stop(true)
                            .on_a11y_action(AccessibleAction::Increment, move |_, _, cx| {
                                let _ = increment_view.update(cx, |this, cx| {
                                    this.slide(kind, value.saturating_add(5).min(100), cx)
                                });
                            })
                            .on_a11y_action(AccessibleAction::Decrement, move |_, _, cx| {
                                let _ = decrement_view.update(cx, |this, cx| {
                                    this.slide(kind, value.saturating_sub(5), cx)
                                });
                            })
                            .on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
                                if let Some(accesskit::ActionData::NumericValue(requested)) = data {
                                    let value = requested.round().clamp(0.0, 100.0) as u8;
                                    let _ = set_value_view
                                        .update(cx, |this, cx| this.slide(kind, value, cx));
                                }
                            })
                    })
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
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.dragging = Some(kind);
                                this.sync_slider_bulge(kind, cx);
                                let value = super::slider_value(kind, f32::from(event.position.x));
                                this.slide(kind, value, cx);
                            }),
                        )
                        .on_hover(cx.listener(
                            move |this, hovered: &bool, _, cx| {
                                this.set_slider_hovered(kind, *hovered, cx);
                            },
                        ))
                    })
                    .child(super::controls::slider_bar(
                        g::SLIDER_WIDTH,
                        value,
                        fill,
                        bulge,
                        1.0,
                    )),
            )
            .when(!enabled, |slider| slider.opacity(0.5))
            .into_any_element()
    }

    /// Display's round toggles (Dark Mode), centred on the Mac's columns.
    fn detail_toggles(&self, toggles: &[Toggle], cx: &Context<Self>) -> Vec<AnyElement> {
        let count = toggles.len() as f32;
        toggles
            .iter()
            .enumerate()
            .map(|(index, toggle)| {
                let centre_x =
                    g::PANEL_WIDTH / 2.0 + (index as f32 - (count - 1.0) / 2.0) * g::TOGGLE_PITCH;
                let target = Target::Toggle(index);
                let ring = self.target_ring(target);
                let label = toggle.kind.label();
                let state = if toggle.on { "On" } else { "Off" };
                let view = cx.entity().downgrade();
                let glyph = match toggle.kind {
                    DisplayToggle::DarkMode => "cc/dark-mode.svg",
                };
                let left = centre_x - g::TOGGLE_WIDTH / 2.0;
                let circle_top = g::TOGGLE_CENTRE - g::TOGGLE_CIRCLE / 2.0;
                div()
                    .id(("control-center-detail-toggle", index))
                    .role(Role::Switch)
                    .aria_label(label)
                    .aria_toggled(if toggle.on {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .focusable()
                    .tab_stop(true)
                    .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| {
                            this.detail_focus = Some(target);
                            this.activate_detail_target(target, window, cx);
                        });
                    })
                    .absolute()
                    .left(px(left))
                    .top(px(circle_top))
                    .w(px(g::TOGGLE_WIDTH))
                    .h(px(g::TOGGLE_STATE_CENTRE + 9.0 - circle_top))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.detail_focus = Some(target);
                        this.activate_detail_target(target, window, cx)
                    }))
                    .child(
                        div()
                            .absolute()
                            .left(px((g::TOGGLE_WIDTH - g::TOGGLE_CIRCLE) / 2.0))
                            .top_0()
                            .size(px(g::TOGGLE_CIRCLE))
                            .rounded_full()
                            .bg(if toggle.on {
                                mac::white()
                            } else {
                                circle_off()
                            })
                            .when(ring, |circle| circle.shadow(mac::focus_ring_shadow()))
                            .child(glyph_at(
                                glyph,
                                g::TOGGLE_CIRCLE / 2.0,
                                g::TOGGLE_CIRCLE / 2.0,
                                16.0,
                                16.0,
                                if toggle.on { toggle_on_glyph() } else { ink() },
                            )),
                    )
                    .child(
                        line(
                            0.0,
                            g::TOGGLE_NAME_CENTRE - circle_top - 8.0,
                            16.0,
                            13.0,
                            mac::BOLD,
                            title_text(),
                        )
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(label),
                    )
                    .child(
                        line(
                            0.0,
                            g::TOGGLE_STATE_CENTRE - circle_top - 8.0,
                            16.0,
                            13.0,
                            mac::REGULAR,
                            subtitle_text(),
                        )
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(state),
                    )
                    .into_any_element()
            })
            .collect()
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
            children.push(self.detail_slider(panel.detail == Detail::Display, cx));
        }
        children.extend(self.detail_toggles(&panel.toggles, cx));
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
                            // Announced, as VoiceOver reads "Output"
                            // before the Mac's list of outputs.
                            .id(("control-center-detail-heading", index))
                            .role(Role::Heading)
                            .aria_label(heading)
                            .aria_level(2)
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
                            .id("control-center-detail-empty")
                            .role(Role::Label)
                            // AccessKit names a label from its value.
                            .aria_value(empty)
                            .child(empty)
                            .into_any_element(),
                        );
                    }
                }
                _ => {}
            }
        }
        div()
            .id("control-center-detail")
            .role(Role::Group)
            .aria_label(panel.detail.title())
            .absolute()
            .left(px(g::PANEL_LEFT))
            .top(px(top))
            .w(px(g::PANEL_WIDTH))
            .h(px(height))
            .children(children)
            .into_any_element()
    }
}
