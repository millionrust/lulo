//! Windows backend: Wi-Fi through the native WLAN API (wlanapi), interfaces
//! and addresses through the IP Helper API. Changes arrive by WLAN and IP
//! address notifications; nothing here polls. VPN is not provided on Windows.

use std::ffi::c_void;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, ConvertInterfaceGuidToLuid, ConvertInterfaceLuidToAlias,
    GetAdaptersAddresses, GetBestInterface, GetNetworkConnectivityHint,
    NotifyUnicastIpAddressChange, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211, IF_TYPE_SOFTWARE_LOOPBACK,
    IF_TYPE_TUNNEL, IP_ADAPTER_ADDRESSES_LH, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW,
};
use windows::Win32::NetworkManagement::Ndis::{
    IfOperStatusDormant, IfOperStatusDown, IfOperStatusLowerLayerDown, IfOperStatusNotPresent,
    IfOperStatusTesting, IfOperStatusUp, IF_OPER_STATUS, NET_LUID_LH,
};
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_infrastructure, dot11_radio_state_off, dot11_radio_state_on,
    wlan_connection_mode_discovery_secure, wlan_connection_mode_profile,
    wlan_interface_state_connected, wlan_intf_opcode_current_connection,
    wlan_intf_opcode_radio_state, wlan_notification_acm_connection_attempt_fail,
    wlan_notification_acm_connection_complete, wlan_notification_acm_scan_complete,
    wlan_notification_acm_scan_fail, WlanCloseHandle, WlanConnect, WlanDeleteProfile,
    WlanDisconnect, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
    WlanGetProfile, WlanGetProfileList, WlanOpenHandle, WlanQueryInterface,
    WlanRegisterNotification, WlanScan, WlanSetInterface, WlanSetProfile, DOT11_SSID,
    L2_NOTIFICATION_DATA, WLAN_API_VERSION_2_0, WLAN_AVAILABLE_NETWORK_CONNECTED,
    WLAN_AVAILABLE_NETWORK_HAS_PROFILE, WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_ATTRIBUTES,
    WLAN_CONNECTION_NOTIFICATION_DATA, WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST,
    WLAN_NOTIFICATION_SOURCE_ACM, WLAN_NOTIFICATION_SOURCE_MSM, WLAN_NOTIFICATION_SOURCE_NONE,
    WLAN_PHY_RADIO_STATE, WLAN_PROFILE_INFO_LIST, WLAN_RADIO_STATE,
};
use windows::Win32::Networking::WinSock::{
    AF_UNSPEC, NL_NETWORK_CONNECTIVITY_HINT, SOCKET_ADDRESS,
};
use zeroize::Zeroize as _;

use super::*;

/// How long a connection attempt may take before it is reported as failed.
/// Windows reports the outcome by notification well within this.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
/// Windows finishes a scan within four seconds (WlanScan's documentation);
/// a little more before giving up on the notification.
const SCAN_TIMEOUT: Duration = Duration::from_secs(6);

// WlanOpenHandle fails with these when the WLAN AutoConfig service is
// stopped or not installed (desktops without Wi-Fi, Server editions).
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;
const ERROR_ALREADY_EXISTS: u32 = 183;

// ---------------------------------------------------------------------------
// Wi-Fi service

impl WifiService for SystemWifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error> {
        win_wifi_snapshot()
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        const OPERATION: &str = "change Wi-Fi power";
        let client = Client::open_required(OPERATION)?;
        let interfaces = client.interfaces(OPERATION)?;
        if interfaces.is_empty() {
            return Err(Error::new(OPERATION, "no Wi-Fi adapter found"));
        }
        let target = if enabled {
            dot11_radio_state_on
        } else {
            dot11_radio_state_off
        };
        for interface in &interfaces {
            let radio = client.radio_state(&interface.guid, OPERATION)?;
            if enabled && !radio.is_empty() && radio.iter().all(|phy| phy.hardware_off) {
                return Err(Error::new(
                    OPERATION,
                    "the Wi-Fi hardware switch is off; turn it on on the computer",
                ));
            }
            for phy in radio {
                let state = WLAN_PHY_RADIO_STATE {
                    dwPhyIndex: phy.index,
                    dot11SoftwareRadioState: target,
                    // Ignored on set; only the software state can be changed.
                    dot11HardwareRadioState: target,
                };
                // SAFETY: the pointer and size describe `state`, which
                // outlives the call.
                let code = unsafe {
                    WlanSetInterface(
                        client.handle(),
                        &interface.guid,
                        wlan_intf_opcode_radio_state,
                        std::mem::size_of::<WLAN_PHY_RADIO_STATE>() as u32,
                        &state as *const WLAN_PHY_RADIO_STATE as *const c_void,
                        None,
                    )
                };
                if code != 0 {
                    return Err(wlan_error(OPERATION, code));
                }
            }
        }
        Ok(())
    }

    fn request_scan(&self) -> Result<(), Error> {
        const OPERATION: &str = "scan for Wi-Fi networks";
        let client = Client::open_required(OPERATION)?;
        let interfaces = client.interfaces(OPERATION)?;
        if interfaces.is_empty() {
            return Err(Error::new(OPERATION, "no Wi-Fi adapter found"));
        }
        drop(client);
        // Windows empties an interface's network list while it scans, so a
        // list read straight after WlanScan shows only the joined network.
        // Return once each scan has finished (its scan-complete or
        // scan-failed notification), so the caller's next snapshot sees
        // the new results. The wait blocks on the notification channel;
        // nothing polls.
        let session = ConnectSession::open(OPERATION)?;
        let mut first_error = None;
        let mut pending = Vec::new();
        for interface in &interfaces {
            // SAFETY: plain call on an open handle with a valid GUID.
            let code =
                unsafe { WlanScan(session.client.handle(), &interface.guid, None, None, None) };
            if code == 0 {
                pending.push(interface.guid);
            } else if first_error.is_none() {
                first_error = Some(code);
            }
        }
        if pending.is_empty() {
            return match first_error {
                Some(code) => Err(wlan_error(OPERATION, code)),
                None => Ok(()),
            };
        }
        let deadline = Instant::now() + SCAN_TIMEOUT;
        while !pending.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match session.events.recv_timeout(remaining) {
                Ok(ConnectEvent::ScanDone { interface }) => {
                    pending.retain(|guid| *guid != interface);
                }
                Ok(_) => {}
                // A scan Windows never reports on still leaves whatever it
                // found in the list; the caller reads that.
                Err(_) => break,
            }
        }
        Ok(())
    }

    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        const OPERATION: &str = "connect Wi-Fi";
        let session = ConnectSession::open(OPERATION)?;
        let interface = session.ready_interface(OPERATION)?;
        let entries = session
            .client
            .available_networks(&interface.guid, OPERATION)?;
        let in_range = entries.iter().filter(|entry| entry.id == *network);
        let mut found = false;
        let mut profile = None;
        for entry in in_range {
            found = true;
            if entry.connected {
                drop(session);
                return win_wifi_snapshot();
            }
            if profile.is_none() {
                profile = entry.profile.clone();
            }
        }
        if !found {
            return Err(Error::new(OPERATION, "the network is no longer in range"));
        }
        if profile.is_none() {
            profile = session
                .client
                .saved_profiles(&interface.guid)
                .into_iter()
                .find(|saved| {
                    saved
                        .id
                        .as_ref()
                        .is_some_and(|id| network.matches_profile(id))
                })
                .map(|saved| saved.name);
        }

        if let Some(profile) = profile {
            session.connect_profile(&interface.guid, &profile, network, None, false, OPERATION)?;
        } else {
            match network.security {
                WifiSecurity::Open => {
                    // A saved, auto-joining profile, so the network is
                    // remembered like any other joined network.
                    let profile = session.add_profile(
                        &interface.guid,
                        &profile_xml(&network.ssid, ProfileAuth::Open, None),
                        &network.ssid,
                        "connect open Wi-Fi network",
                    )?;
                    session.connect_profile(
                        &interface.guid,
                        &profile,
                        network,
                        None,
                        true,
                        "connect open Wi-Fi network",
                    )?;
                }
                WifiSecurity::EnhancedOpen => {
                    // Enhanced Open needs no secret; Windows discovers the
                    // network and joins it with a temporary profile.
                    session.connect_discovery(&interface.guid, network, OPERATION)?;
                }
                _ => {
                    return Err(Error::new(
                        OPERATION,
                        "this network needs security information",
                    ));
                }
            }
        }
        drop(session);
        win_wifi_snapshot()
    }

    fn forget(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        const OPERATION: &str = "forget Wi-Fi network";
        let client = Client::open_required(OPERATION)?;
        let interfaces = client.interfaces(OPERATION)?;
        let mut deleted = 0usize;
        let mut first_error = None;
        for interface in &interfaces {
            let mut names = client
                .saved_profiles(&interface.guid)
                .into_iter()
                .filter(|saved| {
                    saved
                        .id
                        .as_ref()
                        .is_some_and(|id| network.matches_profile(id))
                })
                .map(|saved| saved.name)
                .collect::<Vec<_>>();
            // Profiles whose XML could not be read are still matched through
            // the visible network that Windows ties them to.
            if let Ok(entries) = client.available_networks(&interface.guid, OPERATION) {
                for entry in entries {
                    if let Some(name) = entry.profile {
                        if network.matches_profile(&entry.id) && !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
            for name in names {
                let name = nul_terminated(&name);
                // SAFETY: `name` is a NUL-terminated UTF-16 string that
                // outlives the call. Deleting the profile of the current
                // connection also disconnects it.
                let code = unsafe {
                    WlanDeleteProfile(
                        client.handle(),
                        &interface.guid,
                        PCWSTR(name.as_ptr()),
                        None,
                    )
                };
                if code == 0 {
                    deleted += 1;
                } else if first_error.is_none() {
                    first_error = Some(code);
                }
            }
        }
        if let Some(code) = first_error {
            return Err(wlan_error(OPERATION, code));
        }
        if deleted == 0 {
            return Err(Error::new(
                OPERATION,
                "the saved network is no longer available",
            ));
        }
        drop(client);
        win_wifi_snapshot()
    }

    fn connect_with_password(
        &self,
        network: &WifiNetworkId,
        password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        const OPERATION: &str = "connect protected Wi-Fi";
        if !network.security.needs_password() {
            return Err(Error::new(
                OPERATION,
                "the selected network does not use Wi-Fi Personal security",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(Error::cancelled(OPERATION));
        }
        let session = ConnectSession::open(OPERATION)?;
        let interface = session.ready_interface(OPERATION)?;
        let entries = session
            .client
            .available_networks(&interface.guid, OPERATION)?;
        let matching = entries
            .iter()
            .filter(|entry| entry.id == *network)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Err(Error::new(OPERATION, "the network is no longer in range"));
        }
        if matching.iter().any(|entry| entry.connected) {
            drop(session);
            return win_wifi_snapshot();
        }
        if matching.iter().any(|entry| entry.profile.is_some())
            || session
                .client
                .saved_profiles(&interface.guid)
                .iter()
                .any(|saved| {
                    saved
                        .id
                        .as_ref()
                        .is_some_and(|id| network.matches_profile(id))
                })
        {
            return Err(Error::new(
                OPERATION,
                "the network became known; select it again to use the saved profile",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(Error::cancelled(OPERATION));
        }
        let auth = match network.security {
            WifiSecurity::Personal(WifiPersonalMode::Sae) => ProfileAuth::Wpa3Sae,
            // WPA2-Personal also joins WPA2/WPA3 transition networks.
            _ => ProfileAuth::Wpa2Psk,
        };
        let mut xml = password.expose(|value| profile_xml(&network.ssid, auth, Some(value)));
        drop(password);
        let added = session.add_profile(&interface.guid, &xml, &network.ssid, OPERATION);
        xml.zeroize();
        let profile = added?;
        session.connect_profile(
            &interface.guid,
            &profile,
            network,
            Some(cancellation),
            true,
            OPERATION,
        )?;
        drop(session);
        win_wifi_snapshot()
    }

    fn connect_enterprise(
        &self,
        _network: &WifiNetworkId,
        _credentials: WifiEnterpriseCredentials,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled("connect enterprise Wi-Fi"));
        }
        Err(Error::new(
            "connect enterprise Wi-Fi",
            "enterprise (802.1X) networks are joined through Windows' own sign-in prompt",
        ))
    }
}

fn win_wifi_snapshot() -> Result<WifiSnapshot, Error> {
    const OPERATION: &str = "read Wi-Fi status";
    let client = match Client::open() {
        Ok(client) => client,
        // No WLAN service means no Wi-Fi hardware Windows can use.
        Err(code) if wlan_service_absent(code) => return Ok(WifiSnapshot::default()),
        Err(code) => return Err(wlan_error(OPERATION, code)),
    };
    let interfaces = client.interfaces(OPERATION)?;
    let Some(interface) = primary_interface(&interfaces) else {
        return Ok(WifiSnapshot::default());
    };
    let radio = client.radio_state(&interface.guid, OPERATION)?;
    let enabled = radio_enabled(&radio);
    let (networks, current_ssid) = if enabled {
        let entries = client.available_networks(&interface.guid, OPERATION)?;
        let current = client
            .current_ssid(interface)
            .or_else(|| {
                entries
                    .iter()
                    .find(|entry| entry.connected)
                    .map(|entry| entry.id.ssid.clone())
            })
            .map(|ssid| display_ssid(&ssid));
        let raw = entries
            .into_iter()
            .map(|entry| RawNetwork {
                id: entry.id,
                strength: entry.strength,
                known: entry.profile.is_some(),
                connected: entry.connected,
            })
            .collect();
        (normalize_networks(raw), current)
    } else {
        (Vec::new(), None)
    };
    let profiles = client.saved_profiles(&interface.guid);
    let count = profiles.len() as u64;
    // Windows lists profiles in connection-preference order; keep it.
    let saved_networks = normalize_saved_networks(
        profiles
            .into_iter()
            .enumerate()
            .filter_map(|(index, saved)| saved.id.map(|id| (id, count - index as u64))),
    );
    Ok(WifiSnapshot {
        available: true,
        enabled,
        interface: Some(
            interface_alias(&interface.guid).unwrap_or_else(|| interface.description.clone()),
        ),
        current_ssid,
        networks,
        saved_networks,
    })
}

fn wlan_service_absent(code: u32) -> bool {
    matches!(
        code,
        ERROR_SERVICE_NOT_ACTIVE | ERROR_SERVICE_DOES_NOT_EXIST
    )
}

/// The interface the Wi-Fi pane describes: the connected one, else the first.
fn primary_interface(interfaces: &[WlanInterface]) -> Option<&WlanInterface> {
    interfaces
        .iter()
        .find(|interface| interface.connected)
        .or_else(|| interfaces.first())
}

fn radio_enabled(radio: &[PhyRadio]) -> bool {
    radio
        .iter()
        .any(|phy| !phy.software_off && !phy.hardware_off)
}

// ---------------------------------------------------------------------------
// WLAN client handle and queries

struct WlanInterface {
    guid: GUID,
    description: String,
    connected: bool,
}

struct PhyRadio {
    index: u32,
    software_off: bool,
    hardware_off: bool,
}

struct AvailableEntry {
    id: WifiNetworkId,
    strength: u8,
    connected: bool,
    /// The saved profile Windows matched to this network, if any.
    profile: Option<Vec<u16>>,
}

struct SavedProfile {
    /// The profile name, without the terminating NUL.
    name: Vec<u16>,
    /// The network it joins, when its XML could be read.
    id: Option<WifiNetworkId>,
}

/// An open WLAN client handle, closed on drop.
struct Client(HANDLE);

impl Client {
    fn open() -> Result<Self, u32> {
        let mut negotiated = 0u32;
        let mut handle = HANDLE::default();
        // SAFETY: plain out-parameters.
        let code =
            unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut handle) };
        if code == 0 {
            Ok(Self(handle))
        } else {
            Err(code)
        }
    }

    fn open_required(operation: &'static str) -> Result<Self, Error> {
        Self::open().map_err(|code| {
            if wlan_service_absent(code) {
                Error::new(operation, "no Wi-Fi adapter found")
            } else {
                wlan_error(operation, code)
            }
        })
    }

    fn handle(&self) -> HANDLE {
        self.0
    }

    /// Closes the handle. WlanCloseHandle waits for this handle's
    /// notification callbacks that are still running.
    fn close(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle came from WlanOpenHandle and is closed once.
            unsafe {
                let _ = WlanCloseHandle(self.0, None);
            }
            self.0 = HANDLE::default();
        }
    }

    fn interfaces(&self, operation: &'static str) -> Result<Vec<WlanInterface>, Error> {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        // SAFETY: the list is allocated by the WLAN API, read within its
        // reported length, then freed with WlanFreeMemory.
        unsafe {
            let code = WlanEnumInterfaces(self.0, None, &mut list);
            if code != 0 {
                return Err(wlan_error(operation, code));
            }
            if list.is_null() {
                return Ok(Vec::new());
            }
            let count = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count);
            let interfaces = items
                .iter()
                .map(|item| WlanInterface {
                    guid: item.InterfaceGuid,
                    description: wide_to_string(&item.strInterfaceDescription),
                    connected: item.isState == wlan_interface_state_connected,
                })
                .collect::<Vec<_>>();
            WlanFreeMemory(list as *const c_void);
            Ok(interfaces)
        }
    }

    fn radio_state(&self, guid: &GUID, operation: &'static str) -> Result<Vec<PhyRadio>, Error> {
        let mut size = 0u32;
        let mut data: *mut c_void = std::ptr::null_mut();
        // SAFETY: the WLAN API allocates `data`; it is read as the
        // documented WLAN_RADIO_STATE when large enough, then freed.
        unsafe {
            let code = WlanQueryInterface(
                self.0,
                guid,
                wlan_intf_opcode_radio_state,
                None,
                &mut size,
                &mut data,
                None,
            );
            if code != 0 {
                return Err(wlan_error(operation, code));
            }
            if data.is_null() {
                return Ok(Vec::new());
            }
            // Read only what the reported size covers.
            let offset = std::mem::offset_of!(WLAN_RADIO_STATE, PhyRadioState);
            let mut phys = Vec::new();
            if size as usize >= offset {
                let base = data as *const u8;
                let reported = std::ptr::read_unaligned(base as *const u32) as usize;
                let fits = (size as usize - offset) / std::mem::size_of::<WLAN_PHY_RADIO_STATE>();
                let first = base.add(offset) as *const WLAN_PHY_RADIO_STATE;
                for index in 0..reported.min(fits) {
                    let phy = std::ptr::read_unaligned(first.add(index));
                    phys.push(PhyRadio {
                        index: phy.dwPhyIndex,
                        software_off: phy.dot11SoftwareRadioState == dot11_radio_state_off,
                        hardware_off: phy.dot11HardwareRadioState == dot11_radio_state_off,
                    });
                }
            }
            WlanFreeMemory(data);
            Ok(phys)
        }
    }

    fn current_ssid(&self, interface: &WlanInterface) -> Option<Vec<u8>> {
        if !interface.connected {
            return None;
        }
        let mut size = 0u32;
        let mut data: *mut c_void = std::ptr::null_mut();
        // SAFETY: as in `radio_state`; the data is WLAN_CONNECTION_ATTRIBUTES.
        unsafe {
            let code = WlanQueryInterface(
                self.0,
                &interface.guid,
                wlan_intf_opcode_current_connection,
                None,
                &mut size,
                &mut data,
                None,
            );
            if code != 0 || data.is_null() {
                return None;
            }
            let mut ssid = None;
            if size as usize >= std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
                let attributes = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
                ssid = Some(ssid_bytes(&attributes.wlanAssociationAttributes.dot11Ssid))
                    .filter(|ssid| !ssid.is_empty());
            }
            WlanFreeMemory(data);
            ssid
        }
    }

    fn available_networks(
        &self,
        guid: &GUID,
        operation: &'static str,
    ) -> Result<Vec<AvailableEntry>, Error> {
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        // SAFETY: as in `interfaces`. Flags 0 return one entry per visible
        // network plus one per saved profile that matches it.
        unsafe {
            let code = WlanGetAvailableNetworkList(self.0, guid, 0, None, &mut list);
            if code != 0 {
                return Err(wlan_error(operation, code));
            }
            if list.is_null() {
                return Ok(Vec::new());
            }
            let count = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).Network.as_ptr(), count);
            let entries = items
                .iter()
                .filter(|item| item.dot11BssType == dot11_BSS_type_infrastructure)
                .filter_map(|item| {
                    let security = security_from_auth_algorithm(
                        item.dot11DefaultAuthAlgorithm.0,
                        item.bSecurityEnabled.as_bool(),
                    );
                    let id = WifiNetworkId::from_bytes(ssid_bytes(&item.dot11Ssid), security)?;
                    let profile = (item.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0)
                        .then(|| trim_wide(&item.strProfileName).to_vec())
                        .filter(|name| !name.is_empty());
                    Some(AvailableEntry {
                        id,
                        strength: item.wlanSignalQuality.min(100) as u8,
                        connected: item.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                        profile,
                    })
                })
                .collect::<Vec<_>>();
            WlanFreeMemory(list as *const c_void);
            Ok(entries)
        }
    }

    /// The interface's saved profiles. A profile whose XML cannot be read
    /// (group policy, permissions) is still listed, without a network.
    fn saved_profiles(&self, guid: &GUID) -> Vec<SavedProfile> {
        let mut list: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
        // SAFETY: as in `interfaces`.
        let names = unsafe {
            if WlanGetProfileList(self.0, guid, None, &mut list) != 0 || list.is_null() {
                return Vec::new();
            }
            let count = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).ProfileInfo.as_ptr(), count);
            let names = items
                .iter()
                .map(|item| trim_wide(&item.strProfileName).to_vec())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
            WlanFreeMemory(list as *const c_void);
            names
        };
        names
            .into_iter()
            .map(|name| {
                let id = self
                    .read_profile_xml(guid, &name)
                    .and_then(|xml| profile_network(&xml));
                SavedProfile { name, id }
            })
            .collect()
    }

    fn read_profile_xml(&self, guid: &GUID, name: &[u16]) -> Option<String> {
        let name = nul_terminated(name);
        let mut xml = windows::core::PWSTR::null();
        // SAFETY: `name` outlives the call; the returned string is
        // allocated by the WLAN API and freed after it is copied. Without
        // WLAN_PROFILE_GET_PLAINTEXT_KEY the key stays encrypted.
        unsafe {
            let code = WlanGetProfile(
                self.0,
                guid,
                PCWSTR(name.as_ptr()),
                None,
                &mut xml,
                None,
                None,
            );
            if code != 0 || xml.is_null() {
                return None;
            }
            let text = wide_ptr_to_string(xml.0);
            WlanFreeMemory(xml.0 as *const c_void);
            Some(text)
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

/// The interface's Windows connection name ("Wi-Fi"), which is also the
/// adapter's friendly name in the Network pane.
fn interface_alias(guid: &GUID) -> Option<String> {
    let mut luid = NET_LUID_LH::default();
    let mut alias = [0u16; 257];
    // SAFETY: plain out-parameters and a caller-owned buffer.
    unsafe {
        if ConvertInterfaceGuidToLuid(guid, &mut luid).0 != 0 {
            return None;
        }
        if ConvertInterfaceLuidToAlias(&luid, &mut alias).0 != 0 {
            return None;
        }
    }
    Some(wide_to_string(&alias)).filter(|alias| !alias.is_empty())
}

fn interface_luid(guid: &GUID) -> Option<u64> {
    let mut luid = NET_LUID_LH::default();
    // SAFETY: plain out-parameter; `Value` is the whole 64-bit LUID.
    unsafe {
        if ConvertInterfaceGuidToLuid(guid, &mut luid).0 != 0 {
            return None;
        }
        Some(luid.Value)
    }
}

// ---------------------------------------------------------------------------
// Connecting

/// A WLAN handle registered for ACM notifications, so a connection attempt
/// started on it reports its outcome on `events`.
struct ConnectSession {
    client: Client,
    context: *mut mpsc::Sender<ConnectEvent>,
    events: mpsc::Receiver<ConnectEvent>,
}

enum ConnectEvent {
    /// Some WLAN activity; a chance to notice cancellation.
    Progress,
    Finished {
        interface: GUID,
        ssid: Vec<u8>,
        reason: u32,
    },
    /// A scan on `interface` finished or failed.
    ScanDone { interface: GUID },
}

unsafe extern "system" fn on_connect_notification(
    data: *mut L2_NOTIFICATION_DATA,
    context: *mut c_void,
) {
    if data.is_null() || context.is_null() {
        return;
    }
    // SAFETY: `context` is the session's sender, freed only after the
    // handle is closed, which waits for running callbacks; `data` is valid
    // for the duration of the callback.
    let (sender, data) = unsafe { (&*(context as *const mpsc::Sender<ConnectEvent>), &*data) };
    let code = data.NotificationCode as i32;
    let finished = data.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM
        && (code == wlan_notification_acm_connection_complete.0
            || code == wlan_notification_acm_connection_attempt_fail.0);
    let scanned = data.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM
        && (code == wlan_notification_acm_scan_complete.0
            || code == wlan_notification_acm_scan_fail.0);
    let event = if scanned {
        ConnectEvent::ScanDone {
            interface: data.InterfaceGuid,
        }
    } else if finished
        && !data.pData.is_null()
        && data.dwDataSize as usize
            >= std::mem::offset_of!(WLAN_CONNECTION_NOTIFICATION_DATA, dwFlags)
    {
        // SAFETY: both codes carry WLAN_CONNECTION_NOTIFICATION_DATA.
        let connection = unsafe { &*(data.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA) };
        ConnectEvent::Finished {
            interface: data.InterfaceGuid,
            ssid: ssid_bytes(&connection.dot11Ssid),
            reason: connection.wlanReasonCode,
        }
    } else {
        ConnectEvent::Progress
    };
    let _ = sender.send(event);
}

impl ConnectSession {
    fn open(operation: &'static str) -> Result<Self, Error> {
        let client = Client::open_required(operation)?;
        let (sender, events) = mpsc::channel();
        let context = Box::into_raw(Box::new(sender));
        let session = Self {
            client,
            context,
            events,
        };
        // SAFETY: the callback reads `context` until the handle is closed
        // in Drop, before the sender is freed.
        let code = unsafe {
            WlanRegisterNotification(
                session.client.handle(),
                WLAN_NOTIFICATION_SOURCE_ACM | WLAN_NOTIFICATION_SOURCE_MSM,
                true,
                Some(on_connect_notification),
                Some(session.context as *const c_void),
                None,
                None,
            )
        };
        if code != 0 {
            return Err(wlan_error(operation, code));
        }
        Ok(session)
    }

    /// The Wi-Fi interface to join on, refusing when the radio is off.
    fn ready_interface(&self, operation: &'static str) -> Result<WlanInterface, Error> {
        let mut interfaces = self.client.interfaces(operation)?;
        let index = interfaces
            .iter()
            .position(|interface| interface.connected)
            .or_else(|| (!interfaces.is_empty()).then_some(0))
            .ok_or_else(|| Error::new(operation, "no Wi-Fi adapter found"))?;
        let interface = interfaces.swap_remove(index);
        if !radio_enabled(&self.client.radio_state(&interface.guid, operation)?) {
            return Err(Error::new(operation, "Wi-Fi is turned off"));
        }
        Ok(interface)
    }

    /// Saves an all-user profile and returns its name. Never overwrites an
    /// existing profile.
    fn add_profile(
        &self,
        guid: &GUID,
        xml: &str,
        ssid: &[u8],
        operation: &'static str,
    ) -> Result<Vec<u16>, Error> {
        let mut wide = nul_terminated_str(xml);
        let mut reason = 0u32;
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let code = unsafe {
            WlanSetProfile(
                self.client.handle(),
                guid,
                0,
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                false,
                None,
                &mut reason,
            )
        };
        wide.zeroize();
        if code == ERROR_ALREADY_EXISTS {
            return Err(Error::new(
                operation,
                "a saved network with this name already exists; forget it first",
            ));
        }
        if code != 0 {
            return Err(Error::new(
                operation,
                reason_text(reason).unwrap_or_else(|| wlan_error_text(code)),
            ));
        }
        Ok(profile_name(ssid).encode_utf16().collect())
    }

    fn delete_profile(&self, guid: &GUID, name: &[u16]) {
        let name = nul_terminated(name);
        // SAFETY: `name` is NUL-terminated and outlives the call.
        unsafe {
            let _ = WlanDeleteProfile(self.client.handle(), guid, PCWSTR(name.as_ptr()), None);
        }
    }

    fn connect_profile(
        &self,
        guid: &GUID,
        profile: &[u16],
        network: &WifiNetworkId,
        cancellation: Option<&WifiCancellation>,
        created: bool,
        operation: &'static str,
    ) -> Result<(), Error> {
        let name = nul_terminated(profile);
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(name.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };
        let result = self.start_and_wait(guid, &parameters, network, cancellation, operation);
        if result.is_err() && created {
            // A profile made for this attempt (for example with a mistyped
            // password) must not stay behind as a saved network.
            self.delete_profile(guid, profile);
        }
        result
    }

    fn connect_discovery(
        &self,
        guid: &GUID,
        network: &WifiNetworkId,
        operation: &'static str,
    ) -> Result<(), Error> {
        let mut ssid = dot11_ssid(&network.ssid);
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_discovery_secure,
            strProfile: PCWSTR::null(),
            pDot11Ssid: &mut ssid,
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };
        self.start_and_wait(guid, &parameters, network, None, operation)
    }

    fn start_and_wait(
        &self,
        guid: &GUID,
        parameters: &WLAN_CONNECTION_PARAMETERS,
        network: &WifiNetworkId,
        cancellation: Option<&WifiCancellation>,
        operation: &'static str,
    ) -> Result<(), Error> {
        // Earlier notifications on this handle (from listing networks)
        // belong to no attempt.
        while self.events.try_recv().is_ok() {}
        // SAFETY: `parameters` and the strings it points to outlive the call.
        let code = unsafe { WlanConnect(self.client.handle(), guid, parameters, None) };
        if code != 0 {
            return Err(wlan_error(operation, code));
        }
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            if cancellation.is_some_and(WifiCancellation::is_cancelled) {
                self.disconnect(guid);
                return Err(Error::cancelled(operation));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(remaining) {
                Ok(ConnectEvent::Finished {
                    interface,
                    ssid,
                    reason,
                }) if interface == *guid && ssid == network.ssid => {
                    if cancellation.is_some_and(WifiCancellation::is_cancelled) {
                        self.disconnect(guid);
                        return Err(Error::cancelled(operation));
                    }
                    return if reason == 0 {
                        Ok(())
                    } else {
                        Err(Error::new(
                            operation,
                            reason_text(reason)
                                .unwrap_or_else(|| "the connection attempt failed".to_string()),
                        ))
                    };
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.disconnect(guid);
                    return Err(Error::new(
                        operation,
                        "the connection did not finish in time",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Error::new(operation, "the Wi-Fi service stopped reporting"));
                }
            }
        }
    }

    fn disconnect(&self, guid: &GUID) {
        // SAFETY: plain call on an open handle.
        unsafe {
            let _ = WlanDisconnect(self.client.handle(), guid, None);
        }
    }
}

impl Drop for ConnectSession {
    fn drop(&mut self) {
        // SAFETY: unregistering, then closing the handle (which waits for
        // running callbacks), leaves no reader of `context` before it is
        // freed.
        unsafe {
            let _ = WlanRegisterNotification(
                self.client.handle(),
                WLAN_NOTIFICATION_SOURCE_NONE,
                true,
                None,
                None,
                None,
                None,
            );
        }
        self.client.close();
        // SAFETY: created by Box::into_raw in `open`, freed once.
        drop(unsafe { Box::from_raw(self.context) });
    }
}

// ---------------------------------------------------------------------------
// Watching

/// The watcher's registrations. Holds plain integers so the future that
/// owns it stays `Send`.
struct WatchRegistrations {
    wlan: Option<usize>,
    addresses: Option<usize>,
    context: usize,
}

unsafe extern "system" fn on_wlan_change(_data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // SAFETY: `context` is the watcher's sender, freed only after every
    // registration is cancelled.
    let sender = unsafe { &*(context as *const async_channel::Sender<WifiWatchEvent>) };
    // A full channel already holds a pending change, which is enough.
    let _ = sender.try_send(WifiWatchEvent::Changed);
}

unsafe extern "system" fn on_address_change(
    context: *const c_void,
    _row: *const MIB_UNICASTIPADDRESS_ROW,
    _kind: MIB_NOTIFICATION_TYPE,
) {
    if context.is_null() {
        return;
    }
    // SAFETY: as in `on_wlan_change`.
    let sender = unsafe { &*(context as *const async_channel::Sender<WifiWatchEvent>) };
    let _ = sender.try_send(WifiWatchEvent::Changed);
}

impl WatchRegistrations {
    fn register(sender: async_channel::Sender<WifiWatchEvent>) -> Self {
        let context = Box::into_raw(Box::new(sender)) as usize;
        let mut registrations = Self {
            wlan: None,
            addresses: None,
            context,
        };
        if let Ok(mut client) = Client::open() {
            // SAFETY: the callback reads `context` until the handle is
            // closed in Drop.
            let code = unsafe {
                WlanRegisterNotification(
                    client.handle(),
                    WLAN_NOTIFICATION_SOURCE_ACM | WLAN_NOTIFICATION_SOURCE_MSM,
                    true,
                    Some(on_wlan_change),
                    Some(context as *const c_void),
                    None,
                    None,
                )
            };
            if code == 0 {
                registrations.wlan = Some(client.handle().0 as usize);
                // Ownership moves to the registrations.
                client.0 = HANDLE::default();
            }
        }
        let mut handle = HANDLE::default();
        // SAFETY: the callback reads `context` until the registration is
        // cancelled in Drop.
        let code = unsafe {
            NotifyUnicastIpAddressChange(
                AF_UNSPEC,
                Some(on_address_change),
                Some(context as *const c_void),
                false,
                &mut handle,
            )
        };
        if code.0 == 0 {
            registrations.addresses = Some(handle.0 as usize);
        }
        registrations
    }

    fn any(&self) -> bool {
        self.wlan.is_some() || self.addresses.is_some()
    }
}

impl Drop for WatchRegistrations {
    fn drop(&mut self) {
        if let Some(handle) = self.wlan.take() {
            let mut client = Client(HANDLE(handle as *mut c_void));
            // SAFETY: unregister, then close (which waits for running
            // callbacks).
            unsafe {
                let _ = WlanRegisterNotification(
                    client.handle(),
                    WLAN_NOTIFICATION_SOURCE_NONE,
                    true,
                    None,
                    None,
                    None,
                    None,
                );
            }
            client.close();
        }
        if let Some(handle) = self.addresses.take() {
            // SAFETY: cancels the registration made in `register`; this
            // waits for a running callback to return.
            unsafe {
                let _ = CancelMibChangeNotify2(HANDLE(handle as *mut c_void));
            }
        }
        // SAFETY: created by Box::into_raw in `register`; no registration
        // can read it any more.
        drop(unsafe { Box::from_raw(self.context as *mut async_channel::Sender<WifiWatchEvent>) });
    }
}

pub(super) async fn system_watch_wifi(
    sender: async_channel::Sender<WifiWatchEvent>,
) -> Result<(), Error> {
    let registrations = WatchRegistrations::register(sender.clone());
    if !registrations.any() {
        drop(registrations);
        return sender
            .send(WifiWatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("watch Wi-Fi changes", "the event consumer closed"));
    }
    sender.closed().await;
    drop(registrations);
    Ok(())
}

// ---------------------------------------------------------------------------
// Network interfaces

pub(super) fn win_network_snapshot() -> Result<NetworkSnapshot, Error> {
    const OPERATION: &str = "read network interfaces";
    let buffer = adapter_addresses(OPERATION)?;
    let wlan_luids = wlan_interface_luids();
    let mut best_index = 0u32;
    // SAFETY: plain out-parameter. 1.1.1.1 reads the same in either byte
    // order; nothing is sent, Windows only consults its route table.
    let has_route =
        unsafe { GetBestInterface(u32::from_ne_bytes([1, 1, 1, 1]), &mut best_index) } == 0;

    let mut devices = Vec::new();
    let mut primary_connection = None;
    let mut adapter = if buffer.is_empty() {
        std::ptr::null()
    } else {
        buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH
    };
    // SAFETY: GetAdaptersAddresses filled `buffer` with a linked list of
    // adapters whose pointers stay inside it; it outlives this loop.
    unsafe {
        while !adapter.is_null() {
            let current = &*adapter;
            adapter = current.Next;
            let if_type = current.IfType;
            if if_type == IF_TYPE_SOFTWARE_LOOPBACK || if_type == IF_TYPE_TUNNEL {
                continue;
            }
            if current.OperStatus == IfOperStatusNotPresent {
                continue;
            }
            let kind = match if_type {
                IF_TYPE_ETHERNET_CSMACD => DeviceKind::Ethernet,
                IF_TYPE_IEEE80211 => DeviceKind::WiFi,
                _ => DeviceKind::Other,
            };
            let state = device_state(current.OperStatus);
            // Wi-Fi Direct and similar virtual 802.11 adapters are not
            // WLAN interfaces; other unknown adapters show only when up.
            if kind == DeviceKind::WiFi && !wlan_luids.contains(&current.Luid.Value) {
                continue;
            }
            if kind == DeviceKind::Other && state != DeviceState::Connected {
                continue;
            }
            let name = wide_ptr_to_string(current.FriendlyName.0);
            let interface = if name.is_empty() {
                wide_ptr_to_string(current.Description.0)
            } else {
                name
            };
            let if_index = current.Anonymous1.Anonymous.IfIndex;
            let primary = has_route && if_index == best_index && state.is_connected();

            let mut addresses = Vec::new();
            let mut unicast = current.FirstUnicastAddress;
            while !unicast.is_null() {
                let entry = &*unicast;
                unicast = entry.Next;
                if let Some(address) = socket_address(&entry.Address) {
                    if !is_link_local(&address) {
                        addresses.push(format_address(
                            &address.to_string(),
                            Some(u32::from(entry.OnLinkPrefixLength)),
                        ));
                    }
                }
            }
            let mut gateways = Vec::new();
            let mut gateway = current.FirstGatewayAddress;
            while !gateway.is_null() {
                let entry = &*gateway;
                gateway = entry.Next;
                if let Some(address) = socket_address(&entry.Address) {
                    gateways.push(address);
                }
            }
            // Prefer the IPv4 router, as the macOS and Linux views do.
            gateways.sort_by_key(|address| !address.is_ipv4());
            let mut dns = Vec::new();
            let mut server = current.FirstDnsServerAddress;
            while !server.is_null() {
                let entry = &*server;
                server = entry.Next;
                if let Some(address) = socket_address(&entry.Address) {
                    if !is_placeholder_dns(&address) {
                        let text = address.to_string();
                        if !dns.contains(&text) {
                            dns.push(text);
                        }
                    }
                }
            }
            let length =
                (current.PhysicalAddressLength as usize).min(current.PhysicalAddress.len());
            let hardware_address = hardware_address(&current.PhysicalAddress[..length]);
            if primary {
                primary_connection = Some(interface.clone());
            }
            devices.push(NetworkDevice {
                interface: interface.clone(),
                kind,
                state,
                connection: Some(interface),
                primary,
                addresses,
                gateway: gateways.first().map(ToString::to_string),
                dns,
                hardware_address,
                configuration: None,
                configuration_error: Some("Connection editing is not available on Windows".into()),
            });
        }
    }
    sort_devices(&mut devices);
    Ok(NetworkSnapshot {
        available: true,
        connectivity: connectivity(),
        primary_connection,
        devices,
    })
}

/// Reads the adapter list into an 8-byte-aligned buffer, growing it when
/// Windows asks for more room (the list can change between calls).
fn adapter_addresses(operation: &'static str) -> Result<Vec<u64>, Error> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size = 16 * 1024u32;
    for _ in 0..4 {
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let mut length = (buffer.len() * 8) as u32;
        // SAFETY: `buffer` is writable for `length` bytes and aligned for
        // IP_ADAPTER_ADDRESSES_LH.
        let code = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut length,
            )
        };
        match code {
            0 => return Ok(buffer),
            // ERROR_NO_DATA: no adapters at all.
            232 => return Ok(Vec::new()),
            code if code == ERROR_BUFFER_OVERFLOW.0 => size = length.max(size * 2),
            code => return Err(Error::new(operation, wlan_error_text(code))),
        }
    }
    Err(Error::new(operation, "the adapter list kept changing"))
}

fn wlan_interface_luids() -> Vec<u64> {
    let Ok(client) = Client::open() else {
        return Vec::new();
    };
    client
        .interfaces("read Wi-Fi interfaces")
        .unwrap_or_default()
        .iter()
        .filter_map(|interface| interface_luid(&interface.guid))
        .collect()
}

fn device_state(status: IF_OPER_STATUS) -> DeviceState {
    if status == IfOperStatusUp {
        DeviceState::Connected
    } else if status == IfOperStatusDown || status == IfOperStatusLowerLayerDown {
        DeviceState::Disconnected
    } else if status == IfOperStatusDormant || status == IfOperStatusTesting {
        DeviceState::Connecting
    } else if status == IfOperStatusNotPresent {
        DeviceState::Unavailable
    } else {
        DeviceState::Unknown
    }
}

fn connectivity() -> Connectivity {
    let mut hint = NL_NETWORK_CONNECTIVITY_HINT::default();
    // SAFETY: plain out-parameter.
    if unsafe { GetNetworkConnectivityHint(&mut hint) }.0 != 0 {
        return Connectivity::Unknown;
    }
    connectivity_from_hint(hint.ConnectivityLevel.0)
}

/// Reads an IPv4 or IPv6 address from a SOCKET_ADDRESS.
fn socket_address(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if address.lpSockaddr.is_null() || address.iSockaddrLength <= 0 {
        return None;
    }
    // SAFETY: Windows reports the sockaddr's length with it.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            address.lpSockaddr as *const u8,
            address.iSockaddrLength as usize,
        )
    };
    parse_sockaddr(bytes)
}

// ---------------------------------------------------------------------------
// VPN

pub(super) fn win_vpn_snapshot() -> VpnSnapshot {
    // Windows' own VPN connections are not exposed through this backend.
    VpnSnapshot {
        available: false,
        profiles: Vec::new(),
    }
}

pub(super) fn win_set_vpn_enabled(_id: &VpnProfileId, enabled: bool) -> Result<VpnSnapshot, Error> {
    Err(Error::new(
        if enabled {
            "connect VPN"
        } else {
            "disconnect VPN"
        },
        "VPN is not available on Windows",
    ))
}

// ---------------------------------------------------------------------------
// Errors and strings

fn wlan_error(operation: &'static str, code: u32) -> Error {
    Error::new(operation, wlan_error_text(code))
}

fn wlan_error_text(code: u32) -> String {
    let message = HRESULT::from_win32(code).message();
    let message = message.trim();
    if message.is_empty() {
        format!("Windows error {code}")
    } else {
        message.to_string()
    }
}

// The `windows` crate binds WlanReasonCodeToString with a shared `&[u16]`
// output buffer; declaring it here with a `*mut u16` keeps the write sound.
#[cfg_attr(
    target_arch = "x86",
    link(
        name = "wlanapi.dll",
        kind = "raw-dylib",
        modifiers = "+verbatim",
        import_name_type = "undecorated"
    )
)]
#[cfg_attr(
    not(target_arch = "x86"),
    link(name = "wlanapi.dll", kind = "raw-dylib", modifiers = "+verbatim")
)]
extern "system" {
    #[link_name = "WlanReasonCodeToString"]
    fn wlan_reason_code_to_string(
        reason: u32,
        buffer_size: u32,
        buffer: *mut u16,
        reserved: *const c_void,
    ) -> u32;
}

/// Windows' own wording for a WLAN reason code, such as a wrong key.
fn reason_text(reason: u32) -> Option<String> {
    if reason == 0 {
        return None;
    }
    let mut buffer = [0u16; 512];
    // SAFETY: the API writes at most `buffer.len()` characters, including
    // the terminating NUL, into the caller-owned buffer.
    let code = unsafe {
        wlan_reason_code_to_string(
            reason,
            buffer.len() as u32,
            buffer.as_mut_ptr(),
            std::ptr::null(),
        )
    };
    if code != 0 {
        return None;
    }
    Some(wide_to_string(&buffer))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn ssid_bytes(ssid: &DOT11_SSID) -> Vec<u8> {
    let length = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
    ssid.ucSSID[..length].to_vec()
}

fn dot11_ssid(bytes: &[u8]) -> DOT11_SSID {
    let mut ssid = DOT11_SSID::default();
    let length = bytes.len().min(ssid.ucSSID.len());
    ssid.ucSSID[..length].copy_from_slice(&bytes[..length]);
    ssid.uSSIDLength = length as u32;
    ssid
}

fn trim_wide(value: &[u16]) -> &[u16] {
    let end = value
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(value.len());
    &value[..end]
}

fn wide_to_string(value: &[u16]) -> String {
    String::from_utf16_lossy(trim_wide(value))
}

fn wide_ptr_to_string(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    // SAFETY: Windows hands out NUL-terminated strings.
    unsafe {
        while *value.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value, length))
    }
}

fn nul_terminated(value: &[u16]) -> Vec<u16> {
    let mut wide = trim_wide(value).to_vec();
    wide.push(0);
    wide
}

fn nul_terminated_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------
// Pure mapping helpers

/// The model's security class for a network's default authentication
/// algorithm (DOT11_AUTH_ALGORITHM) and its security flag.
fn security_from_auth_algorithm(algorithm: i32, security_enabled: bool) -> WifiSecurity {
    match algorithm {
        // DOT11_AUTH_ALGO_80211_OPEN: open, or static WEP when secured.
        1 if security_enabled => WifiSecurity::Legacy,
        1 => WifiSecurity::Open,
        // DOT11_AUTH_ALGO_80211_SHARED_KEY (WEP).
        2 => WifiSecurity::Legacy,
        // WPA, RSNA (WPA2), WPA3-Enterprise 192-bit and WPA3-Enterprise.
        3 | 6 | 8 | 11 => WifiSecurity::Enterprise,
        // WPA-PSK and RSNA-PSK (WPA2-Personal).
        4 | 7 => WifiSecurity::Personal(WifiPersonalMode::Psk),
        // WPA3-SAE.
        9 => WifiSecurity::Personal(WifiPersonalMode::Sae),
        // OWE (Enhanced Open).
        10 => WifiSecurity::EnhancedOpen,
        _ if security_enabled => WifiSecurity::Protected,
        _ => WifiSecurity::Open,
    }
}

/// The model's security class for a profile's `<authentication>` and
/// `<encryption>` values.
fn security_from_profile(authentication: &str, encryption: &str) -> WifiSecurity {
    match authentication {
        "open" if encryption.eq_ignore_ascii_case("WEP") => WifiSecurity::Legacy,
        "open" => WifiSecurity::Open,
        "shared" => WifiSecurity::Legacy,
        "WPAPSK" | "WPA2PSK" => WifiSecurity::Personal(WifiPersonalMode::Psk),
        "WPA3SAE" => WifiSecurity::Personal(WifiPersonalMode::Sae),
        "OWE" => WifiSecurity::EnhancedOpen,
        "WPA" | "WPA2" | "WPA3" | "WPA3ENT" | "WPA3ENT192" => WifiSecurity::Enterprise,
        _ => WifiSecurity::Protected,
    }
}

/// The network a WLAN profile joins: its SSID (hex form preferred, as
/// Windows does) and security class.
fn profile_network(xml: &str) -> Option<WifiNetworkId> {
    let ssid_config = element(xml, "SSIDConfig")?;
    let ssid = element(ssid_config, "SSID")?;
    let bytes = match element(ssid, "hex") {
        Some(hex) => decode_hex(hex.trim())?,
        None => xml_unescape(element(ssid, "name")?).into_bytes(),
    };
    let authentication = element(xml, "authentication").unwrap_or("open").trim();
    let encryption = element(xml, "encryption").unwrap_or("none").trim();
    WifiNetworkId::from_bytes(bytes, security_from_profile(authentication, encryption))
}

/// The text inside the first `<tag>…</tag>` (namespace-free, as Windows
/// writes profiles).
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    hex
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileAuth {
    Open,
    Wpa2Psk,
    Wpa3Sae,
}

/// The profile's name: the SSID as people read it.
fn profile_name(ssid: &[u8]) -> String {
    display_ssid(ssid)
}

/// A WLAN profile that joins `ssid` automatically. The SSID is given as
/// hex (exact bytes) and, when it is text, as a name too.
fn profile_xml(ssid: &[u8], auth: ProfileAuth, key: Option<&str>) -> String {
    let name = xml_escape(&profile_name(ssid));
    let ssid_name = std::str::from_utf8(ssid)
        .ok()
        .map(|text| format!("<name>{}</name>", xml_escape(text)))
        .unwrap_or_default();
    // The key's copies are wiped; only the returned profile holds it.
    let mut security = match (auth, key) {
        (ProfileAuth::Open, _) | (_, None) => {
            "<authEncryption><authentication>open</authentication>\
             <encryption>none</encryption><useOneX>false</useOneX></authEncryption>"
                .to_string()
        }
        (auth, Some(key)) => {
            let authentication = if auth == ProfileAuth::Wpa3Sae {
                "WPA3SAE"
            } else {
                "WPA2PSK"
            };
            // A 64-digit hex string is the raw PSK, not a passphrase.
            let key_type = if auth == ProfileAuth::Wpa2Psk
                && key.len() == 64
                && key.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                "networkKey"
            } else {
                "passPhrase"
            };
            let mut key = xml_escape(key);
            let security = format!(
                "<authEncryption><authentication>{authentication}</authentication>\
                 <encryption>AES</encryption><useOneX>false</useOneX></authEncryption>\
                 <sharedKey><keyType>{key_type}</keyType><protected>false</protected>\
                 <keyMaterial>{key}</keyMaterial></sharedKey>"
            );
            key.zeroize();
            security
        }
    };
    let profile = format!(
        "<?xml version=\"1.0\"?>\
         <WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\
         <name>{name}</name>\
         <SSIDConfig><SSID><hex>{hex}</hex>{ssid_name}</SSID></SSIDConfig>\
         <connectionType>ESS</connectionType>\
         <connectionMode>auto</connectionMode>\
         <MSM><security>{security}</security></MSM>\
         </WLANProfile>",
        hex = encode_hex(ssid),
    );
    security.zeroize();
    profile
}

/// Reads a sockaddr's address by its documented layout: the family at
/// offset 0, IPv4 at offset 4, IPv6 at offset 8.
fn parse_sockaddr(bytes: &[u8]) -> Option<IpAddr> {
    const AF_INET: u16 = 2;
    const AF_INET6: u16 = 23;
    let family = u16::from_ne_bytes(bytes.get(0..2)?.try_into().ok()?);
    match family {
        AF_INET => {
            let octets: [u8; 4] = bytes.get(4..8)?.try_into().ok()?;
            Some(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        AF_INET6 => {
            let octets: [u8; 16] = bytes.get(8..24)?.try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

fn is_link_local(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(_) => false,
        IpAddr::V6(address) => address.segments()[0] & 0xffc0 == 0xfe80,
    }
}

/// Windows lists fec0:0:0:ffff::1-3 on interfaces with no IPv6 DNS
/// configured; they are not real servers.
fn is_placeholder_dns(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(_) => false,
        IpAddr::V6(address) => {
            let segments = address.segments();
            segments[..4] == [0xfec0, 0, 0, 0xffff] && segments[4..7] == [0, 0, 0]
        }
    }
}

fn hardware_address(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.iter().all(|&byte| byte == 0) {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// NL_NETWORK_CONNECTIVITY_LEVEL_HINT to the model's reachability.
fn connectivity_from_hint(level: i32) -> Connectivity {
    match level {
        1 => Connectivity::None,
        2 => Connectivity::Limited,
        3 => Connectivity::Full,
        4 => Connectivity::Portal,
        _ => Connectivity::Unknown,
    }
}

#[cfg(test)]
mod win_tests {
    use super::*;

    #[test]
    fn auth_algorithms_map_to_security_classes() {
        assert_eq!(security_from_auth_algorithm(1, false), WifiSecurity::Open);
        assert_eq!(security_from_auth_algorithm(1, true), WifiSecurity::Legacy);
        assert_eq!(
            security_from_auth_algorithm(7, true),
            WifiSecurity::Personal(WifiPersonalMode::Psk)
        );
        assert_eq!(
            security_from_auth_algorithm(9, true),
            WifiSecurity::Personal(WifiPersonalMode::Sae)
        );
        assert_eq!(
            security_from_auth_algorithm(6, true),
            WifiSecurity::Enterprise
        );
        assert_eq!(
            security_from_auth_algorithm(10, true),
            WifiSecurity::EnhancedOpen
        );
    }

    #[test]
    fn profile_xml_round_trips_to_the_same_network() {
        let ssid = b"Caf\xc3\xa9 <&>".to_vec();
        let xml = profile_xml(&ssid, ProfileAuth::Wpa2Psk, Some("pa<ss>&word"));
        assert!(xml.contains("<hex>436166C3A9203C263E</hex>"));
        assert!(xml.contains("<keyMaterial>pa&lt;ss&gt;&amp;word</keyMaterial>"));
        assert!(xml.contains("<keyType>passPhrase</keyType>"));
        let id = profile_network(&xml).expect("profile network");
        assert_eq!(id.ssid, ssid);
        assert_eq!(id.security, WifiSecurity::Personal(WifiPersonalMode::Psk));

        let sae = profile_xml(b"Home", ProfileAuth::Wpa3Sae, Some("secret"));
        assert!(sae.contains("<authentication>WPA3SAE</authentication>"));
        let open = profile_xml(b"Guest", ProfileAuth::Open, None);
        assert_eq!(
            profile_network(&open).map(|id| id.security),
            Some(WifiSecurity::Open)
        );
    }

    #[test]
    fn raw_hex_keys_are_network_keys() {
        let key = "a".repeat(64);
        let xml = profile_xml(b"Home", ProfileAuth::Wpa2Psk, Some(&key));
        assert!(xml.contains("<keyType>networkKey</keyType>"));
    }

    #[test]
    fn profile_without_hex_uses_the_ssid_name() {
        let xml = "<WLANProfile><name>Profile</name><SSIDConfig><SSID><name>A&amp;B</name>\
                   </SSID></SSIDConfig><MSM><security><authEncryption>\
                   <authentication>WPA2</authentication><encryption>AES</encryption>\
                   </authEncryption></security></MSM></WLANProfile>";
        let id = profile_network(xml).expect("profile network");
        assert_eq!(id.ssid, b"A&B");
        assert_eq!(id.security, WifiSecurity::Enterprise);
    }

    #[test]
    fn sockaddrs_are_read_by_family() {
        let mut v4 = [0u8; 16];
        v4[..2].copy_from_slice(&2u16.to_ne_bytes());
        v4[4..8].copy_from_slice(&[192, 168, 1, 20]);
        assert_eq!(parse_sockaddr(&v4), Some("192.168.1.20".parse().unwrap()));
        let mut v6 = [0u8; 28];
        v6[..2].copy_from_slice(&23u16.to_ne_bytes());
        v6[8..24].copy_from_slice(&"fe80::1".parse::<Ipv6Addr>().unwrap().octets());
        let address = parse_sockaddr(&v6).unwrap();
        assert!(is_link_local(&address));
        assert!(is_placeholder_dns(&"fec0:0:0:ffff::1".parse().unwrap()));
        assert!(!is_placeholder_dns(
            &"2001:4860:4860::8888".parse().unwrap()
        ));
    }

    #[test]
    fn hardware_addresses_are_colon_separated() {
        assert_eq!(
            hardware_address(&[0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]).as_deref(),
            Some("00:1A:2B:3C:4D:5E")
        );
        assert_eq!(hardware_address(&[]), None);
    }
}
