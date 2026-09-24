use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

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
///
/// `option_as_meta` is the Mac's "Use Option as Meta Key" setting (off by
/// default): while it is off, ⌥-combinations reach here too, so the
/// platform's composed character (e.g. "#" from ⌥3 on a UK layout) types
/// instead of `encode_legacy_key`'s Escape-prefixed Meta sequence.
pub(crate) fn uses_platform_text_input(
    keystroke: &Keystroke,
    mode: TermMode,
    option_as_meta: bool,
) -> bool {
    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        return false;
    }
    let mut allowed = Modifiers::shift();
    if !option_as_meta {
        allowed.alt = true;
    }
    keystroke
        .key_char
        .as_ref()
        .is_some_and(|text| !text.chars().any(char::is_control))
        && keystroke.modifiers.is_subset_of(&allowed)
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
fn encode_legacy_key(keystroke: &Keystroke, mode: TermMode) -> Vec<u8> {
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

#[derive(Clone, Copy)]
enum SequenceTerminator {
    Character(char),
    Kitty,
}

impl SequenceTerminator {
    fn character(self) -> char {
        match self {
            Self::Character(character) => character,
            Self::Kitty => 'u',
        }
    }
}

fn functional_sequence_base(
    key: &str,
    kitty_sequence: bool,
    include_one: bool,
) -> Option<(String, SequenceTerminator)> {
    let one = if include_one { "1" } else { "" };
    let sequence = match key {
        "up" => (one.into(), SequenceTerminator::Character('A')),
        "down" => (one.into(), SequenceTerminator::Character('B')),
        "right" => (one.into(), SequenceTerminator::Character('C')),
        "left" => (one.into(), SequenceTerminator::Character('D')),
        "home" => (one.into(), SequenceTerminator::Character('H')),
        "end" => (one.into(), SequenceTerminator::Character('F')),
        "insert" => ("2".into(), SequenceTerminator::Character('~')),
        "delete" => ("3".into(), SequenceTerminator::Character('~')),
        "pageup" => ("5".into(), SequenceTerminator::Character('~')),
        "pagedown" => ("6".into(), SequenceTerminator::Character('~')),
        "f1" => (one.into(), SequenceTerminator::Character('P')),
        "f2" => (one.into(), SequenceTerminator::Character('Q')),
        "f3" if kitty_sequence => ("13".into(), SequenceTerminator::Character('~')),
        "f3" => (one.into(), SequenceTerminator::Character('R')),
        "f4" => (one.into(), SequenceTerminator::Character('S')),
        "f5" => ("15".into(), SequenceTerminator::Character('~')),
        "f6" => ("17".into(), SequenceTerminator::Character('~')),
        "f7" => ("18".into(), SequenceTerminator::Character('~')),
        "f8" => ("19".into(), SequenceTerminator::Character('~')),
        "f9" => ("20".into(), SequenceTerminator::Character('~')),
        "f10" => ("21".into(), SequenceTerminator::Character('~')),
        "f11" => ("23".into(), SequenceTerminator::Character('~')),
        "f12" => ("24".into(), SequenceTerminator::Character('~')),
        key @ ("f13" | "f14" | "f15" | "f16" | "f17" | "f18" | "f19" | "f20") if kitty_sequence => {
            let number = key[1..].parse::<u32>().ok()?;
            ((57_363 + number).to_string(), SequenceTerminator::Kitty)
        }
        "f13" => ("25".into(), SequenceTerminator::Character('~')),
        "f14" => ("26".into(), SequenceTerminator::Character('~')),
        "f15" => ("28".into(), SequenceTerminator::Character('~')),
        "f16" => ("29".into(), SequenceTerminator::Character('~')),
        "f17" => ("31".into(), SequenceTerminator::Character('~')),
        "f18" => ("32".into(), SequenceTerminator::Character('~')),
        "f19" => ("33".into(), SequenceTerminator::Character('~')),
        "f20" => ("34".into(), SequenceTerminator::Character('~')),
        _ => return None,
    };
    Some(sequence)
}

fn control_key_code(key: &str) -> Option<u32> {
    match key {
        "tab" => Some(9),
        "enter" => Some(13),
        "escape" => Some(27),
        "space" => Some(32),
        "backspace" => Some(127),
        _ => None,
    }
}

fn text_key_code(keystroke: &Keystroke, report_alternate: bool) -> Option<String> {
    let key = keystroke.key.chars().next()?;
    if keystroke.key.chars().count() != 1 {
        return None;
    }
    let unshifted = key.to_lowercase().next().unwrap_or(key);
    let key_code = u32::from(unshifted);
    let shifted = keystroke
        .key_char
        .as_deref()
        .and_then(|text| {
            let mut characters = text.chars();
            let character = characters.next()?;
            characters.next().is_none().then_some(character)
        })
        .map(u32::from);
    match shifted {
        Some(shifted) if report_alternate && keystroke.modifiers.shift && shifted != key_code => {
            Some(format!("{key_code}:{shifted}"))
        }
        _ => Some(key_code.to_string()),
    }
}

fn associated_text(keystroke: &Keystroke, mode: TermMode, event: KeyEventKind) -> Option<String> {
    if event == KeyEventKind::Release
        || !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT)
    {
        return None;
    }
    let text = keystroke.key_char.as_deref()?;
    if text.is_empty()
        || text
            .chars()
            .any(|character| character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&character))
    {
        return None;
    }
    Some(
        text.chars()
            .map(|character| u32::from(character).to_string())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

fn should_use_kitty_sequence(keystroke: &Keystroke, mode: TermMode, event: KeyEventKind) -> bool {
    let enhanced = mode.intersects(
        TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ALL_KEYS_AS_ESC,
    );
    if !enhanced {
        return false;
    }
    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) || event == KeyEventKind::Release {
        return true;
    }
    if functional_sequence_base(&keystroke.key, true, false).is_some() {
        return true;
    }
    mode.contains(TermMode::DISAMBIGUATE_ESC_CODES)
        && (keystroke.key == "escape"
            || (keystroke.modifiers != Modifiers::default()
                && (!keystroke.modifiers.is_subset_of(&Modifiers::shift())
                    || matches!(keystroke.key.as_str(), "tab" | "enter" | "backspace"))))
}

fn encode_kitty_key(keystroke: &Keystroke, mode: TermMode, event: KeyEventKind) -> Vec<u8> {
    let explicit_event = mode.contains(TermMode::REPORT_EVENT_TYPES)
        && matches!(event, KeyEventKind::Repeat | KeyEventKind::Release);
    let associated = associated_text(keystroke, mode, event);
    let include_one =
        explicit_event || xterm_modifier(&keystroke.modifiers) != 1 || associated.is_some();
    let kitty_sequence = mode.intersects(
        TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ALL_KEYS_AS_ESC,
    );

    let (payload, terminator) = if let Some(sequence) =
        functional_sequence_base(&keystroke.key, kitty_sequence, include_one)
    {
        sequence
    } else if let Some(code) = control_key_code(&keystroke.key) {
        (code.to_string(), SequenceTerminator::Kitty)
    } else if let Some(code) =
        text_key_code(keystroke, mode.contains(TermMode::REPORT_ALTERNATE_KEYS))
    {
        (code, SequenceTerminator::Kitty)
    } else if associated.is_some() {
        ("0".into(), SequenceTerminator::Kitty)
    } else {
        return Vec::new();
    };

    let mut sequence = format!("\x1b[{payload}");
    if include_one {
        sequence.push(';');
        sequence.push_str(&xterm_modifier(&keystroke.modifiers).to_string());
    }
    if explicit_event {
        sequence.push(':');
        sequence.push(match event {
            KeyEventKind::Press => '1',
            KeyEventKind::Repeat => '2',
            KeyEventKind::Release => '3',
        });
    }
    if let Some(text) = associated {
        sequence.push(';');
        sequence.push_str(&text);
    }
    sequence.push(terminator.character());
    sequence.into_bytes()
}

/// Encode one physical key event according to the terminal's parsed mode.
pub(crate) fn encode_key_event(
    keystroke: &Keystroke,
    mode: TermMode,
    event: KeyEventKind,
) -> Vec<u8> {
    // Application-level Command shortcuts remain owned by the macOS-like shell.
    if keystroke.modifiers.platform {
        return Vec::new();
    }
    if event == KeyEventKind::Release && !mode.contains(TermMode::REPORT_EVENT_TYPES) {
        return Vec::new();
    }
    if event == KeyEventKind::Release
        && !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
        && matches!(keystroke.key.as_str(), "enter" | "tab" | "backspace")
    {
        return Vec::new();
    }

    if should_use_kitty_sequence(keystroke, mode, event) {
        encode_kitty_key(keystroke, mode, event)
    } else if event == KeyEventKind::Release {
        Vec::new()
    } else {
        encode_legacy_key(keystroke, mode)
    }
}

/// Encode a platform IME commit. Multi-scalar commits have no physical key
/// identity, so the Kitty protocol uses key code zero only when the application
/// explicitly requested associated text; otherwise the committed UTF-8 remains
/// the truthful input representation.
pub(crate) fn encode_text_input(text: &str, mode: TermMode) -> Vec<u8> {
    if text.is_empty()
        || !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT)
    {
        return text.as_bytes().to_vec();
    }
    if text
        .chars()
        .any(|character| character <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&character))
    {
        return Vec::new();
    }
    let codepoints = text
        .chars()
        .map(|character| u32::from(character).to_string())
        .collect::<Vec<_>>()
        .join(":");
    format!("\x1b[0;1;{codepoints}u").into_bytes()
}

#[cfg(test)]
pub(crate) fn encode_key(keystroke: &Keystroke, mode: TermMode) -> Vec<u8> {
    encode_key_event(keystroke, mode, KeyEventKind::Press)
}

#[cfg(test)]
mod tests {
    use super::{
        encode_key, encode_key_event, encode_text_input, uses_platform_text_input, KeyEventKind,
    };
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

        assert!(uses_platform_text_input(
            &keystroke("x", Some("λ"), Modifiers::default()),
            TermMode::default(),
            true
        ));
        assert!(uses_platform_text_input(
            &keystroke("a", Some("A"), shift),
            TermMode::default(),
            true
        ));
        assert!(!uses_platform_text_input(
            &keystroke("tab", Some("\t"), shift),
            TermMode::default(),
            true
        ));
        assert!(!uses_platform_text_input(
            &keystroke("enter", Some("\r"), Modifiers::default()),
            TermMode::default(),
            true
        ));
        assert!(!uses_platform_text_input(
            &keystroke("backspace", Some("\u{8}"), Modifiers::default()),
            TermMode::default(),
            true
        ));
        assert!(!uses_platform_text_input(
            &keystroke("c", Some("c"), control),
            TermMode::default(),
            true
        ));
        // Use Option as Meta Key: on, ⌥ stays reserved for the legacy Meta
        // path; off (the Mac's default), it reaches the platform's composed
        // character instead.
        assert!(!uses_platform_text_input(
            &keystroke("x", Some("λ"), alt),
            TermMode::default(),
            true
        ));
        assert!(uses_platform_text_input(
            &keystroke("x", Some("λ"), alt),
            TermMode::default(),
            false
        ));
        assert!(!uses_platform_text_input(
            &keystroke("x", Some("x"), platform),
            TermMode::default(),
            false
        ));
        assert!(!uses_platform_text_input(
            &keystroke("left", None, Modifiers::default()),
            TermMode::default(),
            false
        ));
        assert!(!uses_platform_text_input(
            &keystroke("x", Some("x"), Modifiers::default()),
            TermMode::REPORT_ALL_KEYS_AS_ESC,
            false
        ));
    }

    #[test]
    fn kitty_modes_encode_disambiguated_repeated_and_released_events() {
        let control_shift = Modifiers {
            control: true,
            shift: true,
            ..Default::default()
        };
        let key = keystroke("a", Some("A"), control_shift);
        assert_eq!(
            encode_key_event(&key, TermMode::DISAMBIGUATE_ESC_CODES, KeyEventKind::Press),
            b"\x1b[97;6u"
        );

        let mode = TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ALTERNATE_KEYS
            | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(
            encode_key_event(&key, mode, KeyEventKind::Press),
            b"\x1b[97:65;6;65u"
        );
        assert_eq!(
            encode_key_event(&key, mode, KeyEventKind::Repeat),
            b"\x1b[97:65;6:2;65u"
        );
        assert_eq!(
            encode_key_event(&key, mode, KeyEventKind::Release),
            b"\x1b[97:65;6:3u"
        );
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            encode_key_event(
                &keystroke("a", Some("å"), alt),
                TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ALTERNATE_KEYS,
                KeyEventKind::Press,
            ),
            b"\x1b[97;3u",
            "composed text without Shift is not a shifted alternate key"
        );
    }

    #[test]
    fn kitty_function_keys_preserve_protocol_specific_forms() {
        let report_events = TermMode::REPORT_EVENT_TYPES;
        let up = keystroke("up", None, Modifiers::default());
        assert_eq!(
            encode_key_event(&up, report_events, KeyEventKind::Press),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key_event(&up, report_events, KeyEventKind::Repeat),
            b"\x1b[1;1:2A"
        );
        assert_eq!(
            encode_key_event(&up, report_events, KeyEventKind::Release),
            b"\x1b[1;1:3A"
        );
        assert_eq!(
            encode_key_event(
                &keystroke("f13", None, Modifiers::default()),
                TermMode::DISAMBIGUATE_ESC_CODES,
                KeyEventKind::Press,
            ),
            b"\x1b[57376u"
        );
        assert!(encode_key_event(
            &keystroke("enter", Some("\r"), Modifiers::default()),
            report_events,
            KeyEventKind::Release,
        )
        .is_empty());

        let text_mode = TermMode::REPORT_ALL_KEYS_AS_ESC | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(
            encode_text_input("日本", text_mode),
            b"\x1b[0;1;26085:26412u"
        );
        assert_eq!(
            encode_text_input("日本", TermMode::REPORT_ALL_KEYS_AS_ESC),
            "日本".as_bytes()
        );
    }
}
