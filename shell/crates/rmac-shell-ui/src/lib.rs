//! Framework-neutral shell presentation model and the live design tokens
//! shared by the menu bar, Dock, OSD, and later overlays.

use std::sync::Arc;
use std::time::Duration;

use gpui::FontFeatures;

pub mod svg_icon;
pub mod text_edit;
pub mod text_field;
pub mod tokens;
pub mod touch;

pub use svg_icon::svg_icon;

/// Stable-width figures and the two measured Inter alternates used by shell
/// values whose numbers change in place.
pub fn tabular_font_features() -> FontFeatures {
    FontFeatures(Arc::new(vec![
        ("tnum".into(), 1),
        ("cv08".into(), 1),
        ("ss03".into(), 1),
    ]))
}

/// Laid-out width of one line of body text in the window's font, for sizing
/// menus to their content the way AppKit does.
pub fn text_width(window: &gpui::Window, text: &str, weight: gpui::FontWeight) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let style = window.text_style();
    let mut font = style.font();
    font.weight = weight;
    let run = gpui::TextRun {
        len: text.len(),
        font,
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        gpui::SharedString::from(text.to_owned()),
        gpui::px(tokens::body_text_size()),
        &[run],
        None,
    );
    f32::from(line.width())
}

/// Delay to the next wall-clock minute without a periodic redraw loop.
pub fn delay_until_next_minute(epoch_millis: u128) -> Duration {
    const MINUTE_MILLIS: u128 = 60_000;
    let remaining = MINUTE_MILLIS - epoch_millis % MINUTE_MILLIS;
    Duration::from_millis(remaining as u64)
}

pub fn delay_until_next_clock_tick(epoch_millis: u128, show_seconds: bool) -> Duration {
    if show_seconds {
        let remaining = 1_000 - epoch_millis % 1_000;
        Duration::from_millis(remaining as u64)
    } else {
        delay_until_next_minute(epoch_millis)
    }
}

pub fn top_bar_clock_pattern(settings: &rmac_shell_settings::ClockSettings) -> String {
    match top_bar_clock_parts(settings) {
        (Some(date), time) => format!("{date} {time}"),
        (None, time) => time.to_owned(),
    }
}

/// The clock split into its optional date and its time, so the menu bar can
/// set the wider macOS gap between them. Menu Bar ▸ Clock Options… chooses
/// the date, the day of the week, seconds and am/pm.
pub fn top_bar_clock_parts(
    settings: &rmac_shell_settings::ClockSettings,
) -> (Option<&'static str>, &'static str) {
    use rmac_shell_settings::ClockFormat;

    let date = match (settings.show_date, settings.show_weekday) {
        (true, true) => Some("%a %-d %b"),
        (true, false) => Some("%-d %b"),
        (false, true) => Some("%a"),
        (false, false) => None,
    };
    let twenty_four = settings.format == ClockFormat::TwentyFourHour;
    let time = match (twenty_four, settings.show_seconds, settings.show_am_pm) {
        (true, false, _) => "%H:%M",
        (true, true, _) => "%H:%M:%S",
        (false, false, true) => "%-I:%M %p",
        (false, true, true) => "%-I:%M:%S %p",
        (false, false, false) => "%-I:%M",
        (false, true, false) => "%-I:%M:%S",
    };
    (date, time)
}

pub fn top_bar_workspace_label(snapshot: &rmac_shell_status::Snapshot) -> Option<String> {
    snapshot.clock.show_workspace.then(|| {
        snapshot
            .focused
            .workspace_label
            .clone()
            .or_else(|| {
                snapshot
                    .focused
                    .workspace_id
                    .map(|workspace| workspace.0.to_string())
            })
            .unwrap_or_else(|| "No workspace".into())
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopBarIndicatorLabel {
    pub kind: TopBarIndicatorKind,
    pub visible: String,
    pub accessible: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopBarIndicatorKind {
    Focus,
    Vpn,
    Network,
    Bluetooth,
    Sound,
    Battery,
    /// An app is recording from the microphone (BAR-09): the Mac's orange
    /// privacy dot.
    Microphone,
    /// A screen recording is running (SHOT-01): the stop button.
    ScreenRecording,
    Notifications,
}

pub fn top_bar_active_app_name(snapshot: &rmac_shell_status::Snapshot) -> String {
    snapshot
        .focused
        .app_id
        .as_deref()
        .map(app_display_name)
        .filter(|name| !name.is_empty())
        // The first-party file manager owns the desktop
        // identity when no application window has focus.
        .unwrap_or_else(|| "Files".to_owned())
}

/// The menu-bar name for an app id. Opening a popup temporarily moves keyboard
/// focus to the shell's layer surface, so hosts keep the last known app id and
/// name it with this instead of falling back to the desktop identity.
pub fn app_display_name(app_id: &str) -> String {
    rmac_apps::identity::window_title(app_id)
        .map(str::to_owned)
        // A Windows app (Lulo on Windows) has the name its compositor
        // backend read from it; Lulo OS records none.
        .or_else(|| rmac_apps::windows_apps::display_name_for(app_id))
        .unwrap_or_else(|| humanize_app_id(app_id))
}

fn humanize_app_id(app_id: &str) -> String {
    let leaf = app_id.rsplit(['.', '/']).next().unwrap_or(app_id);
    leaf.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) if first.is_lowercase() => {
                    first.to_uppercase().chain(characters).collect()
                }
                Some(first) => first.to_string() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn top_bar_indicator_labels(
    snapshot: &rmac_shell_status::Snapshot,
) -> Vec<TopBarIndicatorLabel> {
    let mut labels = Vec::new();
    if let Some(focus) = snapshot.focus.as_ref().filter(|focus| focus.enabled) {
        let mode = focus.mode.as_deref().unwrap_or("Focus");
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Focus,
            visible: String::new(),
            accessible: format!("Focus enabled: {mode}"),
        });
    }
    if let Some(vpn) = snapshot
        .vpn
        .as_ref()
        .filter(|vpn| vpn.transitioning || !vpn.active_names.is_empty())
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Vpn,
            visible: String::new(),
            accessible: if vpn.active_names.is_empty() {
                "VPN connecting".into()
            } else {
                format!("VPN connected: {}", vpn.active_names.join(", "))
            },
        });
    }
    // macOS order and defaults: battery, then Wi-Fi; Bluetooth and Sound
    // live in Control Center and are not shown in the menu bar by default.
    if let Some(battery) = snapshot.battery {
        let percentage = format!("{}%", battery.percentage);
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Battery,
            visible: if snapshot.show_battery_percentage {
                percentage.clone()
            } else {
                String::new()
            },
            accessible: format!("Battery {percentage}, {}", battery.state.label()),
        });
    }
    if let Some(network) = &snapshot.network {
        let strength = network
            .wifi_bars
            .map(|bars| format!(", signal {bars} of 3 bars"))
            .unwrap_or_default();
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Network,
            visible: String::new(),
            accessible: format!("Wi-Fi {}{strength}", network_state_label(network.state)),
        });
    }
    if let Some(microphone) = &snapshot.microphone {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Microphone,
            visible: String::new(),
            accessible: format!("Microphone in use by {}", microphone.apps.join(", ")),
        });
    }
    if snapshot.screen_recording.is_some() {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::ScreenRecording,
            visible: String::new(),
            accessible: "Stop screen recording".into(),
        });
    }
    if let Some(notifications) = snapshot
        .notifications
        .filter(|notifications| notifications.unread_count > 0)
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Notifications,
            visible: notifications.unread_count.to_string(),
            accessible: format!("{} unread notifications", notifications.unread_count),
        });
    }
    labels
}

fn network_state_label(state: rmac_shell_status::NetworkState) -> &'static str {
    match state {
        rmac_shell_status::NetworkState::Unavailable => "unavailable",
        rmac_shell_status::NetworkState::Disconnected => "disconnected",
        rmac_shell_status::NetworkState::Portal => "requires sign-in",
        rmac_shell_status::NetworkState::Limited => "limited",
        rmac_shell_status::NetworkState::Connected => "connected",
        rmac_shell_status::NetworkState::Unknown => "state unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_numbers_use_stable_inter_figures() {
        assert_eq!(
            tabular_font_features().tag_value_list(),
            &[("tnum".into(), 1), ("cv08".into(), 1), ("ss03".into(), 1),]
        );
    }

    #[test]
    fn a_recording_app_shows_the_microphone_indicator() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        assert!(top_bar_indicator_labels(&snapshot).is_empty());
        snapshot.microphone = Some(rmac_shell_status::MicrophoneIndicator {
            apps: vec!["Lulo Voice".into()],
        });
        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].kind, TopBarIndicatorKind::Microphone);
        assert_eq!(labels[0].accessible, "Microphone in use by Lulo Voice");
        assert!(labels[0].visible.is_empty());
    }

    #[test]
    fn a_running_screen_recording_shows_the_stop_button() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.screen_recording = Some(rmac_shell_status::ScreenRecordingIndicator {
            started_unix_ms: 1,
        });
        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].kind, TopBarIndicatorKind::ScreenRecording);
        assert_eq!(labels[0].accessible, "Stop screen recording");
    }

    #[test]
    fn minute_delay_is_bounded_and_never_busy_loops() {
        assert_eq!(delay_until_next_minute(0), Duration::from_secs(60));
        assert_eq!(delay_until_next_minute(59_999), Duration::from_millis(1));
        assert_eq!(delay_until_next_minute(60_000), Duration::from_secs(60));
    }

    #[test]
    fn clock_tick_and_pattern_follow_visible_precision() {
        let mut settings = rmac_shell_settings::ClockSettings::default();
        assert_eq!(
            delay_until_next_clock_tick(59_999, false),
            Duration::from_millis(1)
        );
        assert_eq!(top_bar_clock_pattern(&settings), "%a %-d %b %-I:%M %p");

        settings.show_date = false;
        settings.show_weekday = false;
        settings.show_seconds = true;
        settings.format = rmac_shell_settings::ClockFormat::TwentyFourHour;
        assert_eq!(
            delay_until_next_clock_tick(1_999, true),
            Duration::from_millis(1)
        );
        assert_eq!(top_bar_clock_pattern(&settings), "%H:%M:%S");
    }

    #[test]
    fn clock_options_choose_the_weekday_and_am_pm() {
        let mut settings = rmac_shell_settings::ClockSettings {
            format: rmac_shell_settings::ClockFormat::TwelveHour,
            ..Default::default()
        };
        settings.show_weekday = false;
        assert_eq!(
            top_bar_clock_parts(&settings),
            (Some("%-d %b"), "%-I:%M %p")
        );
        settings.show_date = false;
        settings.show_weekday = true;
        settings.show_am_pm = false;
        assert_eq!(top_bar_clock_parts(&settings), (Some("%a"), "%-I:%M"));
        settings.show_weekday = false;
        assert_eq!(top_bar_clock_pattern(&settings), "%-I:%M");
    }

    #[test]
    fn workspace_label_is_only_projected_when_enabled() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.workspace_label = Some("Writing".into());
        assert_eq!(top_bar_workspace_label(&snapshot), None);
        snapshot.clock.show_workspace = true;
        assert_eq!(
            top_bar_workspace_label(&snapshot).as_deref(),
            Some("Writing")
        );
    }

    #[test]
    fn focused_application_uses_the_stable_app_identity() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some(rmac_apps::identity::FILES.into());
        snapshot.focused.title = Some("Downloads".into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Files");
    }

    #[test]
    fn first_party_identity_uses_its_reviewed_application_name() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some(rmac_apps::identity::APP_DRAWER.into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Apps");
    }

    #[test]
    fn desktop_without_a_focused_window_is_owned_by_files() {
        assert_eq!(
            top_bar_active_app_name(&rmac_shell_status::Snapshot::default()),
            "Files"
        );
    }

    #[test]
    fn generic_desktop_ids_are_humanized_without_using_document_titles() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some("org.mozilla.firefox-nightly".into());
        snapshot.focused.title = Some("Private document title".into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Firefox Nightly");
    }

    #[test]
    fn an_app_is_named_from_its_id_while_its_popup_holds_focus() {
        // A live popup makes the focused window None; the menu bar still needs
        // the app it belongs to rather than the Finder desktop fallback.
        assert_eq!(app_display_name(rmac_apps::identity::FILES), "Files");
        assert_eq!(app_display_name("org.mozilla.firefox"), "Firefox");
    }

    #[test]
    fn clock_parts_split_the_date_from_the_time() {
        let mut settings = rmac_shell_settings::ClockSettings {
            show_date: true,
            show_seconds: false,
            format: rmac_shell_settings::ClockFormat::TwelveHour,
            ..Default::default()
        };
        assert_eq!(
            top_bar_clock_parts(&settings),
            (Some("%a %-d %b"), "%-I:%M %p")
        );
        settings.show_date = false;
        assert_eq!(top_bar_clock_parts(&settings), (Some("%a"), "%-I:%M %p"));
        settings.show_weekday = false;
        assert_eq!(top_bar_clock_parts(&settings), (None, "%-I:%M %p"));
    }

    #[test]
    fn hidden_indicators_do_not_create_placeholder_items() {
        let snapshot = rmac_shell_status::Snapshot::default();
        assert!(top_bar_indicator_labels(&snapshot).is_empty());
    }

    #[test]
    fn unavailable_sound_is_not_shown_as_a_meaningless_status_item() {
        let snapshot = rmac_shell_status::Snapshot {
            sound: Some(rmac_shell_status::SoundIndicator::default()),
            ..Default::default()
        };

        let labels = top_bar_indicator_labels(&snapshot);
        assert!(labels.is_empty());
    }

    #[test]
    fn live_indicators_have_human_accessible_state() {
        let snapshot = rmac_shell_status::Snapshot {
            network: Some(rmac_shell_status::NetworkIndicator {
                state: rmac_shell_status::NetworkState::Connected,
                connection_name: Some("Home".into()),
                wifi_bars: Some(3),
            }),
            sound: Some(rmac_shell_status::SoundIndicator {
                available: true,
                volume: 37,
                muted: false,
            }),
            ..Default::default()
        };

        // Sound lives in Control Center, as on macOS, not in the menu bar.
        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].accessible, "Wi-Fi connected, signal 3 of 3 bars");
    }
}
