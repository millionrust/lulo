//! Control Center detail views and keyboard focus.
//!
//! Choosing the Wi-Fi or Bluetooth module, or the Sound module's output
//! button, replaces the grid with a list in the same surface, as macOS 26
//! does: nearby and saved networks, paired devices, or sound outputs, then a
//! "… Settings…" row. Geometry is from `design-lab/control-center.html`
//! (measured 2026-09-24), in points from the detail panel's top-left.

use crate::{Command, Inputs};

/// Which detail view is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Detail {
    Wifi,
    Bluetooth,
    Sound,
}

impl Detail {
    pub fn title(self) -> &'static str {
        match self {
            Self::Wifi => "Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Sound => "Sound",
        }
    }

    /// The last row, which opens this System Settings pane.
    pub fn settings(self) -> (&'static str, &'static str) {
        match self {
            Self::Wifi => ("Wi-Fi Settings\u{2026}", "wifi"),
            Self::Bluetooth => ("Bluetooth Settings\u{2026}", "bluetooth"),
            Self::Sound => ("Sound Settings\u{2026}", "sound"),
        }
    }
}

/// Panel geometry, measured unless marked S.
pub mod geometry {
    /// The Mac's detail panel; rmac's 316 surface holds it 4 in.
    pub const PANEL_WIDTH: f32 = 308.0;
    pub const PANEL_LEFT: f32 = 4.0;
    pub const INSET: f32 = 14.0;
    pub const ROW_WIDTH: f32 = 280.0;
    pub const TITLE_TOP: f32 = 12.0;
    pub const TITLE_HEIGHT: f32 = 16.0;
    pub const SWITCH_TOP: f32 = 8.0;
    pub const SWITCH_WIDTH: f32 = 54.0;
    pub const SWITCH_HEIGHT: f32 = 24.0;
    pub const NOTICE_TOP: f32 = 36.0;
    /// First separator below a notice row or the Sound slider.
    pub const TALL_HEADER: f32 = 63.0;
    /// S: first separator below the title alone.
    pub const SHORT_HEADER: f32 = 41.0;
    pub const SLIDER_TOP: f32 = 38.0;
    pub const SLIDER_HEIGHT: f32 = 14.0;
    pub const SLIDER_LEFT: f32 = 34.0;
    pub const SLIDER_WIDTH: f32 = 230.0;
    pub const HEADING_GAP: f32 = 9.0;
    pub const HEADING_HEIGHT: f32 = 15.0;
    pub const HEADING_TO_ROW: f32 = 4.0;
    pub const ROW_HEIGHT: f32 = 32.0;
    pub const CIRCLE: f32 = 26.0;
    pub const NAME_LEFT: f32 = 34.0;
    pub const ITEM_HEIGHT: f32 = 22.0;
    /// Rows end → separator.
    pub const SEPARATOR_GAP: f32 = 6.0;
    /// Separator → a 22 pt row, and a 22 pt row → the next separator.
    pub const ITEM_GAP: f32 = 5.0;
    /// Separator → the settings row, and the settings row → the bottom.
    pub const FOOTER_GAP: f32 = 6.0;
}

/// The circle glyph a row shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowGlyph {
    Wifi,
    Bluetooth,
    Speaker,
}

/// What choosing a row does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowAction {
    Run(Command),
    /// A new protected network: Wi-Fi Settings asks for its password.
    OpenSettings(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub label: String,
    pub glyph: RowGlyph,
    /// Connected, or the current output: a white circle with a blue glyph.
    pub on: bool,
    pub locked: bool,
    /// `None` when choosing it does nothing (it is already current).
    pub action: Option<RowAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    pub heading: &'static str,
    pub rows: Vec<Row>,
}

/// Everything one detail view shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Panel {
    pub detail: Detail,
    /// The title switch (Wi-Fi, Bluetooth) and whether it is on.
    pub switch: Option<bool>,
    /// "Weak Security…" under the Wi-Fi title while on an open network.
    pub notice: Option<&'static str>,
    pub slider: bool,
    pub sections: Vec<Section>,
    /// Wi-Fi's "Other Networks" disclosure: `Some(expanded)`.
    pub disclosure: Option<bool>,
    /// Networks under the disclosure, shown when it is expanded.
    pub others: Vec<Row>,
    /// One honest line when there is nothing to list.
    pub empty: Option<&'static str>,
}

/// Saved networks and open ones join straight away; a new protected network
/// needs credentials first. Same policy as the menu bar's Wi-Fi menu.
pub fn joins_directly(network: &rmac_network::WifiNetwork) -> bool {
    !network.connected
        && (network.known
            || matches!(
                network.security,
                rmac_network::WifiSecurity::Open | rmac_network::WifiSecurity::EnhancedOpen
            ))
}

/// The most networks listed per section; the Mac scrolls a longer list.
pub const MAX_NETWORKS: usize = 8;

fn unique_networks<'a>(
    networks: impl Iterator<Item = &'a rmac_network::WifiNetwork>,
) -> Vec<&'a rmac_network::WifiNetwork> {
    let mut unique: Vec<&rmac_network::WifiNetwork> = Vec::new();
    for network in networks.filter(|network| !network.ssid.is_empty()) {
        match unique.iter_mut().find(|seen| seen.ssid == network.ssid) {
            Some(seen) => {
                if (network.connected, network.strength) > (seen.connected, seen.strength) {
                    *seen = network;
                }
            }
            None => unique.push(network),
        }
    }
    unique.sort_by(|left, right| {
        left.ssid
            .to_lowercase()
            .cmp(&right.ssid.to_lowercase())
            .then_with(|| left.ssid.cmp(&right.ssid))
    });
    unique.truncate(MAX_NETWORKS);
    unique
}

fn network_row(network: &rmac_network::WifiNetwork) -> Row {
    let action = if network.connected {
        None
    } else if joins_directly(network) {
        Some(RowAction::Run(Command::JoinWifi(network.id.clone())))
    } else {
        Some(RowAction::OpenSettings("wifi"))
    };
    Row {
        label: network.ssid.clone(),
        glyph: RowGlyph::Wifi,
        on: network.connected,
        locked: network.security.is_secure(),
        action,
    }
}

pub fn wifi_panel(wifi: &rmac_network::WifiSnapshot, others_expanded: bool) -> Panel {
    let on = wifi.available && wifi.enabled;
    let mut panel = Panel {
        detail: Detail::Wifi,
        switch: wifi.available.then_some(wifi.enabled),
        notice: None,
        slider: false,
        sections: Vec::new(),
        disclosure: None,
        others: Vec::new(),
        empty: None,
    };
    if !wifi.available {
        panel.empty = Some("Wi-Fi Unavailable");
        return panel;
    }
    if !on {
        return panel;
    }
    let weak = wifi.networks.iter().any(|network| {
        network.connected
            && matches!(
                network.security,
                rmac_network::WifiSecurity::Open | rmac_network::WifiSecurity::Legacy
            )
    });
    if weak {
        panel.notice = Some("Weak Security\u{2026}");
    }
    let known = unique_networks(wifi.networks.iter().filter(|network| network.known));
    if !known.is_empty() {
        panel.sections.push(Section {
            heading: "Known Networks",
            rows: known.iter().map(|network| network_row(network)).collect(),
        });
    }
    let known_names = known
        .iter()
        .map(|network| network.ssid.as_str())
        .collect::<Vec<_>>();
    panel.disclosure = Some(others_expanded);
    if others_expanded {
        panel.others = unique_networks(
            wifi.networks
                .iter()
                .filter(|network| !network.known && !known_names.contains(&network.ssid.as_str())),
        )
        .into_iter()
        .map(network_row)
        .collect();
        if panel.others.is_empty() {
            panel.empty = Some("No Other Networks");
        }
    }
    panel
}

pub fn bluetooth_panel(bluetooth: &rmac_bluetooth::Snapshot) -> Panel {
    let mut panel = Panel {
        detail: Detail::Bluetooth,
        switch: bluetooth.available.then_some(bluetooth.powered),
        notice: None,
        slider: false,
        sections: Vec::new(),
        disclosure: None,
        others: Vec::new(),
        empty: None,
    };
    if !bluetooth.available {
        panel.empty = Some("Bluetooth Unavailable");
        return panel;
    }
    if !bluetooth.powered {
        return panel;
    }
    let mut devices = bluetooth
        .devices
        .iter()
        .filter(|device| device.paired)
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    devices.truncate(MAX_NETWORKS);
    if devices.is_empty() {
        panel.empty = Some("No Paired Devices");
        return panel;
    }
    panel.sections.push(Section {
        heading: "Devices",
        rows: devices
            .into_iter()
            .map(|device| Row {
                label: if device.name.is_empty() {
                    device.address.clone()
                } else {
                    device.name.clone()
                },
                glyph: RowGlyph::Bluetooth,
                on: device.connected,
                locked: false,
                action: Some(RowAction::Run(Command::SetBluetoothDeviceConnected {
                    device: device.id.clone(),
                    connected: !device.connected,
                })),
            })
            .collect(),
    });
    panel
}

/// One sound output: its id, name and whether it is the default.
pub struct Output<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub current: bool,
}

pub fn sound_panel<'a>(
    available: bool,
    can_choose: bool,
    outputs: impl IntoIterator<Item = Output<'a>>,
) -> Panel {
    let mut panel = Panel {
        detail: Detail::Sound,
        switch: None,
        notice: None,
        slider: true,
        sections: Vec::new(),
        disclosure: None,
        others: Vec::new(),
        empty: None,
    };
    if !available {
        panel.empty = Some("No Output Device");
        return panel;
    }
    let rows = outputs
        .into_iter()
        .take(MAX_NETWORKS)
        .map(|output| Row {
            label: output.name.to_owned(),
            glyph: RowGlyph::Speaker,
            on: output.current,
            locked: false,
            action: (can_choose && !output.current)
                .then(|| RowAction::Run(Command::SetDefaultOutput(output.id.to_owned()))),
        })
        .collect::<Vec<_>>();
    if !rows.is_empty() {
        panel.sections.push(Section {
            heading: "Output",
            rows,
        });
    }
    panel
}

/// The panel for `detail` from the live inputs.
pub fn panel(detail: Detail, inputs: &Inputs, others_expanded: bool) -> Panel {
    match detail {
        Detail::Wifi => wifi_panel(&inputs.wifi, others_expanded),
        Detail::Bluetooth => bluetooth_panel(&inputs.bluetooth),
        Detail::Sound => sound_panel(
            inputs.audio.available && inputs.audio.has_output,
            inputs.audio.can_set_default,
            inputs.audio.outputs.iter().map(|device| Output {
                id: &device.id,
                name: &device.name,
                current: device.is_default,
            }),
        ),
    }
}

/// Something in a detail view the keyboard can reach, top to bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Switch,
    Notice,
    Slider,
    /// Row `row` of section `section`.
    Row {
        section: usize,
        row: usize,
    },
    Disclosure,
    /// Row `row` under the disclosure.
    Other(usize),
    Settings,
}

/// Where each part of a panel sits, from the panel's top.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub target: Option<Target>,
    pub kind: Part,
    pub top: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Separator,
    Heading,
    Row,
    Item,
    Empty,
}

impl Panel {
    /// Keyboard order: the switch, the notice or slider, every row, the
    /// disclosure and what it shows, then the settings row.
    pub fn targets(&self) -> Vec<Target> {
        let mut targets = Vec::new();
        if self.switch.is_some() {
            targets.push(Target::Switch);
        }
        if self.notice.is_some() {
            targets.push(Target::Notice);
        }
        if self.slider {
            targets.push(Target::Slider);
        }
        for (section, rows) in self.sections.iter().enumerate() {
            for row in 0..rows.rows.len() {
                targets.push(Target::Row { section, row });
            }
        }
        if self.disclosure.is_some() {
            targets.push(Target::Disclosure);
        }
        targets.extend((0..self.others.len()).map(Target::Other));
        targets.push(Target::Settings);
        targets
    }

    /// Drop rows from the end until the panel is no taller than `max`: the
    /// surface cannot grow past what the output was checked to fit, and the
    /// Mac scrolls such a list instead. The connected row is never dropped
    /// before the others in its section.
    pub fn fit(&mut self, max: f32) {
        while self.layout().1 > max {
            if self.others.pop().is_some() {
                continue;
            }
            let Some(section) = self
                .sections
                .iter_mut()
                .rev()
                .find(|section| section.rows.len() > 1)
            else {
                return;
            };
            match section.rows.iter().rposition(|row| !row.on) {
                Some(index) => {
                    section.rows.remove(index);
                }
                None => {
                    section.rows.pop();
                }
            }
        }
    }

    pub fn row(&self, target: Target) -> Option<&Row> {
        match target {
            Target::Row { section, row } => self.sections.get(section)?.rows.get(row),
            Target::Other(row) => self.others.get(row),
            _ => None,
        }
    }

    /// Separators, headings, rows and 22 pt items with their tops, and the
    /// panel height, following the measured spacing.
    pub fn layout(&self) -> (Vec<Placed>, f32) {
        use geometry::*;
        let mut placed = Vec::new();
        let mut y = if self.notice.is_some() || self.slider {
            TALL_HEADER
        } else {
            SHORT_HEADER
        };
        let separator = |placed: &mut Vec<Placed>, y: f32| {
            placed.push(Placed {
                target: None,
                kind: Part::Separator,
                top: y,
            });
        };
        let mut first = true;
        for (section_index, section) in self.sections.iter().enumerate() {
            if !first {
                y += SEPARATOR_GAP;
            }
            first = false;
            separator(&mut placed, y);
            y += HEADING_GAP;
            placed.push(Placed {
                target: None,
                kind: Part::Heading,
                top: y,
            });
            y += HEADING_HEIGHT + HEADING_TO_ROW;
            for row in 0..section.rows.len() {
                placed.push(Placed {
                    target: Some(Target::Row {
                        section: section_index,
                        row,
                    }),
                    kind: Part::Row,
                    top: y,
                });
                y += ROW_HEIGHT;
            }
        }
        if let Some(expanded) = self.disclosure {
            if !first {
                y += SEPARATOR_GAP;
            }
            first = false;
            separator(&mut placed, y);
            y += ITEM_GAP;
            placed.push(Placed {
                target: Some(Target::Disclosure),
                kind: Part::Item,
                top: y,
            });
            y += ITEM_HEIGHT;
            if expanded {
                y += HEADING_TO_ROW;
                for row in 0..self.others.len() {
                    placed.push(Placed {
                        target: Some(Target::Other(row)),
                        kind: Part::Row,
                        top: y,
                    });
                    y += ROW_HEIGHT;
                }
                if self.others.is_empty() && self.empty.is_some() {
                    placed.push(Placed {
                        target: None,
                        kind: Part::Empty,
                        top: y,
                    });
                    y += ITEM_HEIGHT;
                }
            }
            // The next separator sits 5 below a 22 pt row, not 6.
            y -= SEPARATOR_GAP - ITEM_GAP;
        } else if self.empty.is_some() && first {
            separator(&mut placed, y);
            y += ITEM_GAP;
            first = false;
            placed.push(Placed {
                target: None,
                kind: Part::Empty,
                top: y,
            });
            y += ITEM_HEIGHT;
            y -= SEPARATOR_GAP - ITEM_GAP;
        }
        if !first {
            y += SEPARATOR_GAP;
        }
        separator(&mut placed, y);
        y += FOOTER_GAP;
        placed.push(Placed {
            target: Some(Target::Settings),
            kind: Part::Item,
            top: y,
        });
        y += ITEM_HEIGHT + FOOTER_GAP;
        (placed, y)
    }
}

/// Keyboard focus inside the grid, in reading order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Module {
    Wifi,
    Bluetooth,
    LowPower,
    Screenshot,
    Focus,
    Display,
    Sound,
}

impl Module {
    /// The detail view Return opens, for the modules that have one.
    pub fn detail(self) -> Option<Detail> {
        match self {
            Self::Wifi => Some(Detail::Wifi),
            Self::Bluetooth => Some(Detail::Bluetooth),
            Self::Sound => Some(Detail::Sound),
            _ => None,
        }
    }

    pub fn is_slider(self) -> bool {
        matches!(self, Self::Display | Self::Sound)
    }
}

/// Move `current` one step through `order`, stopping at the ends.
pub fn step<T: Copy + PartialEq>(order: &[T], current: Option<T>, forward: bool) -> Option<T> {
    if order.is_empty() {
        return None;
    }
    let index = current.and_then(|current| order.iter().position(|item| *item == current));
    let next = match (index, forward) {
        (None, true) => 0,
        (None, false) => order.len() - 1,
        (Some(index), true) => (index + 1).min(order.len() - 1),
        (Some(index), false) => index.saturating_sub(1),
    };
    Some(order[next])
}

/// A slider nudged by the arrow keys: 1/16 of the range per press, as the
/// Mac's volume keys step.
pub fn nudge(value: u8, up: bool) -> u8 {
    const STEP: u8 = 6;
    if up {
        value.saturating_add(STEP).min(100)
    } else {
        value.saturating_sub(STEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_network::{WifiNetwork, WifiNetworkId, WifiPersonalMode, WifiSecurity};

    fn network(ssid: &str, security: WifiSecurity, known: bool, connected: bool) -> WifiNetwork {
        WifiNetwork {
            id: WifiNetworkId::from_bytes(ssid.as_bytes(), security).unwrap(),
            ssid: ssid.into(),
            strength: 60,
            security,
            known,
            connected,
        }
    }

    fn wifi(networks: Vec<WifiNetwork>) -> rmac_network::WifiSnapshot {
        rmac_network::WifiSnapshot {
            available: true,
            enabled: true,
            networks,
            ..Default::default()
        }
    }

    const WPA: WifiSecurity = WifiSecurity::Personal(WifiPersonalMode::Psk);

    #[test]
    fn wifi_lists_known_networks_and_hides_the_rest_behind_the_disclosure() {
        let snapshot = wifi(vec![
            network("Home", WPA, true, true),
            network("Cafe", WifiSecurity::Open, false, false),
            network("Studio", WPA, true, false),
            network("Neighbour", WPA, false, false),
        ]);
        let closed = wifi_panel(&snapshot, false);
        assert_eq!(closed.switch, Some(true));
        assert_eq!(closed.notice, None);
        assert_eq!(closed.sections.len(), 1);
        let labels = closed.sections[0]
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["Home", "Studio"]);
        // The connected network does nothing; a saved one joins.
        assert_eq!(closed.sections[0].rows[0].action, None);
        assert!(matches!(
            closed.sections[0].rows[1].action,
            Some(RowAction::Run(Command::JoinWifi(_)))
        ));
        assert!(closed.others.is_empty());

        let open = wifi_panel(&snapshot, true);
        let others = open
            .others
            .iter()
            .map(|row| (row.label.as_str(), row.action.clone()))
            .collect::<Vec<_>>();
        assert_eq!(others.len(), 2);
        assert_eq!(others[0].0, "Cafe");
        assert!(matches!(
            others[0].1,
            Some(RowAction::Run(Command::JoinWifi(_)))
        ));
        // A new protected network goes to Wi-Fi Settings for its password.
        assert_eq!(
            others[1],
            ("Neighbour", Some(RowAction::OpenSettings("wifi")))
        );
    }

    #[test]
    fn an_open_connection_shows_weak_security_and_wifi_off_lists_nothing() {
        let weak = wifi_panel(
            &wifi(vec![network("Cafe", WifiSecurity::Open, true, true)]),
            false,
        );
        assert_eq!(weak.notice, Some("Weak Security\u{2026}"));
        let mut off = wifi(vec![network("Home", WPA, true, false)]);
        off.enabled = false;
        let panel = wifi_panel(&off, false);
        assert_eq!(panel.switch, Some(false));
        assert!(panel.sections.is_empty() && panel.disclosure.is_none());
        assert_eq!(panel.targets(), [Target::Switch, Target::Settings]);
    }

    fn device(name: &str, paired: bool, connected: bool) -> rmac_bluetooth::Device {
        rmac_bluetooth::Device {
            id: format!("/org/bluez/hci0/{name}"),
            name: name.into(),
            address: "00:11:22:33:44:55".into(),
            kind: "audio-headphones".into(),
            paired,
            trusted: paired,
            connected,
        }
    }

    #[test]
    fn bluetooth_lists_paired_devices_and_toggles_their_connection() {
        let snapshot = rmac_bluetooth::Snapshot {
            available: true,
            powered: true,
            devices: vec![
                device("Speaker", true, false),
                device("Stranger", false, false),
                device("Headphones", true, true),
            ],
            ..Default::default()
        };
        let panel = bluetooth_panel(&snapshot);
        let rows = &panel.sections[0].rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Headphones");
        assert!(rows[0].on);
        assert_eq!(
            rows[0].action,
            Some(RowAction::Run(Command::SetBluetoothDeviceConnected {
                device: "/org/bluez/hci0/Headphones".into(),
                connected: false,
            }))
        );
        let empty = bluetooth_panel(&rmac_bluetooth::Snapshot {
            available: true,
            powered: true,
            ..Default::default()
        });
        assert_eq!(empty.empty, Some("No Paired Devices"));
    }

    #[test]
    fn sound_lists_outputs_and_picks_a_new_default() {
        let outputs = [
            Output {
                id: "51",
                name: "Built-in Audio",
                current: true,
            },
            Output {
                id: "64",
                name: "HDMI Output",
                current: false,
            },
        ];
        let panel = sound_panel(true, true, outputs);
        let rows = &panel.sections[0].rows;
        assert_eq!(rows[0].action, None);
        assert_eq!(
            rows[1].action,
            Some(RowAction::Run(Command::SetDefaultOutput("64".into())))
        );
        let fixed = sound_panel(
            true,
            false,
            [Output {
                id: "64",
                name: "HDMI Output",
                current: false,
            }],
        );
        assert_eq!(fixed.sections[0].rows[0].action, None);
    }

    #[test]
    fn layout_follows_the_measured_spacing() {
        // Sound with two outputs: slider header 63, heading 72, rows 91 and
        // 123, separator 161, settings 167, height 195.
        let panel = sound_panel(
            true,
            true,
            [
                Output {
                    id: "1",
                    name: "A",
                    current: true,
                },
                Output {
                    id: "2",
                    name: "B",
                    current: false,
                },
            ],
        );
        let (placed, height) = panel.layout();
        let tops = placed
            .iter()
            .map(|part| (part.kind, part.top))
            .collect::<Vec<_>>();
        assert_eq!(
            tops,
            [
                (Part::Separator, 63.0),
                (Part::Heading, 72.0),
                (Part::Row, 91.0),
                (Part::Row, 123.0),
                (Part::Separator, 161.0),
                (Part::Item, 167.0),
            ]
        );
        assert_eq!(height, 195.0);

        // Wi-Fi with a notice, two known networks and the disclosure: the
        // disclosure sits 5 below its separator and the settings separator 5
        // below it, as measured.
        let wifi_panel = wifi_panel(
            &wifi(vec![
                network("Cafe", WifiSecurity::Open, true, true),
                network("Home", WPA, true, false),
            ]),
            false,
        );
        let (placed, height) = wifi_panel.layout();
        let disclosure = placed
            .iter()
            .find(|part| part.target == Some(Target::Disclosure))
            .unwrap();
        assert_eq!(disclosure.top, 166.0);
        let settings = placed
            .iter()
            .find(|part| part.target == Some(Target::Settings))
            .unwrap();
        assert_eq!(settings.top, 199.0);
        assert_eq!(height, 227.0);
        assert_eq!(
            wifi_panel.targets(),
            [
                Target::Switch,
                Target::Notice,
                Target::Row { section: 0, row: 0 },
                Target::Row { section: 0, row: 1 },
                Target::Disclosure,
                Target::Settings,
            ]
        );
    }

    #[test]
    fn list_commands_are_checked_against_the_live_inputs() {
        use crate::{StartError, State};
        let home = network("Home", WPA, true, false);
        let stranger = network("Neighbour", WPA, false, false);
        let mut state = State::new(Inputs {
            wifi: wifi(vec![home.clone(), stranger.clone()]),
            bluetooth: rmac_bluetooth::Snapshot {
                available: true,
                powered: true,
                devices: vec![
                    device("Speaker", true, false),
                    device("Stranger", false, false),
                ],
                ..Default::default()
            },
            audio: rmac_audio::Snapshot {
                available: true,
                has_output: true,
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(state.begin(Command::JoinWifi(home.id.clone())).is_ok());
        assert!(matches!(
            state.begin(Command::JoinWifi(stranger.id.clone())),
            Err(StartError::Busy(_))
        ));
        let mut fresh = state.clone();
        fresh.pending.clear();
        assert!(matches!(
            fresh.begin(Command::JoinWifi(stranger.id)),
            Err(StartError::Unsupported { .. })
        ));
        assert!(fresh
            .begin(Command::SetBluetoothDeviceConnected {
                device: "/org/bluez/hci0/Speaker".into(),
                connected: true,
            })
            .is_ok());
        fresh.pending.clear();
        assert!(matches!(
            fresh.begin(Command::SetBluetoothDeviceConnected {
                device: "/org/bluez/hci0/Stranger".into(),
                connected: true,
            }),
            Err(StartError::Invalid { .. })
        ));
        // The sound server here cannot change the default output.
        assert!(matches!(
            fresh.begin(Command::SetDefaultOutput("51".into())),
            Err(StartError::Unsupported { .. })
        ));
    }

    #[test]
    fn a_long_list_is_cut_to_fit_the_tallest_surface() {
        let mut networks = (0..8)
            .map(|index| network(&format!("Known {index}"), WPA, true, index == 7))
            .collect::<Vec<_>>();
        networks.extend((0..8).map(|index| network(&format!("Other {index}"), WPA, false, false)));
        let mut panel = wifi_panel(&wifi(networks), true);
        let max = crate::layout::MAX_SURFACE_HEIGHT as f32;
        assert!(panel.layout().1 > max);
        panel.fit(max);
        assert!(panel.layout().1 <= max);
        assert!(panel.sections[0].rows.iter().any(|row| row.on));
    }

    #[test]
    fn stepping_stops_at_the_ends_and_nudges_clamp() {
        let order = [1, 2, 3];
        assert_eq!(step(&order, None, true), Some(1));
        assert_eq!(step(&order, None, false), Some(3));
        assert_eq!(step(&order, Some(3), true), Some(3));
        assert_eq!(step(&order, Some(1), false), Some(1));
        assert_eq!(step::<u8>(&[], Some(1), true), None);
        assert_eq!(nudge(98, true), 100);
        assert_eq!(nudge(3, false), 0);
        assert_eq!(nudge(50, true), 56);
    }
}
