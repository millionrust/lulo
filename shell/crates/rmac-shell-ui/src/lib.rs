//! Framework-neutral shell presentation model and the live design tokens
//! shared by the menu bar, Dock, OSD, and later overlays.

use std::time::Duration;

pub mod tokens;

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

pub fn top_bar_clock_pattern(settings: &rmac_shell_settings::ClockSettings) -> &'static str {
    use rmac_shell_settings::ClockFormat;

    match (settings.show_date, settings.show_seconds, settings.format) {
        (true, false, ClockFormat::TwentyFourHour) => "%a %-d %b %H:%M",
        (true, true, ClockFormat::TwentyFourHour) => "%a %-d %b %H:%M:%S",
        (false, false, ClockFormat::TwentyFourHour) => "%H:%M",
        (false, true, ClockFormat::TwentyFourHour) => "%H:%M:%S",
        (true, false, ClockFormat::Locale | ClockFormat::TwelveHour) => "%a %-d %b %-I:%M %p",
        (true, true, ClockFormat::Locale | ClockFormat::TwelveHour) => "%a %-d %b %-I:%M:%S %p",
        (false, false, ClockFormat::Locale | ClockFormat::TwelveHour) => "%-I:%M %p",
        (false, true, ClockFormat::Locale | ClockFormat::TwelveHour) => "%-I:%M:%S %p",
    }
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
    if let Some(bluetooth) = snapshot
        .bluetooth
        .as_ref()
        .filter(|bluetooth| bluetooth.powered)
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Bluetooth,
            visible: String::new(),
            accessible: format!(
                "Bluetooth on, {} connected devices",
                bluetooth.connected_devices
            ),
        });
    }
    if let Some(sound) = snapshot.sound.filter(|sound| sound.available) {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Sound,
            visible: String::new(),
            accessible: if sound.muted {
                "Sound muted".into()
            } else {
                format!("Sound volume {} percent", sound.volume)
            },
        });
    }
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
        settings.show_seconds = true;
        settings.format = rmac_shell_settings::ClockFormat::TwentyFourHour;
        assert_eq!(
            delay_until_next_clock_tick(1_999, true),
            Duration::from_millis(1)
        );
        assert_eq!(top_bar_clock_pattern(&settings), "%H:%M:%S");
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
    fn hidden_indicators_do_not_create_placeholder_items() {
        let snapshot = rmac_shell_status::Snapshot::default();
        assert!(top_bar_indicator_labels(&snapshot).is_empty());
    }

    #[test]
    fn unavailable_sound_is_not_shown_as_a_meaningless_status_item() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.sound = Some(rmac_shell_status::SoundIndicator::default());

        let labels = top_bar_indicator_labels(&snapshot);
        assert!(labels.is_empty());
    }

    #[test]
    fn live_indicators_have_human_accessible_state() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.network = Some(rmac_shell_status::NetworkIndicator {
            state: rmac_shell_status::NetworkState::Connected,
            connection_name: Some("Home".into()),
            wifi_bars: Some(3),
        });
        snapshot.sound = Some(rmac_shell_status::SoundIndicator {
            available: true,
            volume: 37,
            muted: false,
        });

        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels[0].accessible, "Wi-Fi connected, signal 3 of 3 bars");
        assert_eq!(labels[1].accessible, "Sound volume 37 percent");
    }
}
