use super::*;
use crate::api::{
    set_text_scale_with, snapshot_with, CommandOutput, Runner, MAX_ERROR_BYTES, SCHEMA,
    TEXT_SCALE_KEY,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

struct FakeRunner {
    outputs: RefCell<VecDeque<io::Result<CommandOutput>>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn new(outputs: Vec<io::Result<CommandOutput>>) -> Self {
        Self {
            outputs: RefCell::new(outputs.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl Runner for FakeRunner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(arguments.iter().map(|value| (*value).to_string()).collect());
        self.outputs
            .borrow_mut()
            .pop_front()
            .expect("unexpected gsettings call")
    }
}

fn success(stdout: &str) -> io::Result<CommandOutput> {
    Ok(CommandOutput {
        success: true,
        stdout: stdout.to_string(),
    })
}

#[test]
fn missing_gsettings_is_an_unavailable_snapshot() {
    let runner = FakeRunner::new(vec![Err(io::Error::new(
        io::ErrorKind::NotFound,
        "missing",
    ))]);
    let snapshot = snapshot_with(&runner).unwrap();
    assert!(!snapshot.available);
    assert!(!snapshot.writable);
    assert!(snapshot.detail.unwrap().contains("not installed"));
}

#[test]
fn reads_a_policy_locked_factor() {
    let runner = FakeRunner::new(vec![success("1.2\n"), success("false\n")]);
    let snapshot = snapshot_with(&runner).unwrap();
    assert!(snapshot.available);
    assert!(!snapshot.writable);
    assert_eq!(snapshot.factor, 1.2);
    assert!(snapshot.detail.unwrap().contains("locked"));
}

#[test]
fn rejects_invalid_requested_factors_before_running_a_command() {
    let runner = FakeRunner::new(Vec::new());
    assert!(set_text_scale_with(&runner, 0.9).is_err());
    assert!(set_text_scale_with(&runner, f64::NAN).is_err());
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn sets_and_confirms_the_authoritative_value() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.0\n"),
        success("true\n"),
        success(""),
        success("1.3\n"),
        success("true\n"),
    ]);
    let snapshot = set_text_scale_with(&runner, 1.3).unwrap();
    assert_eq!(snapshot.factor, 1.3);
    let calls = runner.calls.borrow();
    assert_eq!(calls[4], ["set", SCHEMA, TEXT_SCALE_KEY, "1.30"]);
}

#[test]
fn rejects_a_readback_mismatch() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.0\n"),
        success("true\n"),
        success(""),
        success("1.2\n"),
        success("true\n"),
    ]);
    let error = set_text_scale_with(&runner, 1.3).unwrap_err();
    assert!(error.to_string().contains("authority reports 1.20"));
}

#[test]
fn refuses_a_value_or_policy_changed_during_preflight() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.1\n"),
        success("true\n"),
    ]);
    let error = set_text_scale_with(&runner, 1.3).unwrap_err();
    assert!(error.to_string().contains("changed before save"));
    assert_eq!(runner.calls.borrow().len(), 4);
}

#[test]
fn a_matching_value_is_a_noop() {
    let runner = FakeRunner::new(vec![success("1.3\n"), success("true\n")]);
    assert_eq!(set_text_scale_with(&runner, 1.3).unwrap().factor, 1.3);
    assert_eq!(runner.calls.borrow().len(), 2);
}

#[test]
fn errors_are_bounded_and_control_normalized() {
    let error = Error::new("test", format!("{}\nprivate", "x".repeat(600)));
    assert!(error.to_string().len() <= MAX_ERROR_BYTES + "test: ".len());
    assert!(!error.to_string().contains('\n'));
}

mod toolkit {
    use super::{success, FakeRunner};
    use crate::toolkit::{
        accent_rgb, gnome_accent, gtk3_settings, gtk4_stylesheet, merge_settings_ini, sync_with,
    };
    use rmac_theme::{AccentPreference, Preferences, SchemePreference};

    fn scratch(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("rmac-gtk-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn merging_replaces_only_rmac_keys_and_keeps_the_rest() {
        let existing = "# mine\n[Settings]\ngtk-icon-theme-name=Papirus\ngtk-theme-name=Yaru\ngtk-theme-name=Old\n\n[Other]\nkey=value\n";
        let merged = merge_settings_ini(existing, &gtk3_settings(true));
        assert!(merged
            .starts_with("# mine\n[Settings]\ngtk-icon-theme-name=Papirus\ngtk-theme-name=rmac\n"));
        assert_eq!(merged.matches("gtk-theme-name=").count(), 1);
        assert!(merged.contains("gtk-application-prefer-dark-theme=true\n"));
        assert!(merged.contains("gtk-decoration-layout=close,minimize,maximize:\n"));
        assert!(merged.contains("gtk-font-name=Inter 9.75\n"));
        assert!(merged.ends_with("[Other]\nkey=value\n"));
        // Merging again changes nothing.
        assert_eq!(merge_settings_ini(&merged, &gtk3_settings(true)), merged);
    }

    #[test]
    fn merging_adds_a_settings_group_when_missing() {
        let merged = merge_settings_ini("", &gtk3_settings(false));
        assert!(merged.starts_with("[Settings]\ngtk-theme-name=rmac\n"));
        assert!(merged.contains("gtk-application-prefer-dark-theme=false\n"));
    }

    #[test]
    fn every_appearance_swatch_maps_to_its_gnome_accent() {
        for (hex, name) in [
            (0x1372f9_u32, "blue"),
            (0xaf52de, "purple"),
            (0xff2d55, "pink"),
            (0xff3b30, "red"),
            (0xff9500, "orange"),
            (0xffcc00, "yellow"),
            (0x34c759, "green"),
            (0x8e8e93, "slate"),
        ] {
            let rgb = accent_rgb(AccentPreference::Custom([
                f64::from((hex >> 16) & 0xff) / 255.0,
                f64::from((hex >> 8) & 0xff) / 255.0,
                f64::from(hex & 0xff) / 255.0,
            ]));
            assert_eq!(gnome_accent(rgb), name, "{hex:06x}");
        }
        assert_eq!(accent_rgb(AccentPreference::Automatic), [0x13, 0x72, 0xf9]);
    }

    #[test]
    fn dark_preference_writes_the_scheme_files_and_stubs() {
        let home = scratch("dark");
        let runner = FakeRunner::new(vec![
            success("'default'\n"),
            success(""),
            success("'blue'\n"),
        ]);
        let preferences = Preferences {
            color_scheme: SchemePreference::Dark,
            ..Preferences::default()
        };
        let applied = sync_with(&runner, &preferences, &home, true).unwrap();
        assert!(applied.dark);
        let calls = runner.calls.borrow();
        assert_eq!(
            calls[1],
            [
                "set",
                "org.gnome.desktop.interface",
                "color-scheme",
                "'prefer-dark'"
            ]
        );
        assert_eq!(calls.len(), 3);
        let gtk3 = std::fs::read_to_string(home.join("gtk-3.0/settings.ini")).unwrap();
        assert!(gtk3.contains("gtk-application-prefer-dark-theme=true\n"));
        let gtk4 = std::fs::read_to_string(home.join("gtk-4.0/settings.ini")).unwrap();
        assert!(!gtk4.contains("prefer-dark"));
        assert!(gtk4.contains("gtk-overlay-scrolling=true\n"));
        let stub = std::fs::read_to_string(home.join("gtk-4.0/gtk.css")).unwrap();
        assert_eq!(stub, gtk4_stylesheet([0x13, 0x72, 0xf9]));
        assert!(stub.contains("--accent-bg-color: #1372F9;"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn automatic_follows_the_host_and_never_writes_the_scheme() {
        let home = scratch("automatic");
        let runner = FakeRunner::new(vec![success("'prefer-dark'\n"), success("'blue'\n")]);
        let preferences = Preferences {
            color_scheme: SchemePreference::Automatic,
            ..Preferences::default()
        };
        let applied = sync_with(&runner, &preferences, &home, false).unwrap();
        assert!(applied.dark);
        assert!(runner.calls.borrow().iter().all(|call| call[0] == "get"));
        assert!(!home.join("gtk-4.0/gtk.css").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_stylesheet_the_user_owns_is_left_alone() {
        let home = scratch("owned");
        std::fs::create_dir_all(home.join("gtk-4.0")).unwrap();
        std::fs::write(home.join("gtk-4.0/gtk.css"), "window { color: red; }\n").unwrap();
        let runner = FakeRunner::new(vec![success("'prefer-light'\n"), success("'blue'\n")]);
        let preferences = Preferences {
            color_scheme: SchemePreference::Light,
            ..Preferences::default()
        };
        sync_with(&runner, &preferences, &home, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("gtk-4.0/gtk.css")).unwrap(),
            "window { color: red; }\n"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
