//! Sound settings lifecycle and presentation.

use super::*;

fn audio_choice_row(
    id: SharedString,
    icon: &'static str,
    title: SharedString,
    detail: Option<SharedString>,
    status: Option<&'static str>,
    selected: bool,
    disabled: bool,
) -> ListRow {
    let has_detail = detail.is_some();
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    let mut text = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(foreground)
            .child(title),
    );
    if let Some(detail) = detail {
        text = text.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary_foreground)
                .child(detail),
        );
    }
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(tile(icon, secondary_foreground, 22.0))
        .child(text)
        .when_some(status, |row, status| {
            row.child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary_foreground)
                    .child(status),
            )
        })
        .when(selected, |row| {
            row.child(glyph("icons/check.svg", 13.0, on_accent()))
        });
    ListRow::new(ElementId::from(id), content)
        .selected(selected)
        .disabled(disabled)
        .h(px(if has_detail { 60.0 } else { 44.0 }))
        .px_3()
}

impl Settings {
    pub(super) fn finish_audio_update(
        &mut self,
        result: std::result::Result<rmac_audio::Snapshot, rmac_audio::Error>,
        cx: &mut Context<Self>,
    ) {
        let refresh_pending = std::mem::take(&mut self.audio_refresh_pending);
        self.audio_loading = false;
        self.audio_busy = false;
        match result {
            Ok(snapshot) => {
                self.replace_audio_snapshot(snapshot, cx);
                self.audio_error = None;
                self.audio_stream_error = None;
            }
            Err(error) => {
                self.audio_error = Some(format!("Could not update Sound: {error}").into());
            }
        }
        if refresh_pending {
            self.refresh_audio(cx);
        }
    }

    pub(super) fn replace_audio_snapshot(
        &mut self,
        snapshot: rmac_audio::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.output_volume_generation = self.output_volume_generation.wrapping_add(1);
        self.input_volume_generation = self.input_volume_generation.wrapping_add(1);
        self.output_balance_generation = self.output_balance_generation.wrapping_add(1);
        self.output_volume = Self::audio_slider(
            cx,
            f32::from(snapshot.output.volume),
            rmac_audio::DeviceKind::Output,
        );
        self.input_volume = Self::audio_slider(
            cx,
            f32::from(snapshot.input.volume),
            rmac_audio::DeviceKind::Input,
        );
        let balance = snapshot
            .outputs
            .iter()
            .find(|device| device.is_default)
            .and_then(|device| device.balance.as_ref())
            .map_or(0.0, |balance| f32::from(balance.value));
        self.output_balance = Self::audio_balance_slider(cx, balance);
        self.audio = snapshot;
    }

    pub(super) fn finish_audio_stream_update(
        &mut self,
        result: std::result::Result<rmac_audio::Snapshot, rmac_audio::Error>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(snapshot) => {
                self.replace_audio_snapshot(snapshot, cx);
                self.audio_stream_error = None;
            }
            Err(_) => {
                self.audio_stream_error =
                    Some("Live audio state could not be refreshed from PipeWire".into());
            }
        }
    }

    pub(super) fn refresh_audio(&mut self, cx: &mut Context<Self>) {
        if self.audio_loading || self.audio_busy {
            return;
        }
        self.audio_generation = self.audio_generation.wrapping_add(1);
        self.audio_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_audio::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn schedule_audio_volume(
        &mut self,
        kind: rmac_audio::DeviceKind,
        volume: f32,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || !self.audio.available {
            return;
        }
        let generation = match kind {
            rmac_audio::DeviceKind::Output => {
                self.output_volume_generation = self.output_volume_generation.wrapping_add(1);
                self.output_volume_generation
            }
            rmac_audio::DeviceKind::Input => {
                self.input_volume_generation = self.input_volume_generation.wrapping_add(1);
                self.input_volume_generation
            }
        };
        let volume = volume.round().clamp(0.0, 100.0) as u8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let current = match kind {
                    rmac_audio::DeviceKind::Output => this.output_volume_generation,
                    rmac_audio::DeviceKind::Input => this.input_volume_generation,
                };
                if current == generation && !this.audio_busy {
                    this.apply_audio_change(AudioChange::Volume(kind, volume), cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn set_audio_muted(
        &mut self,
        kind: rmac_audio::DeviceKind,
        muted: bool,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Muted(kind, muted), cx);
    }

    pub(super) fn set_default_audio_device(
        &mut self,
        kind: rmac_audio::DeviceKind,
        device: rmac_audio::Device,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading
            || self.audio_busy
            || !self.audio.available
            || !self.audio.can_set_default
        {
            return;
        }
        self.apply_audio_change(AudioChange::DefaultDevice(kind, device), cx);
    }

    pub(super) fn set_audio_profile(
        &mut self,
        device: rmac_audio::HardwareDevice,
        profile: rmac_audio::Profile,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Profile(device, profile), cx);
    }

    pub(super) fn set_audio_route(
        &mut self,
        kind: rmac_audio::DeviceKind,
        device: rmac_audio::Device,
        route: rmac_audio::Route,
        cx: &mut Context<Self>,
    ) {
        if self.audio_loading || self.audio_busy || !self.audio.available {
            return;
        }
        self.apply_audio_change(AudioChange::Route(kind, device, route), cx);
    }

    pub(super) fn schedule_audio_balance(&mut self, value: f32, cx: &mut Context<Self>) {
        if self.audio_loading || !self.audio.available || !self.audio.has_output {
            return;
        }
        self.output_balance_generation = self.output_balance_generation.wrapping_add(1);
        let generation = self.output_balance_generation;
        let value = value.round().clamp(-100.0, 100.0) as i8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.output_balance_generation != generation || this.audio_busy {
                    return;
                }
                let Some(device) = this
                    .audio
                    .outputs
                    .iter()
                    .find(|device| device.is_default && device.balance.is_some())
                    .cloned()
                else {
                    return;
                };
                this.apply_audio_change(AudioChange::Balance(device, value), cx);
            });
        })
        .detach();
    }

    pub(super) fn apply_audio_change(&mut self, change: AudioChange, cx: &mut Context<Self>) {
        self.audio_generation = self.audio_generation.wrapping_add(1);
        self.audio_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { change.apply() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_sound(&self, cx: &Context<Self>) -> Div {
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
                    .rounded(px(10.0))
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
                    .rounded(px(10.0))
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

    pub(super) fn audio_device_card(
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

    pub(super) fn audio_route_card(
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

    pub(super) fn audio_profile_card(&self, cx: &Context<Self>) -> Div {
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
