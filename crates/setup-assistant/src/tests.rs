use std::ffi::OsString;
use std::path::PathBuf;

use rmac_keyboard::{MacKeyboard, Status};

use crate::flow::{Availability, Completion, Event, Flow, Outcome, Step};
use crate::{greeting, mac_shortcuts, marker, names};

fn everything() -> Availability {
    Availability {
        wifi_needed: true,
        accounts: true,
    }
}

#[test]
fn a_full_run_visits_every_page_in_order_and_finishes() {
    let mut flow = Flow::new(everything());
    assert_eq!(flow.steps(), Step::ALL);
    assert_eq!(flow.step(), Step::Welcome);
    for expected in &Step::ALL[1..] {
        assert_eq!(flow.handle(Event::Continue), Outcome::Moved(*expected));
    }
    assert_eq!(
        flow.handle(Event::Continue),
        Outcome::Ended(Completion::Finished)
    );
    assert_eq!(flow.ended(), Some(Completion::Finished));
    // Nothing moves a finished flow.
    assert_eq!(flow.handle(Event::Back), Outcome::Ignored);
    assert_eq!(flow.handle(Event::Close), Outcome::Ignored);
}

#[test]
fn pages_without_a_backend_are_left_out() {
    let flow = Flow::new(Availability {
        wifi_needed: false,
        accounts: false,
    });
    assert!(!flow.steps().contains(&Step::WiFi));
    assert!(!flow.steps().contains(&Step::Account));
    assert_eq!(flow.steps().len(), Step::ALL.len() - 2);
    let flow = Flow::new(Availability {
        wifi_needed: true,
        accounts: false,
    });
    assert!(flow.steps().contains(&Step::WiFi));
    assert!(!flow.steps().contains(&Step::Account));
}

#[test]
fn back_returns_to_the_previous_page_and_stops_at_welcome() {
    let mut flow = Flow::new(everything());
    assert!(!flow.can_go_back());
    assert_eq!(flow.handle(Event::Back), Outcome::Ignored);
    flow.handle(Event::Continue);
    flow.handle(Event::Continue);
    assert_eq!(flow.step(), Step::Keyboard);
    assert!(flow.can_go_back());
    assert_eq!(
        flow.handle(Event::Back),
        Outcome::Moved(Step::LanguageRegion)
    );
    assert_eq!(flow.handle(Event::Back), Outcome::Moved(Step::Welcome));
    assert_eq!(flow.handle(Event::Back), Outcome::Ignored);
}

#[test]
fn set_up_later_moves_on_only_from_pages_that_change_settings() {
    let mut flow = Flow::new(everything());
    assert_eq!(flow.handle(Event::SetUpLater), Outcome::Ignored);
    flow.handle(Event::Continue);
    assert_eq!(
        flow.handle(Event::SetUpLater),
        Outcome::Moved(Step::Keyboard)
    );
    assert_eq!(
        flow.handle(Event::SetUpLater),
        Outcome::Moved(Step::MacShortcuts)
    );
    assert_eq!(flow.handle(Event::SetUpLater), Outcome::Moved(Step::WiFi));
    assert_eq!(
        flow.postponed(),
        [Step::LanguageRegion, Step::Keyboard, Step::MacShortcuts]
    );
    // Coming back and continuing clears the postponement.
    flow.handle(Event::Back);
    assert_eq!(flow.handle(Event::Continue), Outcome::Moved(Step::WiFi));
    assert_eq!(flow.postponed(), [Step::LanguageRegion, Step::Keyboard]);
    // Postponing twice records the page once.
    flow.handle(Event::Back);
    flow.handle(Event::Back);
    flow.handle(Event::SetUpLater);
    assert_eq!(flow.postponed(), [Step::LanguageRegion, Step::Keyboard]);
    // Tips and Privacy change nothing, so they have no Set Up Later.
    while flow.step() != Step::Tips {
        flow.handle(Event::Continue);
    }
    assert_eq!(flow.handle(Event::SetUpLater), Outcome::Ignored);
}

#[test]
fn skip_setup_is_offered_only_on_welcome_and_ends_the_flow() {
    let mut flow = Flow::new(everything());
    flow.handle(Event::Continue);
    assert_eq!(flow.handle(Event::SkipSetup), Outcome::Ignored);
    flow.handle(Event::Back);
    assert_eq!(
        flow.handle(Event::SkipSetup),
        Outcome::Ended(Completion::Skipped)
    );
}

#[test]
fn closing_the_window_counts_as_skipped_unless_on_the_last_page() {
    let mut flow = Flow::new(everything());
    flow.handle(Event::Continue);
    assert_eq!(
        flow.handle(Event::Close),
        Outcome::Ended(Completion::Skipped)
    );

    let mut flow = Flow::new(Availability::default());
    while flow.step() != Step::Done {
        flow.handle(Event::Continue);
    }
    assert_eq!(
        flow.handle(Event::Close),
        Outcome::Ended(Completion::Finished)
    );
}

#[test]
fn every_page_has_a_title_and_only_setting_pages_can_wait() {
    for step in Step::ALL {
        assert!(!step.title().is_empty());
    }
    assert!(!Step::Welcome.can_set_up_later());
    assert!(!Step::Done.can_set_up_later());
    assert!(Step::WiFi.can_set_up_later());
    assert!(Step::MacShortcuts.can_set_up_later());
}

#[test]
fn greeting_fades_in_rises_holds_and_fades_out() {
    let start = greeting::frame(0, true);
    assert_eq!(start.word, 0);
    assert_eq!(start.opacity, 0.0);
    assert_eq!(start.offset, greeting::RISE);
    let risen = greeting::frame(greeting::FADE_MS, true);
    assert_eq!((risen.opacity, risen.offset), (1.0, 0.0));
    let holding = greeting::frame(greeting::WORD_MS / 2, true);
    assert_eq!(holding.opacity, 1.0);
    let leaving = greeting::frame(greeting::WORD_MS - greeting::FADE_MS / 2, true);
    assert!(leaving.opacity > 0.4 && leaving.opacity < 0.6);
    assert_eq!(greeting::frame(greeting::WORD_MS, true).word, 1);
    // The loop wraps after the last word.
    let lap = greeting::WORD_MS * greeting::WORDS.len() as u64;
    assert_eq!(greeting::frame(lap, true).word, 0);
    // Reduce Motion keeps the crossfade and drops the rise.
    assert_eq!(greeting::frame(100, false).offset, 0.0);
    assert!(greeting::frame(100, false).opacity > 0.0);
}

#[test]
fn greeting_colours_walk_the_gradient() {
    assert_eq!(greeting::color(0), 0x6FB6FF);
    assert_eq!(greeting::color(greeting::WORDS.len() - 1), 0xFF8FB1);
    assert_ne!(greeting::color(3), greeting::color(4));
}

#[test]
fn marker_follows_xdg_config_home_then_home() {
    assert_eq!(
        marker::marker_path(
            Some(OsString::from("/cfg")),
            Some(OsString::from("/home/a"))
        ),
        Some(PathBuf::from("/cfg/rmac/setup-assistant-complete"))
    );
    // A relative XDG_CONFIG_HOME is ignored, as the XDG spec requires.
    assert_eq!(
        marker::marker_path(Some(OsString::from("cfg")), Some(OsString::from("/home/a"))),
        Some(PathBuf::from(
            "/home/a/.config/rmac/setup-assistant-complete"
        ))
    );
    assert_eq!(marker::marker_path(None, None), None);
    assert_eq!(marker::marker_contents(Completion::Skipped), "skipped\n");
    assert_eq!(marker::marker_contents(Completion::Finished), "finished\n");
}

#[test]
fn installed_locales_become_named_choices() {
    let installed = [
        "C.UTF-8",
        "POSIX",
        "en_IN",
        "en_IN.UTF-8",
        "en_GB.utf8",
        "de_DE.UTF-8",
        "ja_JP.UTF-8",
        "xx_QQ.UTF-8",
    ]
    .map(String::from);
    let choices = names::choices(&installed);
    let codes = choices.iter().map(|c| c.code.as_str()).collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            "de_DE.UTF-8",
            "en_IN.UTF-8",
            "en_GB.utf8",
            "xx_QQ.UTF-8",
            "ja_JP.UTF-8"
        ]
    );
    let india = &choices[1];
    assert_eq!(india.language, "English (India)");
    assert_eq!(india.region, "India");
    assert_eq!(choices[0].language, "Deutsch (Germany)");
    assert_eq!(choices[4].language, "日本語 (Japan)");
    // Unknown codes stay readable instead of disappearing.
    assert_eq!(choices[3].language, "xx (QQ)");
    assert!(names::same_locale("en_GB.utf8", "en_GB.UTF-8"));
    assert!(!names::same_locale("en_GB.UTF-8", "en_IN.UTF-8"));
}

#[test]
fn monograms_use_first_and_last_names() {
    assert_eq!(names::monogram("Alex Example", "alex"), "AE");
    assert_eq!(names::monogram("Mary Ann van Dyke", "mary"), "MD");
    assert_eq!(names::monogram("Cher", "cher"), "C");
    assert_eq!(names::monogram("", "jake"), "J");
    assert_eq!(names::monogram("  ", ""), "");
}

/// A `Status` as `rmac_keyboard::status()` would report it, with Mac
/// shortcuts off and keyd fully available.
fn keyboard_status() -> Status {
    Status {
        state: MacKeyboard::default(),
        keyboard: Default::default(),
        keyd_installed: true,
        helper_installed: true,
        relay_available: false,
        foreign_keyd_configs: Vec::new(),
        option_characters_available: false,
    }
}

#[test]
fn mac_shortcuts_are_available_only_when_keyd_is_clear_to_use() {
    let status = keyboard_status();
    assert!(mac_shortcuts::available(&status));
    assert!(mac_shortcuts::unavailable_reason(&status).is_none());

    let mut missing = status.clone();
    missing.keyd_installed = false;
    assert!(!mac_shortcuts::available(&missing));
    assert!(mac_shortcuts::unavailable_reason(&missing)
        .unwrap()
        .contains("keyd package"));

    let mut foreign = status.clone();
    foreign.foreign_keyd_configs = vec!["other.conf".into()];
    assert!(!mac_shortcuts::available(&foreign));
    assert!(mac_shortcuts::unavailable_reason(&foreign)
        .unwrap()
        .contains("another configuration"));

    let mut unpackaged = status;
    unpackaged.helper_installed = false;
    assert!(!mac_shortcuts::available(&unpackaged));
    assert!(mac_shortcuts::unavailable_reason(&unpackaged)
        .unwrap()
        .contains("installed from its package"));
}

#[test]
fn mac_shortcuts_target_is_none_when_nothing_needs_to_change() {
    let status = keyboard_status();
    // Turning an already-off toggle off again applies nothing.
    assert_eq!(mac_shortcuts::target(&status, false), None);
    // Turning it on computes a target with the flag set.
    let target = mac_shortcuts::target(&status, true).expect("a change to apply");
    assert!(target.shortcuts_in_all_apps);
    assert_eq!(target.layout, status.state.layout);

    let mut already_on = status.clone();
    already_on.state.shortcuts_in_all_apps = true;
    assert_eq!(mac_shortcuts::target(&already_on, true), None);
    assert!(mac_shortcuts::target(&already_on, false).is_some());
}

#[test]
fn mac_shortcuts_target_never_asks_to_turn_on_when_unavailable() {
    let mut status = keyboard_status();
    status.keyd_installed = false;
    // The toggle default is on, but keyd cannot be used, so wanting it on
    // must not produce a target that would fail (and would have prompted
    // for a password for nothing).
    assert_eq!(mac_shortcuts::target(&status, true), None);
}

#[test]
fn mac_shortcuts_declined_note_explains_without_blocking_setup() {
    let note = mac_shortcuts::declined_note(&"authentication was cancelled");
    assert!(note.contains("authentication was cancelled"));
    assert!(note.contains("System Settings"));
}
