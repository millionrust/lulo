use alacritty_terminal::term::TermMode;
use gpui::{Modifiers, MouseButton, NavigationDirection};

const MAX_LEGACY_MOUSE_COORD: usize = 223;
const MAX_UTF8_MOUSE_COORD: usize = 2015;
const MAX_WHEEL_REPORTS_PER_AXIS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseReport {
    Press(MouseButton),
    Release(MouseButton),
    Motion(Option<MouseButton>),
    Wheel(u16),
}

fn button_code(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Navigate(NavigationDirection::Back) => 128,
        MouseButton::Navigate(NavigationDirection::Forward) => 129,
    }
}

fn modifier_bits(modifiers: &Modifiers) -> u16 {
    4 * u16::from(modifiers.shift)
        + 8 * u16::from(modifiers.alt)
        + 16 * u16::from(modifiers.control)
}

pub(crate) fn motion_report(mode: TermMode, pressed: Option<MouseButton>) -> Option<MouseReport> {
    if mode.contains(TermMode::MOUSE_MOTION) {
        Some(MouseReport::Motion(pressed))
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        pressed.map(|button| MouseReport::Motion(Some(button)))
    } else {
        None
    }
}

fn append_utf8_value(bytes: &mut Vec<u8>, value: u16) -> Option<()> {
    let character = char::from_u32(u32::from(value))?;
    let mut encoded = [0; 4];
    bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
    Some(())
}

/// Encode an xterm mouse report for a zero-based viewport cell.
pub(crate) fn encode_report(
    mode: TermMode,
    report: MouseReport,
    column: usize,
    row: usize,
    modifiers: &Modifiers,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    let x = column.checked_add(1)?;
    let y = row.checked_add(1)?;
    let modifier = modifier_bits(modifiers);
    let (button, release) = match report {
        MouseReport::Press(button) => (button_code(button) + modifier, false),
        MouseReport::Release(button) => (button_code(button) + modifier, true),
        MouseReport::Motion(button) => (button.map_or(3, button_code) + 32 + modifier, false),
        MouseReport::Wheel(button) => (button + modifier, false),
    };

    if mode.contains(TermMode::SGR_MOUSE) {
        return Some(
            format!("\x1b[<{button};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes(),
        );
    }

    let max_coordinate = if mode.contains(TermMode::UTF8_MOUSE) {
        MAX_UTF8_MOUSE_COORD
    } else {
        MAX_LEGACY_MOUSE_COORD
    };
    if x > max_coordinate || y > max_coordinate {
        return None;
    }

    // Legacy release reports discard button identity and use low bits 3.
    let button = if release { 3 + modifier } else { button };
    let mut bytes = b"\x1b[M".to_vec();
    if mode.contains(TermMode::UTF8_MOUSE) {
        append_utf8_value(&mut bytes, button + 32)?;
        append_utf8_value(&mut bytes, u16::try_from(x).ok()? + 32)?;
        append_utf8_value(&mut bytes, u16::try_from(y).ok()? + 32)?;
    } else {
        bytes.extend_from_slice(&[
            u8::try_from(button + 32).ok()?,
            u8::try_from(x + 32).ok()?,
            u8::try_from(y + 32).ok()?,
        ]);
    }
    Some(bytes)
}

pub(crate) fn accumulate_wheel_reports(
    accumulator: &mut f32,
    delta: f32,
    positive_button: u16,
    negative_button: u16,
) -> Vec<MouseReport> {
    if !delta.is_finite() {
        return Vec::new();
    }
    let total = *accumulator + delta;
    let unbounded_steps = total.trunc() as i32;
    let step_limit = MAX_WHEEL_REPORTS_PER_AXIS as i32;
    let steps = unbounded_steps.clamp(-step_limit, step_limit);
    // Keep genuine sub-line precision, but discard deliberately bounded excess
    // instead of leaking a near-complete extra report into the next event.
    *accumulator = if steps == unbounded_steps {
        total - steps as f32
    } else {
        0.0
    };
    let button = if steps >= 0 {
        positive_button
    } else {
        negative_button
    };
    (0..steps.unsigned_abs().min(MAX_WHEEL_REPORTS_PER_AXIS as u32))
        .map(|_| MouseReport::Wheel(button))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        accumulate_wheel_reports, append_utf8_value, encode_report, motion_report, MouseReport,
        MAX_LEGACY_MOUSE_COORD, MAX_UTF8_MOUSE_COORD, MAX_WHEEL_REPORTS_PER_AXIS,
    };
    use alacritty_terminal::term::TermMode;
    use gpui::{Modifiers, MouseButton, NavigationDirection};

    #[test]
    fn resource_bounds_are_explicit() {
        assert_eq!(MAX_LEGACY_MOUSE_COORD, 223);
        assert_eq!(MAX_UTF8_MOUSE_COORD, 2015);
        assert_eq!(MAX_WHEEL_REPORTS_PER_AXIS, 16);
    }

    #[test]
    fn sgr_reports_follow_xterm_cells() {
        let mode = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_report(
                mode,
                MouseReport::Press(MouseButton::Left),
                0,
                0,
                &Modifiers::default()
            )
            .as_deref(),
            Some(b"\x1b[<0;1;1M".as_slice())
        );
        assert_eq!(
            encode_report(
                mode,
                MouseReport::Release(MouseButton::Right),
                499,
                299,
                &Modifiers {
                    control: true,
                    ..Modifiers::default()
                }
            )
            .as_deref(),
            Some(b"\x1b[<18;500;300m".as_slice())
        );
        assert_eq!(
            encode_report(
                mode,
                MouseReport::Motion(Some(MouseButton::Middle)),
                4,
                6,
                &Modifiers {
                    alt: true,
                    ..Modifiers::default()
                }
            )
            .as_deref(),
            Some(b"\x1b[<41;5;7M".as_slice())
        );
        assert_eq!(
            encode_report(
                mode,
                MouseReport::Press(MouseButton::Navigate(NavigationDirection::Back)),
                2,
                3,
                &Modifiers::default()
            )
            .as_deref(),
            Some(b"\x1b[<128;3;4M".as_slice())
        );
    }

    #[test]
    fn legacy_and_utf8_encodings_refuse_unrepresentable_cells() {
        let legacy = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            encode_report(
                legacy,
                MouseReport::Press(MouseButton::Left),
                0,
                0,
                &Modifiers::default()
            ),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        assert_eq!(
            encode_report(
                legacy,
                MouseReport::Release(MouseButton::Left),
                0,
                0,
                &Modifiers {
                    control: true,
                    ..Modifiers::default()
                }
            ),
            Some(vec![0x1b, b'[', b'M', 51, 33, 33])
        );
        assert!(encode_report(
            legacy,
            MouseReport::Press(MouseButton::Left),
            MAX_LEGACY_MOUSE_COORD,
            0,
            &Modifiers::default()
        )
        .is_none());
        assert!(encode_report(
            legacy,
            MouseReport::Press(MouseButton::Left),
            MAX_LEGACY_MOUSE_COORD - 1,
            MAX_LEGACY_MOUSE_COORD - 1,
            &Modifiers::default()
        )
        .is_some());

        let utf8 = legacy | TermMode::UTF8_MOUSE;
        let encoded = encode_report(
            utf8,
            MouseReport::Press(MouseButton::Left),
            499,
            299,
            &Modifiers::default(),
        )
        .expect("500x300 must fit UTF-8 mouse coordinates");
        let mut expected = b"\x1b[M ".to_vec();
        append_utf8_value(&mut expected, 532).expect("valid x coordinate");
        append_utf8_value(&mut expected, 332).expect("valid y coordinate");
        assert_eq!(encoded, expected);
        assert!(encode_report(
            utf8,
            MouseReport::Press(MouseButton::Left),
            MAX_UTF8_MOUSE_COORD - 1,
            MAX_UTF8_MOUSE_COORD - 1,
            &Modifiers::default()
        )
        .is_some());
        assert!(encode_report(
            utf8,
            MouseReport::Press(MouseButton::Left),
            MAX_UTF8_MOUSE_COORD,
            0,
            &Modifiers::default()
        )
        .is_none());
    }

    #[test]
    fn motion_and_wheel_reports_are_mode_correct_and_bounded() {
        assert_eq!(
            motion_report(TermMode::MOUSE_REPORT_CLICK, Some(MouseButton::Left)),
            None
        );
        assert_eq!(motion_report(TermMode::MOUSE_DRAG, None), None);
        assert_eq!(
            motion_report(TermMode::MOUSE_DRAG, Some(MouseButton::Right)),
            Some(MouseReport::Motion(Some(MouseButton::Right)))
        );
        assert_eq!(
            motion_report(TermMode::MOUSE_MOTION, None),
            Some(MouseReport::Motion(None))
        );

        let mut accumulator = 0.0;
        assert!(accumulate_wheel_reports(&mut accumulator, 0.4, 64, 65).is_empty());
        assert_eq!(
            accumulate_wheel_reports(&mut accumulator, 0.7, 64, 65),
            vec![MouseReport::Wheel(64)]
        );
        assert_eq!(
            accumulate_wheel_reports(&mut accumulator, -2.2, 64, 65),
            vec![MouseReport::Wheel(65), MouseReport::Wheel(65)]
        );
        assert_eq!(
            accumulate_wheel_reports(&mut accumulator, 1000.0, 64, 65).len(),
            MAX_WHEEL_REPORTS_PER_AXIS
        );
        assert_eq!(accumulator, 0.0);
        assert!(accumulate_wheel_reports(&mut accumulator, f32::NAN, 64, 65).is_empty());
    }
}
