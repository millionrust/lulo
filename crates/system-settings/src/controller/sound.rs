//! Sound settings lifecycle and presentation.

use super::*;

mod render;

#[derive(Clone, Copy)]
pub(super) enum SoundPolicyChange {
    AlertSound(rmac_sound::Cue),
    InterfaceEffects(bool),
    VolumeFeedback(bool),
    LoginSound(bool),
}

fn audio_choice_row(
    id: SharedString,
    icon: &'static str,
    title: SharedString,
    detail: Option<SharedString>,
    status: Option<&'static str>,
    selected: bool,
    disabled: bool,
) -> ListRow {
    // macOS 26 marks the chosen device like a table selection: a grey row,
    // text unchanged, rather than an accent fill.
    let has_detail = detail.is_some();
    // `ListRow` has no name of its own (see its `aria_label` doc comment),
    // so without this a screen reader announced only "selected" or nothing
    // at all when moving through the output/input device and alert-sound
    // lists, with no way to tell which device or sound a row was.
    let mut aria_label = title.to_string();
    if let Some(detail) = &detail {
        aria_label.push_str(", ");
        aria_label.push_str(detail);
    }
    if let Some(status) = status {
        aria_label.push_str(", ");
        aria_label.push_str(status);
    }
    // `selected` is already exposed through `ListRow`'s own
    // `aria_selected(selected)` state, so it is not folded into the name
    // too (that would double-announce it).
    let aria_label = SharedString::from(aria_label);
    let foreground = label();
    let secondary_foreground = secondary();
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
        .child(tile(icon, secondary_foreground, style::ROW_ICON))
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
            row.child(glyph("icons/check.svg", 13.0, foreground))
        });
    ListRow::new(ElementId::from(id), content)
        .aria_label(aria_label)
        .selected(true)
        .bg(if selected {
            gpui::hsla(0.0, 0.0, 1.0, 0.08)
        } else {
            gpui::transparent_black()
        })
        .rounded(px(0.0))
        .disabled(disabled)
        .h(px(if has_detail {
            style::NAV_ROW_HEIGHT + 10.0
        } else {
            style::ROW_HEIGHT
        }))
        .px(px(style::ROW_PADDING))
}

impl Settings {
    pub(super) fn apply_sound_policy_change(
        &mut self,
        change: SoundPolicyChange,
        cx: &mut Context<Self>,
    ) {
        let preview = match change {
            SoundPolicyChange::AlertSound(cue) => {
                self.sound_policy.alert_sound = cue;
                Some(cue)
            }
            SoundPolicyChange::InterfaceEffects(enabled) => {
                self.sound_policy.interface_effects = enabled;
                None
            }
            SoundPolicyChange::VolumeFeedback(enabled) => {
                self.sound_policy.volume_feedback = enabled;
                None
            }
            SoundPolicyChange::LoginSound(enabled) => {
                self.sound_policy.login_sound = enabled;
                None
            }
        };
        if let Some(cue) = preview {
            let _ = rmac_sound::preview(cue, self.sound_policy.alert_volume);
        }
        self.persist_sound_policy(cx);
    }

    pub(super) fn schedule_sound_policy_volume(&mut self, value: f32, cx: &mut Context<Self>) {
        self.sound_policy.alert_volume = value.round().clamp(0.0, 100.0) as u8;
        self.sound_policy_generation = self.sound_policy_generation.wrapping_add(1);
        let generation = self.sound_policy_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.sound_policy_generation == generation {
                    this.persist_current_sound_policy(generation, cx);
                }
            });
        })
        .detach();
    }

    fn persist_sound_policy(&mut self, cx: &mut Context<Self>) {
        self.sound_policy_generation = self.sound_policy_generation.wrapping_add(1);
        self.persist_current_sound_policy(self.sound_policy_generation, cx);
    }

    fn persist_current_sound_policy(&mut self, generation: u64, cx: &mut Context<Self>) {
        let settings = self.sound_policy.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || rmac_sound::save_settings(&settings)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.sound_policy_generation != generation {
                    return;
                }
                this.sound_policy_error = result.err().map(|error| {
                    format!("Could not save interface sound settings: {error}").into()
                });
                cx.notify();
            });
        })
        .detach();
    }

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
}
