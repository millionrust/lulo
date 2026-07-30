#[derive(Clone, Copy)]
pub(super) enum InputChange {
    KeyboardRepeatDelay(u32),
    KeyboardRepeatRate(u32),
    KeyboardRepeatPreset {
        delay_ms: u32,
        rate: u32,
    },
    KeyboardNumlock(bool),
    MouseNaturalScroll(bool),
    MouseLeftHanded(bool),
    MouseMiddleEmulation(bool),
    MouseAccelSpeed(f64),
    MouseAccelProfile(rmac_input::AccelProfile),
    MousePrecisionPreset {
        speed: f64,
        profile: rmac_input::AccelProfile,
    },
    TouchpadNaturalScroll(bool),
    TouchpadLeftHanded(bool),
    TouchpadMiddleEmulation(bool),
    TouchpadAccelSpeed(f64),
    TouchpadAccelProfile(rmac_input::AccelProfile),
    TouchpadTap(bool),
    TouchpadDwt(bool),
    TouchpadDragLock(bool),
}

impl InputChange {
    pub(super) fn apply(self, settings: &mut rmac_input::InputSettings) {
        match self {
            Self::KeyboardRepeatDelay(value) => settings.keyboard.repeat_delay_ms = value,
            Self::KeyboardRepeatRate(value) => settings.keyboard.repeat_rate = value,
            Self::KeyboardRepeatPreset { delay_ms, rate } => {
                settings.keyboard.repeat_delay_ms = delay_ms;
                settings.keyboard.repeat_rate = rate;
            }
            Self::KeyboardNumlock(value) => settings.keyboard.numlock = value,
            Self::MouseNaturalScroll(value) => settings.mouse.natural_scroll = value,
            Self::MouseLeftHanded(value) => settings.mouse.left_handed = value,
            Self::MouseMiddleEmulation(value) => settings.mouse.middle_emulation = value,
            Self::MouseAccelSpeed(value) => settings.mouse.accel_speed = value,
            Self::MouseAccelProfile(value) => settings.mouse.accel_profile = value,
            Self::MousePrecisionPreset { speed, profile } => {
                settings.mouse.accel_speed = speed;
                settings.mouse.accel_profile = profile;
            }
            Self::TouchpadNaturalScroll(value) => settings.touchpad.pointer.natural_scroll = value,
            Self::TouchpadLeftHanded(value) => settings.touchpad.pointer.left_handed = value,
            Self::TouchpadMiddleEmulation(value) => {
                settings.touchpad.pointer.middle_emulation = value
            }
            Self::TouchpadAccelSpeed(value) => settings.touchpad.pointer.accel_speed = value,
            Self::TouchpadAccelProfile(value) => settings.touchpad.pointer.accel_profile = value,
            Self::TouchpadTap(value) => settings.touchpad.tap_to_click = value,
            Self::TouchpadDwt(value) => settings.touchpad.disable_while_typing = value,
            Self::TouchpadDragLock(value) => settings.touchpad.drag_lock = value,
        }
    }
}

pub(super) type InputOption = (&'static str, InputChange);

pub(super) const KEYBOARD_DELAYS: [InputOption; 5] = [
    ("Short", InputChange::KeyboardRepeatDelay(200)),
    ("300", InputChange::KeyboardRepeatDelay(300)),
    ("500", InputChange::KeyboardRepeatDelay(500)),
    ("750", InputChange::KeyboardRepeatDelay(750)),
    ("Long", InputChange::KeyboardRepeatDelay(1_000)),
];

pub(super) const KEYBOARD_RATES: [InputOption; 5] = [
    ("Slow", InputChange::KeyboardRepeatRate(10)),
    ("20", InputChange::KeyboardRepeatRate(20)),
    ("30", InputChange::KeyboardRepeatRate(30)),
    ("40", InputChange::KeyboardRepeatRate(40)),
    ("Fast", InputChange::KeyboardRepeatRate(60)),
];

pub(super) const KEYBOARD_RESPONSE_PRESETS: [InputOption; 3] = [
    (
        "Standard",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 600,
            rate: 25,
        },
    ),
    (
        "Deliberate",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 1_000,
            rate: 15,
        },
    ),
    (
        "Minimal",
        InputChange::KeyboardRepeatPreset {
            delay_ms: 1_500,
            rate: 10,
        },
    ),
];

pub(super) const MOUSE_SPEEDS: [InputOption; 5] = [
    ("Slow", InputChange::MouseAccelSpeed(-1.0)),
    ("−0.5", InputChange::MouseAccelSpeed(-0.5)),
    ("Default", InputChange::MouseAccelSpeed(0.0)),
    ("0.5", InputChange::MouseAccelSpeed(0.5)),
    ("Fast", InputChange::MouseAccelSpeed(1.0)),
];

pub(super) const TOUCHPAD_SPEEDS: [InputOption; 5] = [
    ("Slow", InputChange::TouchpadAccelSpeed(-1.0)),
    ("−0.5", InputChange::TouchpadAccelSpeed(-0.5)),
    ("Default", InputChange::TouchpadAccelSpeed(0.0)),
    ("0.5", InputChange::TouchpadAccelSpeed(0.5)),
    ("Fast", InputChange::TouchpadAccelSpeed(1.0)),
];

pub(super) const MOUSE_PROFILES: [InputOption; 2] = [
    (
        "Adaptive",
        InputChange::MouseAccelProfile(rmac_input::AccelProfile::Adaptive),
    ),
    (
        "Flat",
        InputChange::MouseAccelProfile(rmac_input::AccelProfile::Flat),
    ),
];

pub(super) const MOUSE_PRECISION_PRESETS: [InputOption; 3] = [
    (
        "Standard",
        InputChange::MousePrecisionPreset {
            speed: 0.0,
            profile: rmac_input::AccelProfile::Adaptive,
        },
    ),
    (
        "Steady",
        InputChange::MousePrecisionPreset {
            speed: -0.5,
            profile: rmac_input::AccelProfile::Adaptive,
        },
    ),
    (
        "Precise",
        InputChange::MousePrecisionPreset {
            speed: -0.5,
            profile: rmac_input::AccelProfile::Flat,
        },
    ),
];

pub(super) const TOUCHPAD_PROFILES: [InputOption; 2] = [
    (
        "Adaptive",
        InputChange::TouchpadAccelProfile(rmac_input::AccelProfile::Adaptive),
    ),
    (
        "Flat",
        InputChange::TouchpadAccelProfile(rmac_input::AccelProfile::Flat),
    ),
];

pub(super) fn speed_index(speed: f64) -> usize {
    [-1.0, -0.5, 0.0, 0.5, 1.0]
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (speed - **a).abs().total_cmp(&(speed - **b).abs()))
        .map(|(index, _)| index)
        .unwrap_or(2)
}

pub(super) fn compositor_event_affects_input(event: &rmac_compositor::Event) -> bool {
    compositor_input_config_failed(event) == Some(false)
}

pub(super) fn compositor_input_config_failed(event: &rmac_compositor::Event) -> Option<bool> {
    match event {
        rmac_compositor::Event::Unknown {
            source_kind,
            payload,
        } if source_kind == "ConfigLoaded" => payload["failed"].as_bool(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_only_after_a_loaded_niri_configuration() {
        let mut loaded = rmac_compositor::Event::Unknown {
            source_kind: "ConfigLoaded".into(),
            payload: Default::default(),
        };
        let rmac_compositor::Event::Unknown { payload, .. } = &mut loaded else {
            unreachable!()
        };
        payload["failed"] = false.into();
        assert!(compositor_event_affects_input(&loaded));
        assert_eq!(compositor_input_config_failed(&loaded), Some(false));

        let rmac_compositor::Event::Unknown { payload, .. } = &mut loaded else {
            unreachable!()
        };
        payload["failed"] = true.into();
        assert!(!compositor_event_affects_input(&loaded));
        assert_eq!(compositor_input_config_failed(&loaded), Some(true));
        assert!(!compositor_event_affects_input(
            &rmac_compositor::Event::OutputsReplaced {
                outputs: Vec::new(),
            }
        ));
    }

    #[test]
    fn precision_preset_preserves_keyboard_and_touchpad_settings() {
        let original = rmac_input::InputSettings::default();
        let mut changed = original.clone();
        InputChange::MousePrecisionPreset {
            speed: -0.5,
            profile: rmac_input::AccelProfile::Flat,
        }
        .apply(&mut changed);

        assert_eq!(changed.mouse.accel_speed, -0.5);
        assert_eq!(changed.mouse.accel_profile, rmac_input::AccelProfile::Flat);
        assert_eq!(changed.keyboard, original.keyboard);
        assert_eq!(changed.touchpad, original.touchpad);
    }
}
