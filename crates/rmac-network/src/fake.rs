//! In-memory [`WifiService`] double for app and view-model tests.
//!
//! `FakeWifiService` holds its state in a `Mutex` and never touches
//! NetworkManager, so app crates can exercise Wi-Fi flows (toggling the
//! radio, joining a saved or new network, forgetting a network, a rejected
//! password) without a D-Bus session or hardware. See
//! [`crate::contract::assert_wifi_service_contract`] for the state-machine
//! assertions every implementation, including this one, is expected to
//! satisfy.

use std::sync::Mutex;

use super::*;

#[derive(Clone, Debug, Default)]
struct FakeState {
    available: bool,
    enabled: bool,
    interface: Option<String>,
    networks: Vec<WifiNetwork>,
    saved: Vec<WifiSavedNetwork>,
}

/// An in-memory [`WifiService`] seeded with visible and saved networks.
///
/// Construct with [`FakeWifiService::new`], seed it with
/// [`with_network`](Self::with_network) / [`with_saved`](Self::with_saved),
/// then pass `&fake` anywhere a `&impl WifiService` (or `&dyn WifiService`)
/// is expected.
pub struct FakeWifiService {
    state: Mutex<FakeState>,
}

impl Default for FakeWifiService {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeWifiService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeState {
                available: true,
                enabled: true,
                interface: Some("wlan0".to_string()),
                networks: Vec::new(),
                saved: Vec::new(),
            }),
        }
    }

    /// No Wi-Fi adapter is present: `snapshot` reports `available: false`
    /// and every mutation fails, matching a headless or Wi-Fi-less host.
    pub fn without_adapter() -> Self {
        Self {
            state: Mutex::new(FakeState {
                available: false,
                enabled: false,
                interface: None,
                networks: Vec::new(),
                saved: Vec::new(),
            }),
        }
    }

    pub fn with_enabled(self, enabled: bool) -> Self {
        self.state.lock().unwrap().enabled = enabled;
        self
    }

    /// Adds a visible access point. Networks already marked `known` are
    /// mirrored into the saved-network list.
    pub fn with_network(self, network: WifiNetwork) -> Self {
        {
            let mut state = self.state.lock().unwrap();
            if network.known {
                state.saved.push(WifiSavedNetwork {
                    id: network.id.clone(),
                    ssid: network.ssid.clone(),
                });
            }
            state.networks.push(network);
        }
        self
    }

    pub fn with_saved(self, saved: WifiSavedNetwork) -> Self {
        self.state.lock().unwrap().saved.push(saved);
        self
    }
}

impl WifiService for FakeWifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error> {
        let state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("read Wi-Fi state", "no Wi-Fi adapter found"));
        }
        let current_ssid = state
            .networks
            .iter()
            .find(|network| network.connected)
            .map(|network| network.ssid.clone());
        Ok(WifiSnapshot {
            available: true,
            enabled: state.enabled,
            interface: state.interface.clone(),
            current_ssid,
            networks: state.networks.clone(),
            saved_networks: state.saved.clone(),
        })
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("change Wi-Fi power", "no Wi-Fi adapter found"));
        }
        state.enabled = enabled;
        if !enabled {
            for network in &mut state.networks {
                network.connected = false;
            }
        }
        Ok(())
    }

    fn request_scan(&self) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new(
                "scan for Wi-Fi networks",
                "no Wi-Fi adapter found",
            ));
        }
        if !state.enabled {
            return Err(Error::new("scan for Wi-Fi networks", "Wi-Fi is off"));
        }
        Ok(())
    }

    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("join Wi-Fi network", "no Wi-Fi adapter found"));
        }
        if !state.enabled {
            return Err(Error::new("join Wi-Fi network", "Wi-Fi is off"));
        }
        let index = state
            .networks
            .iter()
            .position(|candidate| &candidate.id == network)
            .ok_or_else(|| Error::new("join Wi-Fi network", "the network is no longer visible"))?;
        if !state.networks[index].can_connect() {
            return Err(Error::new(
                "join Wi-Fi network",
                "this network needs credentials",
            ));
        }
        activate(&mut state, index);
        drop(state);
        self.snapshot()
    }

    fn forget(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("forget Wi-Fi network", "no Wi-Fi adapter found"));
        }
        let was_saved = {
            let before = state.saved.len();
            state.saved.retain(|saved| &saved.id != network);
            state.saved.len() != before
        };
        for candidate in &mut state.networks {
            if &candidate.id == network {
                candidate.known = false;
                candidate.connected = false;
            }
        }
        if !was_saved {
            return Err(Error::new(
                "forget Wi-Fi network",
                "the network is not saved",
            ));
        }
        drop(state);
        self.snapshot()
    }

    fn connect_with_password(
        &self,
        network: &WifiNetworkId,
        password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled("join Wi-Fi network"));
        }
        let _ = password.expose(|_| ());
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("join Wi-Fi network", "no Wi-Fi adapter found"));
        }
        let index = state
            .networks
            .iter()
            .position(|candidate| &candidate.id == network)
            .ok_or_else(|| Error::new("join Wi-Fi network", "the network is no longer visible"))?;
        if !state.networks[index].security.needs_password() {
            return Err(Error::new(
                "join Wi-Fi network",
                "this network does not take a password",
            ));
        }
        state.networks[index].known = true;
        activate(&mut state, index);
        drop(state);
        self.snapshot()
    }

    fn connect_enterprise(
        &self,
        network: &WifiNetworkId,
        credentials: WifiEnterpriseCredentials,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled("join Wi-Fi network"));
        }
        let _ = credentials.expose_password(|_| ());
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("join Wi-Fi network", "no Wi-Fi adapter found"));
        }
        let index = state
            .networks
            .iter()
            .position(|candidate| &candidate.id == network)
            .ok_or_else(|| Error::new("join Wi-Fi network", "the network is no longer visible"))?;
        if state.networks[index].security != WifiSecurity::Enterprise {
            return Err(Error::new(
                "join Wi-Fi network",
                "this network is not an enterprise network",
            ));
        }
        state.networks[index].known = true;
        activate(&mut state, index);
        drop(state);
        self.snapshot()
    }
}

/// Marks `networks[index]` connected and disconnects every other network,
/// matching NetworkManager's single active Wi-Fi connection per adapter.
fn activate(state: &mut FakeState, index: usize) {
    for (candidate, network) in state.networks.iter_mut().enumerate() {
        network.connected = candidate == index;
    }
    state.networks[index].known = true;
    let activated = &state.networks[index];
    if !state.saved.iter().any(|saved| saved.id == activated.id) {
        state.saved.push(WifiSavedNetwork {
            id: activated.id.clone(),
            ssid: activated.ssid.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    #[test]
    fn fake_satisfies_the_shared_wifi_service_contract() {
        let cafe = WifiNetworkId::from_bytes(b"Cafe".to_vec(), WifiSecurity::Open).unwrap();
        let service = FakeWifiService::new().with_network(WifiNetwork {
            id: cafe,
            ssid: "Cafe".into(),
            strength: 55,
            security: WifiSecurity::Open,
            known: false,
            connected: false,
        });
        contract::assert_wifi_service_contract(&service);
    }

    #[test]
    fn fake_reports_snapshot_errors_without_an_adapter() {
        let service = FakeWifiService::without_adapter();
        assert!(service.snapshot().is_err());
        assert!(service.set_enabled(true).is_err());
        assert!(service.request_scan().is_err());
    }

    #[test]
    fn connecting_deactivates_the_previously_connected_network() {
        let home = WifiNetworkId::from_bytes(b"Home".to_vec(), WifiSecurity::Open).unwrap();
        let cafe = WifiNetworkId::from_bytes(b"Cafe".to_vec(), WifiSecurity::Open).unwrap();
        let service = FakeWifiService::new()
            .with_network(WifiNetwork {
                id: home.clone(),
                ssid: "Home".into(),
                strength: 90,
                security: WifiSecurity::Open,
                known: true,
                connected: true,
            })
            .with_network(WifiNetwork {
                id: cafe.clone(),
                ssid: "Cafe".into(),
                strength: 40,
                security: WifiSecurity::Open,
                known: false,
                connected: false,
            });

        let snapshot = service.connect(&cafe).expect("cafe is open and visible");
        assert_eq!(snapshot.current_ssid.as_deref(), Some("Cafe"));
        let home_entry = snapshot
            .networks
            .iter()
            .find(|network| network.id == home)
            .unwrap();
        assert!(!home_entry.connected);
    }

    #[test]
    fn forgetting_an_unsaved_network_is_an_error() {
        let ghost = WifiNetworkId::from_bytes(b"Ghost".to_vec(), WifiSecurity::Open).unwrap();
        let service = FakeWifiService::new();
        assert!(service.forget(&ghost).is_err());
    }
}
