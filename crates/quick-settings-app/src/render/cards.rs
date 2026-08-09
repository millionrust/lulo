//! Quick Settings Sound and Power card projection.

use super::*;

impl QuickSettingsView {
    pub(super) fn sound_card(&self, sound: &Tile<SoundValue>, cx: &Context<Self>) -> AnyElement {
        let mute_view = cx.entity();
        let disabled = !sound.available || sound.busy;
        div()
            .w_full()
            .rounded(px(mac::radius_card()))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::raised())
            .px_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::icon_badge(
                        "icons/volume-2.svg",
                        sound.available && !sound.value.muted,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child("Sound"),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(if sound.busy {
                                        CHANGING_LABEL.to_owned()
                                    } else {
                                        sound.summary.clone()
                                    }),
                            ),
                    )
                    .child(
                        Toggle::new("quick-sound-mute")
                            .checked(sound.value.muted)
                            .disabled(disabled)
                            .tooltip("Mute output")
                            .on_click(move |value, _, cx| {
                                mute_view.update(cx, |this, cx| {
                                    this.execute(Command::SetOutputMuted(*value), cx)
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(72.0))
                            .text_size(rmac_ui::text_px(10.5))
                            .text_color(mac::text_secondary())
                            .child("Output volume"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Slider::new(&self.volume).disabled(disabled).w_full()),
                    )
                    .child(
                        div()
                            .w(px(34.0))
                            .text_right()
                            .text_size(rmac_ui::text_px(10.5))
                            .text_color(mac::text_secondary())
                            .child(format!("{}%", sound.value.volume)),
                    ),
            )
            .when_some(
                self.control_error(Control::Sound, sound.error.clone(), cx),
                |card, error| card.child(error),
            )
            .into_any_element()
    }

    pub(super) fn power_card(&self, power: &Tile<PowerValue>, cx: &Context<Self>) -> AnyElement {
        let buttons = power
            .value
            .supported
            .iter()
            .copied()
            .map(|profile| {
                let view = cx.entity();
                Button::new(
                    SharedString::from(format!("quick-power-{}", profile.id())),
                    profile.label(),
                )
                .xsmall()
                .role(if power.value.active == Some(profile) {
                    ButtonRole::Primary
                } else {
                    ButtonRole::Secondary
                })
                .disabled(!power.available || power.busy)
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.execute(Command::SetPowerProfile(profile), cx)
                    });
                })
            })
            .collect::<Vec<_>>();
        div()
            .w_full()
            .rounded(px(mac::radius_card()))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::raised())
            .px_3()
            .py_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Self::icon_badge(
                        "icons/battery-charging.svg",
                        power.available,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .v_flex()
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child("Power Mode"),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(10.5))
                                    .text_color(mac::text_secondary())
                                    .child(if power.busy {
                                        CHANGING_LABEL.to_owned()
                                    } else {
                                        power.summary.clone()
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .when(buttons.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(rmac_ui::text_px(10.5))
                                .text_color(mac::text_secondary())
                                .child("No power profiles are available"),
                        )
                    })
                    .children(buttons),
            )
            .when_some(
                self.control_error(Control::Power, power.error.clone(), cx),
                |card, error| card.child(error),
            )
            .into_any_element()
    }
}
