//! Accessibility › Pointer Control: niri-backed pointer precision, middle
//! button emulation and the key repeat preset.

use super::*;

impl Settings {
    pub(super) fn accessibility_pointer_page(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut body = div().v_flex();
        if self.input_loading {
            return body.child(footnote("Loading pointer and keyboard settings from niri…"));
        }
        let mouse = &self.input.settings.mouse;
        let mouse_writable = self.input.can_configure && mouse.enabled && !self.input_busy;
        let selected_pointer_preset = MOUSE_PRECISION_PRESETS.iter().position(|(_, change)| {
            matches!(
                change,
                InputChange::MousePrecisionPreset { speed, profile }
                    if *speed == mouse.accel_speed && *profile == mouse.accel_profile
            )
        });
        let middle_view = view.clone();
        body = body.child(card(vec![
            input_segment_row(
                view.clone(),
                "accessibility-pointer-precision",
                "Mouse precision",
                &MOUSE_PRECISION_PRESETS,
                selected_pointer_preset,
                mouse_writable,
            ),
            switch_row(
                "accessibility-middle-emulation",
                "Middle-button emulation",
                Some("Press the left and right mouse buttons together to middle-click.".into()),
                mouse.middle_emulation,
                mouse_writable,
                move |value, _, cx| {
                    middle_view.update(cx, |settings, cx| {
                        settings.apply_input_change(InputChange::MouseMiddleEmulation(value), cx)
                    });
                },
            ),
        ]));

        let keyboard = &self.input.settings.keyboard;
        let selected_preset = KEYBOARD_RESPONSE_PRESETS.iter().position(|(_, change)| {
            matches!(
                change,
                InputChange::KeyboardRepeatPreset { delay_ms, rate }
                    if *delay_ms == keyboard.repeat_delay_ms && *rate == keyboard.repeat_rate
            )
        });
        body = body
            .child(section_header("Keyboard"))
            .child(card(vec![input_segment_row(
                view.clone(),
                "accessibility-key-response",
                "Key repeat",
                &KEYBOARD_RESPONSE_PRESETS,
                selected_preset,
                self.input.can_configure && !self.input_busy,
            )]));
        if let Some(detail) = self
            .input
            .detail
            .clone()
            .filter(|_| !self.input.can_configure)
        {
            body = body.child(footnote(detail));
        }
        // What the compositor cannot do stays visible as an explanation,
        // not as controls that would only change local state.
        body.child(footnote(
            "niri has no Sticky Keys, Slow Keys, Mouse Keys or dwell click, so those controls are not shown.",
        ))
        .child(footer_buttons(vec![push_button("accessibility-input-refresh", "Refresh")
            .disabled(self.input_loading || self.input_busy)
            .on_click(move |_, _, cx| {
                view.update(cx, |settings, cx| settings.refresh_input(cx));
            })
            .into_any_element()]))
    }
}
