use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};

/// xterm's one-based modifier parameter: Shift=1, Alt=2, Control=4.
fn xterm_modifier(modifiers: &Modifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control)
}

pub(crate) fn cursor_key_sequence(
    final_byte: char,
    modifiers: &Modifiers,
    application_cursor: bool,
) -> Vec<u8> {
    let modifier = xterm_modifier(modifiers);
    if modifier != 1 {
        format!("\x1b[1;{modifier}{final_byte}").into_bytes()
    } else if application_cursor {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

fn tilde_key_sequence(code: u8, modifiers: &Modifiers) -> Vec<u8> {
    let modifier = xterm_modifier(modifiers);
    if modifier == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{modifier}~").into_bytes()
    }
}

fn function_key_sequence(key: &str, modifiers: &Modifiers) -> Option<Vec<u8>> {
    let modifier = xterm_modifier(modifiers);
    let sequence = match key {
        "f1" | "f2" | "f3" | "f4" => {
            let final_byte = match key {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if modifier == 1 {
                format!("\x1bO{final_byte}").into_bytes()
            } else {
                format!("\x1b[1;{modifier}{final_byte}").into_bytes()
            }
        }
        "f5" => tilde_key_sequence(15, modifiers),
        "f6" => tilde_key_sequence(17, modifiers),
        "f7" => tilde_key_sequence(18, modifiers),
        "f8" => tilde_key_sequence(19, modifiers),
        "f9" => tilde_key_sequence(20, modifiers),
        "f10" => tilde_key_sequence(21, modifiers),
        "f11" => tilde_key_sequence(23, modifiers),
        "f12" => tilde_key_sequence(24, modifiers),
        "f13" => tilde_key_sequence(25, modifiers),
        "f14" => tilde_key_sequence(26, modifiers),
        "f15" => tilde_key_sequence(28, modifiers),
        "f16" => tilde_key_sequence(29, modifiers),
        "f17" => tilde_key_sequence(31, modifiers),
        "f18" => tilde_key_sequence(32, modifiers),
        "f19" => tilde_key_sequence(33, modifiers),
        "f20" => tilde_key_sequence(34, modifiers),
        _ => return None,
    };
    Some(sequence)
}

fn is_supported_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=20).contains(&number))
}

/// Return true only when GPUI's platform input handler must deliver the text.
pub(crate) fn uses_platform_text_input(keystroke: &Keystroke) -> bool {
    keystroke
        .key_char
        .as_ref()
        .is_some_and(|text| !text.chars().any(char::is_control))
        && keystroke.modifiers.is_subset_of(&Modifiers::shift())
}

fn control_byte(key: &str) -> Option<u8> {
    let character = key.chars().next()?;
    if key.len() == 1 && character.is_ascii_alphabetic() {
        return Some((character.to_ascii_uppercase() as u8) & 0x1f);
    }
    match key {
        "space" | "@" | "2" => Some(0x00),
        "[" | "{" | "3" => Some(0x1b),
        "\\" | "|" | "4" => Some(0x1c),
        "]" | "}" | "5" => Some(0x1d),
        "^" | "~" | "6" => Some(0x1e),
        "_" | "/" | "7" => Some(0x1f),
        "?" | "8" => Some(0x7f),
        _ => None,
    }
}

/// Encode the traditional xterm/DEC key contract represented by GPUI.
///
/// GPUI intentionally does not preserve numeric-keypad location, so APP_KEYPAD
/// remains a separate framework gate. Enhanced Kitty keyboard modes likewise
/// require key release/location metadata beyond this key-down path.
pub(crate) fn encode_key(keystroke: &Keystroke, mode: TermMode) -> Vec<u8> {
    let modifiers = &keystroke.modifiers;
    if modifiers.platform {
        return Vec::new();
    }

    let application_cursor = mode.contains(TermMode::APP_CURSOR);
    let mut bytes = match keystroke.key.as_str() {
        "enter" => vec![b'\r'],
        "backspace" => vec![0x7f],
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        "escape" => vec![0x1b],
        "up" => cursor_key_sequence('A', modifiers, application_cursor),
        "down" => cursor_key_sequence('B', modifiers, application_cursor),
        "right" => cursor_key_sequence('C', modifiers, application_cursor),
        "left" => cursor_key_sequence('D', modifiers, application_cursor),
        "home" => cursor_key_sequence('H', modifiers, application_cursor),
        "end" => cursor_key_sequence('F', modifiers, application_cursor),
        "insert" => tilde_key_sequence(2, modifiers),
        "delete" => tilde_key_sequence(3, modifiers),
        "pageup" => tilde_key_sequence(5, modifiers),
        "pagedown" => tilde_key_sequence(6, modifiers),
        key if is_supported_function_key(key) => {
            return function_key_sequence(key, modifiers).unwrap_or_default();
        }
        key if modifiers.control => control_byte(key).into_iter().collect(),
        _ => keystroke
            .key_char
            .as_deref()
            .unwrap_or_default()
            .as_bytes()
            .to_vec(),
    };

    // xterm's conventional Meta/Alt behavior prefixes ordinary and control
    // characters with Escape. Special cursor/function keys encode Alt in
    // their modifier parameter instead.
    let parameterized_special = matches!(
        keystroke.key.as_str(),
        "up" | "down"
            | "right"
            | "left"
            | "home"
            | "end"
            | "insert"
            | "delete"
            | "pageup"
            | "pagedown"
    );
    if modifiers.alt && !parameterized_special && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{encode_key, uses_platform_text_input};
    use alacritty_terminal::term::TermMode;
    use gpui::{Keystroke, Modifiers};

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.into(),
            key_char: key_char.map(str::to_owned),
            modifiers,
        }
    }

    #[test]
    fn navigation_and_function_keys_encode_xterm_modifiers() {
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift_control = Modifiers {
            shift: true,
            control: true,
            ..Default::default()
        };

        assert_eq!(
            encode_key(&keystroke("left", None, alt), TermMode::APP_CURSOR),
            b"\x1b[1;3D"
        );
        assert_eq!(
            encode_key(
                &keystroke("delete", None, shift_control),
                TermMode::default()
            ),
            b"\x1b[3;6~"
        );
        assert_eq!(
            encode_key(
                &keystroke("f1", None, Modifiers::default()),
                TermMode::default()
            ),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(&keystroke("f1", None, shift_control), TermMode::default()),
            b"\x1b[1;6P"
        );
        assert_eq!(
            encode_key(&keystroke("f5", None, shift_control), TermMode::default()),
            b"\x1b[15;6~"
        );
        assert_eq!(
            encode_key(
                &keystroke("f20", None, Modifiers::default()),
                TermMode::default()
            ),
            b"\x1b[34~"
        );
    }

    #[test]
    fn text_control_and_meta_input_remain_exact() {
        let control = Modifiers {
            control: true,
            ..Default::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let platform = Modifiers {
            platform: true,
            ..Default::default()
        };

        assert_eq!(
            encode_key(&keystroke("c", None, control), TermMode::default()),
            b"\x03"
        );
        assert_eq!(
            encode_key(&keystroke("[", None, control), TermMode::default()),
            b"\x1b"
        );
        assert_eq!(
            encode_key(&keystroke("space", None, control), TermMode::default()),
            b"\x00"
        );
        assert_eq!(
            encode_key(
                &keystroke("f", Some("f"), Modifiers::default()),
                TermMode::default()
            ),
            b"f"
        );
        assert_eq!(
            encode_key(&keystroke("x", Some("λ"), alt), TermMode::default()),
            "\u{1b}λ".as_bytes()
        );
        assert_eq!(
            encode_key(&keystroke("tab", Some("\t"), shift), TermMode::default()),
            b"\x1b[Z"
        );
        assert!(encode_key(&keystroke("x", None, platform), TermMode::default()).is_empty());
    }

    #[test]
    fn plain_text_uses_the_platform_input_path_exactly_once() {
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let control = Modifiers {
            control: true,
            ..Default::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        let platform = Modifiers {
            platform: true,
            ..Default::default()
        };

        assert!(uses_platform_text_input(&keystroke(
            "x",
            Some("λ"),
            Modifiers::default()
        )));
        assert!(uses_platform_text_input(&keystroke("a", Some("A"), shift)));
        assert!(!uses_platform_text_input(&keystroke(
            "tab",
            Some("\t"),
            shift
        )));
        assert!(!uses_platform_text_input(&keystroke(
            "enter",
            Some("\r"),
            Modifiers::default()
        )));
        assert!(!uses_platform_text_input(&keystroke(
            "backspace",
            Some("\u{8}"),
            Modifiers::default()
        )));
        assert!(!uses_platform_text_input(&keystroke(
            "c",
            Some("c"),
            control
        )));
        assert!(!uses_platform_text_input(&keystroke("x", Some("λ"), alt)));
        assert!(!uses_platform_text_input(&keystroke(
            "x",
            Some("x"),
            platform
        )));
        assert!(!uses_platform_text_input(&keystroke(
            "left",
            None,
            Modifiers::default()
        )));
    }
}
