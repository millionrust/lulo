//! Accessibility keyboard and pointer presentation.

use super::*;

impl Settings {
    pub(super) fn append_accessibility_motor(
        &self,
        view: Entity<Self>,
        cx: &Context<Self>,
        cards: &mut Vec<Div>,
    ) {
        let keyboard_view = view.clone();
        let mouse_view = view.clone();
        let trackpad_view = view;
        cards.push(section_header("Motor"));
        cards.push(card(vec![
            row_base()
                .child(tile("icons/keyboard.svg", secondary(), style::ROW_ICON))
                .child(text_block(
                    "Keyboard".into(),
                    Some("Repeat timing and layout controls backed by niri".into()),
                ))
                .child(
                    Button::new("accessibility-keyboard", "Open").on_click(move |_, _, cx| {
                        keyboard_view
                            .update(cx, |settings, cx| settings.select_category("Keyboard", cx));
                    }),
                )
                .into_any_element(),
            row_base()
                .child(tile("icons/mouse.svg", secondary(), style::ROW_ICON))
                .child(text_block(
                    "Pointer".into(),
                    Some("Speed, acceleration, handedness, and scroll controls".into()),
                ))
                .child(
                    Button::new("accessibility-mouse", "Mouse").on_click(move |_, _, cx| {
                        mouse_view.update(cx, |settings, cx| settings.select_category("Mouse", cx));
                    }),
                )
                .child(Button::new("accessibility-trackpad", "Trackpad").on_click(
                    move |_, _, cx| {
                        trackpad_view
                            .update(cx, |settings, cx| settings.select_category("Trackpad", cx));
                    },
                ))
                .into_any_element(),
        ]));

        if !self.input_loading {
            let keyboard = &self.input.settings.keyboard;
            let selected_preset = KEYBOARD_RESPONSE_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::KeyboardRepeatPreset { delay_ms, rate }
                        if *delay_ms == keyboard.repeat_delay_ms && *rate == keyboard.repeat_rate
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-key-response",
                    "Key repeat preset",
                    &KEYBOARD_RESPONSE_PRESETS,
                    selected_preset,
                    self.input.can_configure && !self.input_busy,
                ),
                value_row(
                    "icons/keyboard.svg",
                    secondary(),
                    "Effective key repeat".into(),
                    format!(
                        "{} ms delay · {} characters/s",
                        keyboard.repeat_delay_ms, keyboard.repeat_rate
                    )
                    .into(),
                ),
            ]));
            let mouse = &self.input.settings.mouse;
            let mouse_writable = self.input.can_configure && mouse.enabled && !self.input_busy;
            let selected_pointer_preset = MOUSE_PRECISION_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::MousePrecisionPreset { speed, profile }
                        if *speed == mouse.accel_speed && *profile == mouse.accel_profile
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-pointer-precision",
                    "Mouse precision",
                    &MOUSE_PRECISION_PRESETS,
                    selected_pointer_preset,
                    mouse_writable,
                ),
                input_switch_row(
                    cx.entity(),
                    "accessibility-middle-emulation",
                    "icons/mouse.svg",
                    "Middle-button emulation",
                    Some("Press the left and right mouse buttons together"),
                    mouse.middle_emulation,
                    mouse_writable,
                    InputChange::MouseMiddleEmulation,
                ),
                value_row(
                    "icons/mouse.svg",
                    secondary(),
                    "Effective mouse response".into(),
                    format!(
                        "{} acceleration · speed {}",
                        mouse.accel_profile.label(),
                        mouse.accel_speed
                    )
                    .into(),
                ),
            ]));
            if let Some(detail) = self
                .input
                .detail
                .clone()
                .filter(|_| !self.input.can_configure)
            {
                cards.push(note_card(detail));
            }
        } else {
            cards.push(note_card(
                "Loading keyboard accessibility settings from niri…",
            ));
        }
        cards.push(note_card(
            "Niri currently provides repeat timing but no compositor authority for Sticky Keys, Slow Keys, or Bounce Keys. Those controls remain unavailable instead of being simulated inside individual apps.",
        ));
        cards.push(note_card(
            "Mouse precision and middle-button emulation are applied by niri through libinput. Niri does not currently provide Mouse Keys, dwell click, or a session-wide double-click timing authority, so those controls remain unavailable.",
        ));
    }
}
