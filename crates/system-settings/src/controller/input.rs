//! Keyboard, mouse, and trackpad settings authority.

use super::*;

impl Settings {
    pub(super) fn finish_input_update(
        &mut self,
        result: std::result::Result<rmac_input::Snapshot, rmac_input::Error>,
    ) {
        self.input_loading = false;
        self.input_busy = false;
        match result {
            Ok(snapshot) => {
                self.input = snapshot;
                self.input_error = None;
            }
            Err(error) => {
                self.input_error = Some(format!("Could not update Input settings: {error}").into());
            }
        }
    }
    pub(super) fn refresh_input(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || self.input_stream_refreshing {
            self.input_refresh_pending = true;
            return;
        }
        self.input_refresh_pending = false;
        self.input_stream_refreshing = true;
        self.input_generation = self.input_generation.wrapping_add(1);
        let generation = self.input_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.input_stream_refreshing = false;
                if input_stream_snapshot_is_current(
                    generation,
                    this.input_generation,
                    this.input_loading,
                    this.input_busy,
                ) {
                    this.finish_input_update(result);
                    this.flush_input_stream_refresh(cx);
                    cx.notify();
                } else {
                    this.input_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    pub(super) fn flush_input_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.input_refresh_pending
            && !self.input_loading
            && !self.input_busy
            && !self.input_stream_refreshing
        {
            self.request_input_stream_refresh(cx);
        }
    }
    pub(super) fn apply_input_change(&mut self, change: InputChange, cx: &mut Context<Self>) {
        if self.input_loading || self.input_busy || !self.input.can_configure {
            return;
        }
        let mut settings = self.input.settings.clone();
        change.apply(&mut settings);
        self.input_generation = self.input_generation.wrapping_add(1);
        self.input_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_input::save(&settings) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn input_header(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        div()
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
                    .child("niri · libinput"),
            )
            .child(
                Button::new("input-refresh", "Refresh")
                    .ghost()
                    .busy(self.input_busy)
                    .disabled(self.input_loading || self.input_busy)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |settings, cx| settings.refresh_input(cx));
                    }),
            )
    }

    pub(super) fn input_unavailable_card(&self) -> Option<Div> {
        if self.input_loading {
            return Some(note_card("Loading input settings from niri…"));
        }
        if !self.input.available || !self.input.can_configure {
            return Some(note_card(self.input.detail.clone().unwrap_or_else(|| {
                "Input configuration is unavailable in this desktop session.".into()
            })));
        }
        None
    }

    pub(super) fn input_devices_card(
        &self,
        kinds: &[rmac_input::DeviceKind],
        empty: &'static str,
    ) -> Div {
        let rows = self
            .input
            .devices
            .iter()
            .filter(|device| kinds.contains(&device.kind))
            .map(|device| {
                value_row(
                    match device.kind {
                        rmac_input::DeviceKind::Keyboard => "icons/keyboard.svg",
                        rmac_input::DeviceKind::Touchpad => "icons/touchpad.svg",
                        _ => "icons/mouse.svg",
                    },
                    secondary(),
                    device.name.clone().into(),
                    device.kind.label().into(),
                )
            })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            note_card(empty)
        } else {
            card(rows)
        }
    }

    pub(super) fn has_input_devices(&self, kinds: &[rmac_input::DeviceKind]) -> bool {
        self.input
            .devices
            .iter()
            .any(|device| kinds.contains(&device.kind))
    }

    pub(super) fn render_keyboard(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        cards.push(section_header("Connected Keyboards"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Keyboard],
            "No connected keyboard was reported by the Linux input subsystem.",
        ));
        let other_kinds = [
            rmac_input::DeviceKind::Tablet,
            rmac_input::DeviceKind::Touchscreen,
            rmac_input::DeviceKind::Other,
        ];
        if self.has_input_devices(&other_kinds) {
            cards.push(section_header("Other Input Devices"));
            cards.push(self.input_devices_card(&other_kinds, ""));
            cards.push(note_card(
                "Tablets, touchscreens, and unclassified kernel input devices are listed for live inventory; this page does not apply keyboard settings to them.",
            ));
        }
        let settings = &self.input.settings.keyboard;
        let selected_delay = KEYBOARD_DELAYS.iter().position(|(_, change)| {
            matches!(change, InputChange::KeyboardRepeatDelay(value) if *value == settings.repeat_delay_ms)
        });
        let selected_rate = KEYBOARD_RATES.iter().position(|(_, change)| {
            matches!(change, InputChange::KeyboardRepeatRate(value) if *value == settings.repeat_rate)
        });
        cards.push(section_header("Key Repeat"));
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-delay",
                "Delay until repeat",
                &KEYBOARD_DELAYS,
                selected_delay,
                self.input.can_configure && !self.input_busy,
            ),
            input_segment_row(
                cx.entity(),
                "keyboard-repeat-rate",
                "Key repeat rate",
                &KEYBOARD_RATES,
                selected_rate,
                self.input.can_configure && !self.input_busy,
            ),
            input_switch_row(
                cx.entity(),
                "keyboard-numlock",
                "icons/keyboard.svg",
                "Use Num Lock on startup",
                None,
                settings.numlock,
                self.input.can_configure && !self.input_busy,
                InputChange::KeyboardNumlock,
            ),
        ]));
        cards.push(note_card(
            "Changes are resolved across the positional include graph, candidate-validated, saved to an isolated final rmac include, and applied by niri's live reload.",
        ));
        if self.input.included_files > 0 {
            cards.push(note_card(format!(
                "Effective input values include {} recursively loaded niri configuration file(s).",
                self.input.included_files
            )));
        }
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
        self.pane(cards)
    }

    pub(super) fn render_mouse(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.mouse;
        let writable = self.input.can_configure && settings.enabled && !self.input_busy;
        cards.push(section_header("Connected Mice"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Mouse],
            "No connected mouse was reported by the Linux input subsystem.",
        ));
        let other_pointer_kinds = [
            rmac_input::DeviceKind::Trackpoint,
            rmac_input::DeviceKind::Trackball,
        ];
        if self.has_input_devices(&other_pointer_kinds) {
            cards.push(section_header("Other Pointing Devices"));
            cards.push(self.input_devices_card(&other_pointer_kinds, ""));
            cards.push(note_card(
                "Pointing sticks and trackballs use separate niri device-type sections. They are shown for live inventory and are not changed by the Mouse controls below.",
            ));
        }
        if !settings.enabled {
            cards.push(note_card(
                "The niri mouse section is explicitly off. Its effective settings are preserved but cannot affect devices until that type is enabled in the configuration.",
            ));
        }
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "mouse-tracking",
                "Tracking speed",
                &MOUSE_SPEEDS,
                Some(speed_index(settings.accel_speed)),
                writable,
            ),
            input_segment_row(
                cx.entity(),
                "mouse-acceleration",
                "Acceleration",
                &MOUSE_PROFILES,
                Some(usize::from(
                    settings.accel_profile == rmac_input::AccelProfile::Flat,
                )),
                writable,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-natural-scroll",
                "icons/mouse.svg",
                "Natural scrolling",
                Some("Move content in the direction your finger travels"),
                settings.natural_scroll,
                writable,
                InputChange::MouseNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-left-handed",
                "icons/mouse.svg",
                "Primary button on right",
                Some("Swap the left and right mouse buttons"),
                settings.left_handed,
                writable,
                InputChange::MouseLeftHanded,
            ),
            input_switch_row(
                cx.entity(),
                "mouse-middle-emulation",
                "icons/mouse.svg",
                "Middle-button emulation",
                Some("Press the left and right buttons together for middle click"),
                settings.middle_emulation,
                writable,
                InputChange::MouseMiddleEmulation,
            ),
        ]));
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
        self.pane(cards)
    }

    pub(super) fn render_trackpad(&self, cx: &Context<Self>) -> Div {
        let mut cards = vec![self.input_header(cx)];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.touchpad;
        let writable = self.input.can_configure && settings.pointer.enabled && !self.input_busy;
        cards.push(section_header("Connected Trackpads"));
        cards.push(self.input_devices_card(
            &[rmac_input::DeviceKind::Touchpad],
            "No connected trackpad was reported by the Linux input subsystem.",
        ));
        if !settings.pointer.enabled {
            cards.push(note_card(
                "The niri touchpad section is explicitly off. Its effective settings are preserved but cannot affect devices until that type is enabled in the configuration.",
            ));
        }
        cards.push(card(vec![
            input_segment_row(
                cx.entity(),
                "touchpad-tracking",
                "Tracking speed",
                &TOUCHPAD_SPEEDS,
                Some(speed_index(settings.pointer.accel_speed)),
                writable,
            ),
            input_segment_row(
                cx.entity(),
                "touchpad-acceleration",
                "Acceleration",
                &TOUCHPAD_PROFILES,
                Some(usize::from(
                    settings.pointer.accel_profile == rmac_input::AccelProfile::Flat,
                )),
                writable,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-tap",
                "icons/touchpad.svg",
                "Tap to click",
                None,
                settings.tap_to_click,
                writable,
                InputChange::TouchpadTap,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-natural-scroll",
                "icons/touchpad.svg",
                "Natural scrolling",
                Some("Move content in the direction your fingers travel"),
                settings.pointer.natural_scroll,
                writable,
                InputChange::TouchpadNaturalScroll,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-dwt",
                "icons/keyboard.svg",
                "Ignore while typing",
                Some("Prevent accidental pointer movement while typing"),
                settings.disable_while_typing,
                writable,
                InputChange::TouchpadDwt,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-drag-lock",
                "icons/touchpad.svg",
                "Drag lock",
                Some("Keep dragging briefly after lifting your finger"),
                settings.drag_lock,
                writable,
                InputChange::TouchpadDragLock,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-left-handed",
                "icons/touchpad.svg",
                "Primary click on right",
                None,
                settings.pointer.left_handed,
                writable,
                InputChange::TouchpadLeftHanded,
            ),
            input_switch_row(
                cx.entity(),
                "touchpad-middle-emulation",
                "icons/touchpad.svg",
                "Middle-click emulation",
                Some("Press the left and right click areas together"),
                settings.pointer.middle_emulation,
                writable,
                InputChange::TouchpadMiddleEmulation,
            ),
        ]));
        cards.push(note_card(self.input.device_overrides.detail()));
        if let Some(detail) = &self.input.device_detail {
            cards.push(note_card(detail.clone()));
        }
        self.pane(cards)
    }
}
