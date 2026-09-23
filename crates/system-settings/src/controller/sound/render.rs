//! Sound settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let out = self.output_volume.read(cx).value().start().round() as i32;
        let input = self.input_volume.read(cx).value().start().round() as i32;
        let alert = self.alert_volume.read(cx).value().start().round() as i32;
        let refresh_view = view.clone();
        let alert_sound = self.sound_policy.alert_sound;
        let preview_view = view.clone();
        let alert_picker = PopUpButton::new("sound-alert-picker", alert_sound.display_name())
            .form()
            .dropdown_menu(|menu, _, _| {
                menu.menu("Alert", Box::new(SelectAlert))
                    .menu("Error", Box::new(SelectErrorAlert))
                    .menu("Notification", Box::new(SelectNotificationAlert))
            });
        let output_name: SharedString = self
            .audio
            .outputs
            .iter()
            .find(|device| device.is_default)
            .map(|device| SharedString::from(device.name.clone()))
            .unwrap_or_else(|| "Selected Sound Output Device".into());
        let interface_view = view.clone();
        let interface_effects = Toggle::new("sound-interface-effects")
            .checked(self.sound_policy.interface_effects)
            .on_click(move |enabled, _, cx| {
                interface_view.update(cx, |settings, cx| {
                    settings.apply_sound_policy_change(
                        SoundPolicyChange::InterfaceEffects(*enabled),
                        cx,
                    )
                });
            });
        let feedback_view = view.clone();
        let volume_feedback = Toggle::new("sound-volume-feedback")
            .checked(self.sound_policy.volume_feedback)
            .on_click(move |enabled, _, cx| {
                feedback_view.update(cx, |settings, cx| {
                    settings
                        .apply_sound_policy_change(SoundPolicyChange::VolumeFeedback(*enabled), cx)
                });
            });
        let login_view = view.clone();
        let login_sound = Toggle::new("sound-login")
            .checked(self.sound_policy.login_sound)
            .on_click(move |enabled, _, cx| {
                login_view.update(cx, |settings, cx| {
                    settings.apply_sound_policy_change(SoundPolicyChange::LoginSound(*enabled), cx)
                });
            });

        let mut cards = vec![first_section_header("Sound Effects")];
        cards.push(
            div()
                .v_flex()
                .mb(px(style::GROUP_GAP))
                .rounded(px(style::GROUP_RADIUS))
                .bg(card_bg())
                .child(
                    row_base()
                        .child(text_block("Alert sound".into(), None))
                        .child(alert_picker)
                        // The Mac's circled ▶ plays the chosen alert.
                        .child(
                            div()
                                .id("sound-alert-preview")
                                .size(px(style::INFO_BUTTON + 2.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(label())
                                .cursor_pointer()
                                .tooltip(|window, cx| {
                                    gpui_component::tooltip::Tooltip::new("Play alert sound")
                                        .build(window, cx)
                                })
                                .child(glyph("icons/play.svg", 9.0, label()))
                                .on_click(move |_, _, cx| {
                                    preview_view.update(cx, |settings, _| {
                                        let _ = rmac_sound::preview(
                                            alert_sound,
                                            settings.sound_policy.alert_volume,
                                        );
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .h(px(style::SEPARATOR))
                        .bg(sep())
                        .mx(px(style::ROW_PADDING)),
                )
                .child(
                    row_base()
                        .child(text_block("Play sound effects through".into(), None))
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(secondary())
                                .child(output_name),
                        ),
                )
                .child(
                    div()
                        .h(px(style::SEPARATOR))
                        .bg(sep())
                        .mx(px(style::ROW_PADDING)),
                )
                .child(slider_row(
                    "Alert volume",
                    &self.alert_volume,
                    format!("{alert}%").into(),
                ))
                .child(
                    div()
                        .h(px(style::SEPARATOR))
                        .bg(sep())
                        .mx(px(style::ROW_PADDING)),
                )
                .child(
                    row_base()
                        .child(text_block(
                            "Play user interface sound effects".into(),
                            Some("Trash, screenshots, devices, and system actions".into()),
                        ))
                        .child(interface_effects),
                )
                .child(
                    div()
                        .h(px(style::SEPARATOR))
                        .bg(sep())
                        .mx(px(style::ROW_PADDING)),
                )
                .child(
                    row_base()
                        .child(text_block(
                            "Play feedback when volume is changed".into(),
                            None,
                        ))
                        .child(volume_feedback),
                )
                .child(
                    div()
                        .h(px(style::SEPARATOR))
                        .bg(sep())
                        .mx(px(style::ROW_PADDING)),
                )
                .child(
                    row_base()
                        .child(text_block("Play sound on startup".into(), None))
                        .child(login_sound),
                ),
        );
        if let Some(detail) = &self.sound_policy_error {
            cards.push(note_card(detail.clone()));
        }
        cards.push(section_header("Output & Input"));

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
                    .mb(px(style::GROUP_GAP))
                    .rounded(px(style::GROUP_RADIUS))
                    .bg(card_bg())
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
                        .child(
                            div()
                                .h(px(style::SEPARATOR))
                                .bg(sep())
                                .mx(px(style::ROW_PADDING)),
                        )
                        .child(balance_slider_row(&self.output_balance));
                }
                output_card = output_card
                    .child(
                        div()
                            .h(px(style::SEPARATOR))
                            .bg(sep())
                            .mx(px(style::ROW_PADDING)),
                    )
                    .child(
                        row_base()
                            .child(tile("icons/volume-2.svg", secondary(), style::ROW_ICON))
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
                    .mb(px(style::GROUP_GAP))
                    .rounded(px(style::GROUP_RADIUS))
                    .bg(card_bg())
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
                    input_card = input_card
                        .child(
                            div()
                                .h(px(style::SEPARATOR))
                                .bg(sep())
                                .mx(px(style::ROW_PADDING)),
                        )
                        .child(
                            row_base()
                                .child(tile("icons/volume-2.svg", secondary(), style::ROW_ICON))
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

        cards.push(footer_buttons(vec![push_button(
            "audio-refresh",
            "Refresh",
        )
        .busy(self.audio_busy)
        .disabled(self.audio_loading || self.audio_busy)
        .on_click(move |_, _, cx| {
            refresh_view.update(cx, |settings, cx| settings.refresh_audio(cx));
        })
        .into_any_element()]));
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
