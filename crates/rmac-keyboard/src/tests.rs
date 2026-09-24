use rmac_locale::X11Keyboard;

use crate::*;

fn keyboard(layout: &str, variant: &str, options: &str) -> X11Keyboard {
    X11Keyboard {
        layout: layout.into(),
        model: "pc105".into(),
        variant: variant.into(),
        options: options.into(),
    }
}

fn target(shortcuts: bool, swap: bool, caps: CapsLockAction, option: bool) -> MacKeyboard {
    MacKeyboard {
        shortcuts_in_all_apps: shortcuts,
        layout: PhysicalLayout {
            swap_command_option: swap,
            caps_lock: caps,
            option_characters: option,
        },
    }
}

#[test]
fn keyd_config_matches_the_reviewed_file_for_the_reference_laptop() {
    // Swap on, Caps Lock as Control, no ⌥ characters.
    let config = keyd_config(&PhysicalLayout {
        swap_command_option: true,
        caps_lock: CapsLockAction::Control,
        option_characters: false,
    });
    let expected = "\
# rmac-mac-keyboard 1 swap=on caps=control option-characters=off
# Written by rmac System Settings > Keyboard; changes here are replaced.
# rmac-mac-keyboard follow fills the cmd and opt layers for the focused app.

[ids]
*

[main]
leftalt = layer(cmd)
rightalt = layer(cmd)
leftmeta = layer(opt)
rightmeta = layer(opt)
capslock = layer(control)

[cmd:M]

[opt:A]

[cmd+control]
a = C-M-a
b = C-M-b
c = C-M-c
f = C-M-f
g = C-M-g
i = C-M-i
k = C-M-k
l = C-M-l
n = C-M-n
o = C-M-o
p = C-M-p
q = C-M-q
r = C-M-r
s = C-M-s
t = C-M-t
u = C-M-u
v = C-M-v
w = C-M-w
x = C-M-x
z = C-M-z
0 = C-M-0
equal = C-M-equal
minus = C-M-minus
leftbrace = C-M-leftbrace
rightbrace = C-M-rightbrace
left = C-M-left
right = C-M-right
up = C-M-up
down = C-M-down
";
    assert_eq!(config, expected);
}

#[test]
fn keyd_config_without_swap_keeps_command_on_the_windows_key() {
    let config = keyd_config(&PhysicalLayout::default());
    assert!(config.contains("leftmeta = layer(cmd)\nrightmeta = layer(cmd)\n"));
    assert!(config.contains("leftalt = layer(opt)\nrightalt = layer(opt)\n"));
    assert!(!config.contains("capslock ="));
}

#[test]
fn every_caps_lock_choice_has_a_keyd_action() {
    let expect = [
        (CapsLockAction::CapsLock, None),
        (CapsLockAction::Control, Some("capslock = layer(control)")),
        (CapsLockAction::Option, Some("capslock = layer(opt)")),
        (CapsLockAction::Command, Some("capslock = layer(cmd)")),
        (CapsLockAction::Escape, Some("capslock = esc")),
        (CapsLockAction::NoAction, Some("capslock = noop")),
    ];
    for (action, line) in expect {
        let config = keyd_config(&PhysicalLayout {
            caps_lock: action,
            ..PhysicalLayout::default()
        });
        match line {
            Some(line) => assert!(config.contains(line), "{action:?}"),
            None => assert!(!config.contains("capslock"), "{action:?}"),
        }
    }
}

#[test]
fn option_characters_use_altgr_but_keep_alt_arrows_for_rmac_apps() {
    let config = keyd_config(&PhysicalLayout {
        option_characters: true,
        ..PhysicalLayout::default()
    });
    assert!(config.contains("[opt:G]\nleft = A-left\nright = A-right\n"));
    assert!(config.contains("backspace = A-backspace\ndelete = A-delete\n"));
    assert!(!config.contains("[opt:A]"));
}

#[test]
fn keyd_header_round_trips_every_layout() {
    for swap in [false, true] {
        for option in [false, true] {
            for caps in CapsLockAction::ALL {
                let layout = PhysicalLayout {
                    swap_command_option: swap,
                    caps_lock: caps,
                    option_characters: option,
                };
                assert_eq!(parse_keyd_header(&keyd_config(&layout)), Some(layout));
            }
        }
    }
}

#[test]
fn foreign_or_damaged_keyd_files_are_not_mistaken_for_rmac_files() {
    assert_eq!(parse_keyd_header("[ids]\n*\n"), None);
    assert_eq!(
        parse_keyd_header("# rmac-mac-keyboard 2 swap=on caps=control option-characters=on\n"),
        None
    );
    assert_eq!(
        parse_keyd_header("# rmac-mac-keyboard 1 swap=on caps=hyper option-characters=on\n"),
        None
    );
    assert_eq!(
        parse_keyd_header("# rmac-mac-keyboard 1 swap=on caps=control\n"),
        None
    );
    assert_eq!(
        parse_keyd_header(
            "# rmac-mac-keyboard 1 swap=on swap=off caps=control option-characters=on\n"
        ),
        None
    );
}

#[test]
fn pc_app_profile_translates_the_mac_shortcuts_to_control() {
    let arguments = bind_arguments(Profile::PcApp);
    assert_eq!(arguments[0], "reset");
    for (key, chord) in [
        ("c", "C-c"),
        ("v", "C-v"),
        ("x", "C-x"),
        ("z", "C-z"),
        ("a", "C-a"),
        ("s", "C-s"),
        ("w", "C-w"),
        ("q", "C-q"),
        ("t", "C-t"),
        ("f", "C-f"),
        ("left", "home"),
        ("right", "end"),
    ] {
        assert!(
            arguments.contains(&format!("cmd.{key} = {chord}")),
            "⌘{key} should send {chord}"
        );
    }
    assert!(arguments.contains(&"opt.left = C-left".to_owned()));
    assert!(arguments.contains(&"opt.right = C-right".to_owned()));
}

#[test]
fn terminal_profile_keeps_control_c_for_interrupt() {
    let arguments = bind_arguments(Profile::Terminal);
    assert!(arguments.contains(&"cmd.c = C-S-c".to_owned()));
    assert!(arguments.contains(&"cmd.v = C-S-v".to_owned()));
    assert!(arguments.contains(&"opt.left = A-b".to_owned()));
    assert!(arguments.contains(&"opt.right = A-f".to_owned()));
    // Nothing may ever rebind Control itself.
    assert!(!arguments
        .iter()
        .any(|binding| binding.starts_with("control.")));
    assert!(!arguments.iter().any(|binding| binding.ends_with("= C-c")));
}

#[test]
fn native_profile_is_the_installed_pass_through_file() {
    assert_eq!(bind_arguments(Profile::Native), vec!["reset".to_owned()]);
}

#[test]
fn no_profile_translates_a_key_niri_binds_under_command() {
    // packaging/rmac-session/shell.kdl and shortcuts-fallback.kdl bind
    // Mod+Space, Mod+Tab, Mod+grave, Mod+Shift+3/4/5 and Mod+Alt+Escape.
    for profile in [Profile::PcApp, Profile::Terminal] {
        for key in ["space", "tab", "grave", "3", "4", "5", "h", "m", "esc"] {
            assert!(
                !bind_arguments(profile)
                    .iter()
                    .any(|binding| binding.starts_with(&format!("cmd.{key} "))),
                "{profile:?} translates ⌘{key}"
            );
        }
    }
}

#[test]
fn every_translated_command_key_passes_through_with_control_for_niri() {
    let config = keyd_config(&PhysicalLayout::default());
    let composite = config.split("[cmd+control]\n").nth(1).unwrap();
    for profile in [Profile::PcApp, Profile::Terminal] {
        for binding in bind_arguments(profile) {
            if let Some(rest) = binding.strip_prefix("cmd.") {
                let key = rest.split(" = ").next().unwrap();
                assert!(
                    composite.contains(&format!("{key} = C-M-{key}\n")),
                    "⌃⌘{key} would be translated"
                );
            }
        }
    }
}

#[test]
fn bindings_are_well_formed_keyd_expressions() {
    for profile in [Profile::Native, Profile::PcApp, Profile::Terminal] {
        for binding in bind_arguments(profile).into_iter().skip(1) {
            let (left, right) = binding.split_once(" = ").expect("binding has =");
            let (layer, key) = left.split_once('.').expect("binding names a layer");
            assert!(matches!(layer, "cmd" | "opt"), "{binding}");
            assert!(!key.is_empty() && !right.is_empty(), "{binding}");
            assert!(!right.contains(' '), "{binding}");
        }
    }
}

#[test]
fn focused_apps_map_to_profiles() {
    assert_eq!(profile_for_app(None), Profile::Native);
    assert_eq!(profile_for_app(Some("")), Profile::Native);
    assert_eq!(profile_for_app(Some("org.rmac.Terminal")), Profile::Native);
    assert_eq!(profile_for_app(Some("org.rmac.Files")), Profile::Native);
    assert_eq!(profile_for_app(Some("firefox")), Profile::PcApp);
    assert_eq!(profile_for_app(Some("code")), Profile::PcApp);
    assert_eq!(profile_for_app(Some("org.gnome.Nautilus")), Profile::PcApp);
    assert_eq!(profile_for_app(Some("org.gnome.Ptyxis")), Profile::Terminal);
    assert_eq!(profile_for_app(Some("Alacritty")), Profile::Terminal);
    assert_eq!(
        profile_for_app(Some("com.mitchellh.ghostty")),
        Profile::Terminal
    );
    assert_eq!(profile_for_app(Some("XTerm")), Profile::Terminal);
}

#[test]
fn xkb_mode_adds_swap_and_caps_and_keeps_user_options() {
    let current = keyboard("us,de", "", "grp:alt_shift_toggle,compose:ralt");
    let next = xkb_for(
        &current,
        &target(false, true, CapsLockAction::Control, false),
    );
    assert_eq!(next.layout, "us,de");
    assert_eq!(next.model, "pc105");
    assert_eq!(next.variant, "");
    assert_eq!(
        next.options,
        "grp:alt_shift_toggle,compose:ralt,altwin:swap_alt_win,ctrl:nocaps"
    );
}

#[test]
fn keyd_mode_removes_the_xkb_copies_so_keys_are_not_swapped_twice() {
    let current = keyboard("us", "", "altwin:swap_alt_win,caps:escape,compose:menu");
    let next = xkb_for(&current, &target(true, true, CapsLockAction::Escape, false));
    assert_eq!(next.options, "compose:menu");
}

#[test]
fn option_characters_switch_to_the_mac_variant_and_back() {
    let current = keyboard("gb,de", ",nodeadkeys", "grp:win_space_toggle");
    let on = xkb_for(
        &current,
        &target(false, false, CapsLockAction::CapsLock, true),
    );
    assert_eq!(on.variant, "mac,mac_nodeadkeys");
    let off = xkb_for(&on, &target(false, false, CapsLockAction::CapsLock, false));
    assert_eq!(off.variant, ",nodeadkeys");
    let plain = xkb_for(
        &keyboard("us", "mac", ""),
        &target(false, false, CapsLockAction::CapsLock, false),
    );
    assert_eq!(plain.variant, "");
}

#[test]
fn swap_keeps_right_alt_as_option_on_mac_layouts() {
    let next = xkb_for(
        &keyboard("us", "", ""),
        &target(false, true, CapsLockAction::CapsLock, true),
    );
    assert_eq!(next.variant, "mac");
    assert_eq!(next.options, "altwin:swap_lalt_lwin");
    // A layout without a Mac variant swaps both sides.
    let next = xkb_for(
        &keyboard("in", "", ""),
        &target(false, true, CapsLockAction::CapsLock, true),
    );
    assert_eq!(next.variant, "");
    assert_eq!(next.options, "altwin:swap_alt_win");
}

#[test]
fn every_caps_choice_round_trips_through_xkb() {
    for caps in CapsLockAction::ALL {
        for swap in [false, true] {
            let wanted = target(false, swap, caps, false);
            let applied = xkb_for(&keyboard("us", "", "compose:menu"), &wanted);
            assert_eq!(detect(None, &applied), wanted, "{caps:?} swap={swap}");
            // Applying again changes nothing.
            assert_eq!(xkb_for(&applied, &wanted), applied);
        }
    }
}

#[test]
fn detect_prefers_the_keyd_file_while_shortcuts_are_on() {
    let layout = PhysicalLayout {
        swap_command_option: true,
        caps_lock: CapsLockAction::Command,
        option_characters: true,
    };
    let state = detect(Some(&keyd_config(&layout)), &keyboard("us", "mac", ""));
    assert!(state.shortcuts_in_all_apps);
    assert_eq!(state.layout, layout);
    let state = detect(Some("[ids]\n*\n"), &keyboard("us", "", "caps:none"));
    assert!(!state.shortcuts_in_all_apps);
    assert_eq!(state.layout.caps_lock, CapsLockAction::NoAction);
}

#[test]
fn mac_variants_come_from_xkeyboard_config() {
    assert_eq!(mac_variant("us", ""), Some("mac"));
    assert_eq!(mac_variant("ch", ""), Some("de_mac"));
    assert_eq!(mac_variant("ua", ""), Some("macOS"));
    assert_eq!(mac_variant("in", ""), None);
    assert!(is_mac_variant("us", "mac-iso"));
    assert!(!is_mac_variant("us", "dvorak"));
    assert!(supports_option_characters(&keyboard(
        "de",
        "nodeadkeys",
        ""
    )));
    assert!(!supports_option_characters(&keyboard("in", "", "")));
}

#[test]
fn helper_arguments_round_trip_and_reject_anything_else() {
    for shortcuts in [false, true] {
        for caps in CapsLockAction::ALL {
            let wanted = target(shortcuts, !shortcuts, caps, shortcuts);
            assert_eq!(
                parse_helper_arguments(&helper_arguments(&wanted)[1..]),
                Some(wanted)
            );
        }
    }
    let bad = |values: &[&str]| {
        parse_helper_arguments(&values.iter().map(|v| v.to_string()).collect::<Vec<_>>())
    };
    assert_eq!(
        bad(&[
            "--shortcuts",
            "yes",
            "--swap",
            "on",
            "--caps",
            "control",
            "--option-characters",
            "on"
        ]),
        None
    );
    assert_eq!(
        bad(&[
            "--shortcuts",
            "on",
            "--swap",
            "on",
            "--caps",
            "/etc/shadow",
            "--option-characters",
            "on"
        ]),
        None
    );
    assert_eq!(bad(&["--shortcuts", "on"]), None);
}

#[test]
fn group_ids_are_read_from_etc_group() {
    let source = "root:x:0:\nkeydx:x:5:\nkeyd:x:996:jake\n";
    assert_eq!(parse_group_id(source, "keyd"), Some(996));
    assert_eq!(parse_group_id(source, "input"), None);
}

#[test]
fn group_members_are_read_from_etc_group() {
    let source = "root:x:0:\nkeydx:x:5:eve\nkeyd:x:996:jake,ana\ninput:x:104:\n";
    assert_eq!(parse_group_members(source, "keyd"), vec!["jake", "ana"]);
    assert!(parse_group_members(source, "input").is_empty());
    assert!(parse_group_members(source, "missing").is_empty());
}

#[test]
fn the_relay_accepts_only_an_exact_profile_name() {
    for profile in [Profile::Native, Profile::PcApp, Profile::Terminal] {
        let request = format!("{}\n", profile.id());
        assert_eq!(parse_relay_request(request.as_bytes()), Some(profile));
    }
    for request in [
        &b""[..],
        b"native",
        b"native\n\n",
        b" native\n",
        b"Native\n",
        b"reset\n",
        b"cmd.c = command(id)\n",
        b"pc-app\nterminal\n",
        b"\xff\n",
    ] {
        assert_eq!(parse_relay_request(request), None, "{request:?}");
    }
    let long = format!("{}\n", "a".repeat(RELAY_REQUEST_MAX));
    assert_eq!(parse_relay_request(long.as_bytes()), None);
}

#[test]
fn no_profile_binding_can_run_a_command() {
    // keyd runs `command()` bindings as root; the relay must never send one.
    for profile in [Profile::Native, Profile::PcApp, Profile::Terminal] {
        for argument in bind_arguments(profile) {
            assert!(!argument.contains("command("), "{argument}");
        }
    }
}

#[test]
fn pkexec_denial_is_not_reported_as_cancellation() {
    assert_eq!(
        command_failure(HELPER_LABEL, b"", Some(126)),
        "authentication was cancelled"
    );
    let denied = command_failure(HELPER_LABEL, b"Not authorized", Some(127));
    assert!(denied.contains("not authorised"), "{denied}");
    assert!(!denied.contains("cancelled"));
    assert_eq!(
        command_failure("keyd bind", b"boom\nlast line\n", Some(127)),
        "keyd bind failed: last line"
    );
    assert_eq!(
        command_failure("keyd bind", b"", Some(3)),
        "keyd bind exited with status 3"
    );
}
