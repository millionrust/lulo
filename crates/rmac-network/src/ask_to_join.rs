//! Wi-Fi ▸ Ask to join networks (macOS's Off / Notify): when Wi-Fi is on,
//! nothing is joined and no known network is in range, the menu bar posts
//! one notification that networks are available. Kept as a one-word
//! preference file; Notify is the default, as on the Mac.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AskToJoin {
    Off,
    #[default]
    Notify,
}

impl AskToJoin {
    pub const ALL: [Self; 2] = [Self::Off, Self::Notify];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Notify => "Notify",
        }
    }

    /// The Mac's explanation under the pop-up.
    pub fn description(self) -> &'static str {
        match self {
            Self::Off => "Known networks will be joined automatically. If no known networks are available, you will have to manually select a network.",
            Self::Notify => "Known networks will be joined automatically. If no known networks are available, you will be notified of available networks.",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Notify => "notify",
        }
    }
}

fn preference_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .map(|base| base.join("rmac").join("wifi-ask-to-join"))
}

/// The saved choice (Notify when none is saved). Reads a small file: call
/// it off the UI thread.
pub fn load() -> AskToJoin {
    let Some(path) = preference_path() else {
        return AskToJoin::default();
    };
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim() == AskToJoin::Off.id() => AskToJoin::Off,
        _ => AskToJoin::Notify,
    }
}

pub fn save(value: AskToJoin) -> std::io::Result<()> {
    let path = preference_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no configuration directory")
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, format!("{}\n", value.id()))?;
    std::fs::rename(temporary, path)
}

/// Whether the snapshot is the moment to say networks are available: Wi-Fi
/// on, nothing joined, no known network in range, and some network in
/// range. The caller notifies once per such stretch.
pub fn should_offer(snapshot: &crate::WifiSnapshot) -> bool {
    snapshot.available
        && snapshot.enabled
        && snapshot.current_ssid.is_none()
        && !snapshot
            .networks
            .iter()
            .any(|network| network.known || network.connected)
        && snapshot
            .networks
            .iter()
            .any(|network| network.security.can_create())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WifiNetwork, WifiNetworkId, WifiSecurity, WifiSnapshot};

    fn network(name: &str, known: bool) -> WifiNetwork {
        WifiNetwork {
            id: WifiNetworkId::from_bytes(name.as_bytes().to_vec(), WifiSecurity::Open).unwrap(),
            ssid: name.to_owned(),
            strength: 60,
            security: WifiSecurity::Open,
            known,
            connected: false,
        }
    }

    #[test]
    fn offers_only_unknown_networks_while_unjoined() {
        let mut snapshot = WifiSnapshot {
            available: true,
            enabled: true,
            networks: vec![network("Cafe", false)],
            ..WifiSnapshot::default()
        };
        assert!(should_offer(&snapshot));
        snapshot.networks.push(network("Home", true));
        assert!(!should_offer(&snapshot));
        snapshot.networks.clear();
        assert!(!should_offer(&snapshot));
    }
}
