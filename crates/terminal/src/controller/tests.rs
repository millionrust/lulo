use super::*;

#[test]
fn disabling_mouse_reporting_preserves_terminal_modes_without_forwarding_mouse_events() {
    let requested = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE | TermMode::ALT_SCREEN;
    let allowed = pointer::mouse_reporting_mode(requested, true);
    assert!(allowed.contains(TermMode::MOUSE_DRAG));
    let blocked = pointer::mouse_reporting_mode(requested, false);
    assert!(!blocked.intersects(TermMode::MOUSE_MODE));
    assert!(blocked.contains(TermMode::ALT_SCREEN));
}

#[test]
fn terminal_resources_have_explicit_bounds() {
    assert_eq!(MAX_SEARCH_QUERY_BYTES, 4096);
    assert_eq!(MAX_SEARCH_QUERY_BYTES, 4096);
    assert_eq!(FOCUS_IN_REPORT.len(), 3);
    assert_eq!(FOCUS_OUT_REPORT.len(), 3);
    assert_eq!(MAX_TABS, 16);
    assert_eq!(terminal_content_top(false), 39.0);
    assert_eq!(terminal_content_top(true), 75.0);
}

#[test]
fn focus_reports_follow_the_parsed_xterm_mode() {
    let size = TermSize { cols: 20, lines: 5 };
    let mut term = Term::new(terminal_config(10), &size, EventProxy::default());
    let mut parser: Processor = Processor::new();

    assert_eq!(focus_report(*term.mode(), true), None);
    assert_eq!(focus_report(*term.mode(), false), None);

    parser.advance(&mut term, b"\x1b[?1004h");
    assert!(term.mode().contains(TermMode::FOCUS_IN_OUT));
    assert_eq!(focus_report(*term.mode(), true), Some(FOCUS_IN_REPORT));
    assert_eq!(focus_report(*term.mode(), false), Some(FOCUS_OUT_REPORT));

    parser.advance(&mut term, b"\x1b[?1004l");
    assert!(!term.mode().contains(TermMode::FOCUS_IN_OUT));
    assert_eq!(focus_report(*term.mode(), true), None);
}

#[test]
fn mouse_modes_follow_parsed_xterm_state() {
    let size = TermSize { cols: 20, lines: 5 };
    let mut term = Term::new(terminal_config(10), &size, EventProxy::default());
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"\x1b[?1002;1006h");
    let mode = *term.mode();
    assert!(mode.contains(TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE));
    assert!(!mode.contains(TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_MOTION));

    parser.advance(&mut term, b"\x1b[?1002;1006l");
    assert!(!term
        .mode()
        .intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE));
}

#[test]
fn bracketed_paste_mode_follows_parsed_xterm_state() {
    let size = TermSize { cols: 20, lines: 5 };
    let mut term = Term::new(
        terminal_config(SCROLLBACK_LINES),
        &size,
        EventProxy::default(),
    );
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"\x1b[?2004h");
    assert!(term.mode().contains(TermMode::BRACKETED_PASTE));
    parser.advance(&mut term, b"\x1b[?2004l");
    assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
}

fn test_keystroke(
    key: &str,
    key_char: Option<&str>,
    modifiers: gpui::Modifiers,
) -> gpui::Keystroke {
    gpui::Keystroke {
        key: key.into(),
        key_char: key_char.map(str::to_owned),
        modifiers,
    }
}

#[test]
fn cursor_keys_follow_the_parsed_application_mode() {
    let size = TermSize { cols: 20, lines: 5 };
    let mut term = Term::new(
        terminal_config(SCROLLBACK_LINES),
        &size,
        EventProxy::default(),
    );
    let mut parser: Processor = Processor::new();
    let up = test_keystroke("up", None, gpui::Modifiers::default());
    let home = test_keystroke("home", None, gpui::Modifiers::default());

    assert_eq!(encode_key(&up, *term.mode()), b"\x1b[A");
    assert_eq!(encode_key(&home, *term.mode()), b"\x1b[H");

    parser.advance(&mut term, b"\x1b[?1h");
    assert!(term.mode().contains(TermMode::APP_CURSOR));
    assert_eq!(encode_key(&up, *term.mode()), b"\x1bOA");
    assert_eq!(encode_key(&home, *term.mode()), b"\x1bOH");

    parser.advance(&mut term, b"\x1b[?1l");
    assert!(!term.mode().contains(TermMode::APP_CURSOR));
    assert_eq!(encode_key(&up, *term.mode()), b"\x1b[A");
}

#[test]
fn enhanced_keyboard_protocol_modes_follow_the_parsed_stack() {
    assert!(terminal_config(SCROLLBACK_LINES).kitty_keyboard);
    let size = TermSize { cols: 20, lines: 5 };
    let mut term = Term::new(
        terminal_config(SCROLLBACK_LINES),
        &size,
        EventProxy::default(),
    );
    let mut parser: Processor = Processor::new();

    parser.advance(&mut term, b"\x1b[>1u");
    assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
    parser.advance(&mut term, b"\x1b[>31u");
    assert!(term.mode().contains(TermMode::KITTY_KEYBOARD_PROTOCOL));
    parser.advance(&mut term, b"\x1b[<u");
    assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
    assert!(!term.mode().contains(
        TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ALTERNATE_KEYS
            | TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_ASSOCIATED_TEXT
    ));
    parser.advance(&mut term, b"\x1b[<u");
    assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
}

#[test]
fn tab_and_shift_tab_reach_the_shell_inside_the_terminal() {
    gpui::actions!(terminal_test, [WindowFocusNext, WindowFocusPrevious]);
    let mut bindings = vec![
        KeyBinding::new("tab", WindowFocusNext, Some("Root")),
        KeyBinding::new("shift-tab", WindowFocusPrevious, Some("Root")),
    ];
    bindings.extend(super::input::shell_owned_key_bindings());
    let keymap = gpui::Keymap::new(bindings);
    let root = gpui::KeyContext::parse("Root").unwrap();
    let terminal = gpui::KeyContext::parse("Terminal").unwrap();
    for key in ["tab", "shift-tab"] {
        let keystroke = gpui::Keystroke::parse(key).unwrap();
        let (inside, pending) = keymap.bindings_for_input(
            std::slice::from_ref(&keystroke),
            &[root.clone(), terminal.clone()],
        );
        assert!(
            inside.is_empty() && !pending,
            "{key} must fall through to the grid"
        );
        let (outside, _) = keymap.bindings_for_input(&[keystroke], std::slice::from_ref(&root));
        assert_eq!(
            outside.len(),
            1,
            "{key} still moves focus outside the terminal"
        );
    }
}
