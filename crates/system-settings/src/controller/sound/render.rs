//! Sound settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let out = self.output_volume.read(cx).value().start().round() as i32;
        let input = self.input_volume.read(cx).value().start().round() as i32;
        let refresh_view = view.clone();
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child("System Audio"),
            )
            .child(
                Button::new("audio-refresh", "Refresh")
                    .ghost()
                    .busy(self.audio_busy)
                    .disabled(self.audio_loading || self.audio_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_audio(cx));
                    }),
            )];

        if self.audio_loading {
            cards.push(note_card("Loading audio state from the system…"));
        } else if !self.audio.available {
            cards.push(note_card(
                "The system audio service is not available on this computer.",
            ));
        } else {
            if let Some(detail) = &self.audio.configuration_error {
                cards.push(note_card(detail.clone()));
            }
            if self.audio.has_output {
                let output_view = view.clone();
                let output_mute = Toggle::new("audio-output-mute")
                    .checked(self.audio.output.muted)
                    .on_click(move |muted, _, cx| {
                        output_view.update(cx, |settings, cx| {
                            settings.set_audio_muted(rmac_audio::DeviceKind::Output, *muted, cx)
                        });
                    });
                let mut output_card = div()
                    .v_flex()
                    .mb_3()
                    .rounded(px(rmac_ui::mac::radius_menu()))
                    .bg(card_bg())
                    .border_1()
                    .border_color(sep())
                    .child(slider_row(
                        "Output volume",
                        &self.output_volume,
                        format!("{out}%").into(),
                    ));
                if self
                    .audio
                    .outputs
                    .iter()
                    .any(|device| device.is_default && device.balance.is_some())
                {
                    output_card = output_card
                        .child(div().h(px(1.0)).bg(sep()).mx_3())
                        .child(balance_slider_row(&self.output_balance));
                }
                output_card = output_card.child(div().h(px(1.0)).bg(sep()).mx_3()).child(
                    row_base()
                        .child(tile("icons/volume-2.svg", secondary(), 22.0))
                        .child(text_block("Mute output".into(), None))
                        .child(output_mute),
                );
                cards.push(output_card);
            } else {
                cards.push(note_card("No output device is currently active."));
            }

            if self.audio.has_input {
                let mut input_card = div()
                    .v_flex()
                    .mb_3()
                    .rounded(px(rmac_ui::mac::radius_menu()))
                    .bg(card_bg())
                    .border_1()
                    .border_color(sep())
                    .child(slider_row(
                        "Input volume",
                        &self.input_volume,
                        format!("{input}%").into(),
                    ));
                if self.audio.can_mute_input {
                    let input_view = view.clone();
                    let input_mute = Toggle::new("audio-input-mute")
                        .checked(self.audio.input.muted)
                        .on_click(move |muted, _, cx| {
                            input_view.update(cx, |settings, cx| {
                                settings.set_audio_muted(rmac_audio::DeviceKind::Input, *muted, cx)
                            });
                        });
                    input_card = input_card.child(div().h(px(1.0)).bg(sep()).mx_3()).child(
                        row_base()
                            .child(tile("icons/volume-2.svg", secondary(), 22.0))
                            .child(text_block("Mute microphone".into(), None))
                            .child(input_mute),
                    );
                }
                cards.push(input_card);
            } else {
                cards.push(note_card("No input device is currently active."));
            }
            cards.push(self.audio_device_card(
                "Output Device",
                &self.audio.outputs,
                rmac_audio::DeviceKind::Output,
                cx,
            ));
            cards.push(self.audio_device_card(
                "Input Device",
                &self.audio.inputs,
                rmac_audio::DeviceKind::Input,
                cx,
            ));
            cards.push(self.audio_route_card(
                "Output Ports",
                &self.audio.outputs,
                rmac_audio::DeviceKind::Output,
                cx,
            ));
            cards.push(self.audio_route_card(
                "Input Ports",
                &self.audio.inputs,
                rmac_audio::DeviceKind::Input,
                cx,
            ));
            cards.push(self.audio_profile_card(cx));
        }

        cards.push(note_card(
            "Output, microphone, defaults, ports, and device profiles use the system audio service. Session alert sounds and interface effects stay hidden until the rmac sound policy service exists.",
        ));
        self.pane(cards)
    }

    pub(in crate::controller) fn audio_device_card(
        &self,
        title: &'static str,
        devices: &[rmac_audio::Device],
        kind: rmac_audio::DeviceKind,
        cx: &Context<Self>,
    ) -> Div {
        if devices.is_empty() {
            return div();
        }
        let view = cx.entity();
        let rows: Vec<AnyElement> = devices
            .iter()
            .map(|d| {
                let id = d.id.clone();
                let device = d.clone();
                let device_view = view.clone();
                audio_choice_row(
                    SharedString::from(format!("audio-device-{title}-{id}")),
                    "icons/volume-2.svg",
                    d.name.clone().into(),
                    None,
                    d.is_default.then_some("Default"),
                    d.is_default,
                    self.audio_busy || !self.audio.can_set_default,
                )
                .on_activate(move |_, _, cx| {
                    if !device.is_default {
                        let device = device.clone();
                        device_view.update(cx, |settings, cx| {
                            settings.set_default_audio_device(kind, device, cx)
                        });
                    }
                })
                .into_any_element()
            })
            .collect();
        div()
            .v_flex()
            .child(section_header(title))
            .child(card(rows))
    }

    pub(in crate::controller) fn audio_route_card(
        &self,
        title: &'static str,
        devices: &[rmac_audio::Device],
        kind: rmac_audio::DeviceKind,
        cx: &Context<Self>,
    ) -> Div {
        let view = cx.entity();
        let rows: Vec<AnyElement> = devices
            .iter()
            .flat_map(|device| {
                device.routes.iter().map(|route| {
                    let row_device = device.clone();
                    let row_route = route.clone();
                    let route_view = view.clone();
                    let detail: SharedString = if route.availability.can_select() {
                        device.name.clone().into()
                    } else {
                        format!("{} · Unavailable", device.name).into()
                    };
                    let actionable = audio_choice_is_actionable(
                        route.is_active,
                        route.availability,
                        self.audio_busy,
                    );
                    audio_choice_row(
                        SharedString::from(format!(
                            "audio-route-{title}-{}-{}",
                            device.id, route.index
                        )),
                        "icons/volume-2.svg",
                        route.name.clone().into(),
                        Some(detail),
                        route.is_active.then_some("Active"),
                        route.is_active,
                        self.audio_busy || (!route.is_active && !route.availability.can_select()),
                    )
                    .on_activate(move |_, _, cx| {
                        if actionable {
                            let device = row_device.clone();
                            let route = row_route.clone();
                            route_view.update(cx, |settings, cx| {
                                settings.set_audio_route(kind, device, route, cx)
                            });
                        }
                    })
                    .into_any_element()
                })
            })
            .collect();
        if rows.is_empty() {
            div()
        } else {
            div()
                .v_flex()
                .child(section_header(title))
                .child(card(rows))
        }
    }

    pub(in crate::controller) fn audio_profile_card(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let rows: Vec<AnyElement> = self
            .audio
            .hardware_devices
            .iter()
            .flat_map(|device| {
                device.profiles.iter().map(|profile| {
                    let row_device = device.clone();
                    let row_profile = profile.clone();
                    let profile_view = view.clone();
                    let detail: SharedString = if profile.availability.can_select() {
                        device.name.clone().into()
                    } else {
                        format!("{} · Unavailable", device.name).into()
                    };
                    let actionable = audio_choice_is_actionable(
                        profile.is_active,
                        profile.availability,
                        self.audio_busy,
                    );
                    audio_choice_row(
                        SharedString::from(format!(
                            "audio-profile-{}-{}",
                            device.id, profile.index
                        )),
                        "icons/settings.svg",
                        profile.name.clone().into(),
                        Some(detail),
                        profile.is_active.then_some("Active"),
                        profile.is_active,
                        self.audio_busy
                            || (!profile.is_active && !profile.availability.can_select()),
                    )
                    .on_activate(move |_, _, cx| {
                        if actionable {
                            let device = row_device.clone();
                            let profile = row_profile.clone();
                            profile_view.update(cx, |settings, cx| {
                                settings.set_audio_profile(device, profile, cx)
                            });
                        }
                    })
                    .into_any_element()
                })
            })
            .collect();
        if rows.is_empty() {
            div()
        } else {
            div()
                .v_flex()
                .child(section_header("Device Profiles"))
                .child(card(rows))
        }
    }
}
