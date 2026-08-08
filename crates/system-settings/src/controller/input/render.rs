//! Keyboard, Mouse, and Trackpad settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_keyboard(&self, cx: &Context<Self>) -> Div {
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

    pub(in crate::controller) fn render_mouse(&self, cx: &Context<Self>) -> Div {
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

    pub(in crate::controller) fn render_trackpad(&self, cx: &Context<Self>) -> Div {
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
