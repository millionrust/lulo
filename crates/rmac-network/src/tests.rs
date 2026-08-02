//! Focused network service contracts.

use super::*;
use zbus::zvariant::{DynamicType, OwnedValue, Str, Value};

fn owned<T>(value: T) -> OwnedValue
where
    T: Into<Value<'static>> + DynamicType,
{
    OwnedValue::try_from(Value::new(value)).unwrap()
}

fn string(value: &str) -> OwnedValue {
    OwnedValue::from(Str::from(value.to_owned()))
}

#[test]
fn networks_are_deduplicated_sorted_and_clamped() {
    let networks = normalize_networks(vec![
        RawNetwork {
            id: WifiNetworkId::from_bytes(
                b"Cafe".to_vec(),
                WifiSecurity::Personal(WifiPersonalMode::Psk),
            )
            .unwrap(),
            strength: 45,
            known: false,
            connected: false,
        },
        RawNetwork {
            id: WifiNetworkId::from_bytes(
                b"Home".to_vec(),
                WifiSecurity::Personal(WifiPersonalMode::Psk),
            )
            .unwrap(),
            strength: 150,
            known: true,
            connected: true,
        },
        RawNetwork {
            id: WifiNetworkId::from_bytes(
                b"Cafe".to_vec(),
                WifiSecurity::Personal(WifiPersonalMode::Psk),
            )
            .unwrap(),
            strength: 72,
            known: true,
            connected: false,
        },
        RawNetwork {
            id: WifiNetworkId::from_bytes(
                b"Home".to_vec(),
                WifiSecurity::Personal(WifiPersonalMode::Psk),
            )
            .unwrap(),
            strength: 200,
            known: false,
            connected: false,
        },
    ]);

    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].ssid, "Home");
    assert_eq!(networks[0].strength, 100);
    assert!(networks[0].known);
    assert!(!networks[0].can_connect());
    assert_eq!(networks[1].ssid, "Cafe");
    assert_eq!(networks[1].strength, 72);
    assert!(networks[1].security.is_secure());
    assert!(networks[1].known);
    assert!(networks[1].can_connect());
}

#[test]
fn open_and_protected_networks_with_the_same_ssid_stay_distinct() {
    let networks = normalize_networks(vec![
        RawNetwork {
            id: WifiNetworkId::from_bytes(b"Shared Name".to_vec(), WifiSecurity::Open).unwrap(),
            strength: 80,
            known: false,
            connected: false,
        },
        RawNetwork {
            id: WifiNetworkId::from_bytes(
                b"Shared Name".to_vec(),
                WifiSecurity::Personal(WifiPersonalMode::Psk),
            )
            .unwrap(),
            strength: 70,
            known: true,
            connected: false,
        },
    ]);

    assert_eq!(networks.len(), 2);
    assert!(!networks[0].security.is_secure());
    assert!(networks[0].can_connect());
    assert!(networks[1].security.is_secure());
    assert!(networks[1].known);
    assert!(networks[1].can_connect());
}

#[test]
fn wifi_network_ids_validate_length_and_redact_ssid_bytes() {
    assert!(WifiNetworkId::from_bytes(Vec::new(), WifiSecurity::Open).is_none());
    assert!(WifiNetworkId::from_bytes(vec![b'x'; 33], WifiSecurity::Open).is_none());
    let id = WifiNetworkId::from_bytes(
        b"Private Network".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let debug = format!("{id:?}");
    assert!(!debug.contains("Private Network"));
    assert!(debug.contains("ssid_bytes: 15"));
}

#[test]
fn access_point_security_flags_map_to_supported_flows() {
    assert_eq!(
        wifi_security_from_access_point(0, 0x100, 0),
        WifiSecurity::Personal(WifiPersonalMode::Psk)
    );
    assert_eq!(
        wifi_security_from_access_point(0, 0x100, 0x400),
        WifiSecurity::Personal(WifiPersonalMode::Transition)
    );
    assert_eq!(
        wifi_security_from_access_point(0, 0, 0x400),
        WifiSecurity::Personal(WifiPersonalMode::Sae)
    );
    assert_eq!(
        wifi_security_from_access_point(0, 0, 0x800),
        WifiSecurity::EnhancedOpen
    );
    assert_eq!(
        wifi_security_from_access_point(0, 0, 0x200),
        WifiSecurity::Enterprise
    );
    assert_eq!(
        wifi_security_from_access_point(1, 0, 0),
        WifiSecurity::Legacy
    );
    assert_eq!(wifi_security_from_access_point(0, 0, 0), WifiSecurity::Open);
}

#[test]
fn personal_passwords_are_validated_without_debug_exposure() {
    let psk = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    assert!(WifiPassword::new("12345678".to_string(), &psk).is_ok());
    assert!(WifiPassword::new("a".repeat(63), &psk).is_ok());
    assert!(WifiPassword::new("01".repeat(32), &psk).is_ok());
    assert!(WifiPassword::new("1234567".to_string(), &psk).is_err());
    assert!(WifiPassword::new("z".repeat(64), &psk).is_err());
    assert!(WifiPassword::new("password\n".to_string(), &psk).is_err());

    let password = WifiPassword::new("correct-horse".to_string(), &psk).unwrap();
    assert_eq!(format!("{password:?}"), "WifiPassword(<redacted>)");

    let sae = WifiNetworkId::from_bytes(
        b"Modern".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Sae),
    )
    .unwrap();
    assert!(WifiPassword::new("é".to_string(), &sae).is_ok());
    assert!(WifiPassword::new(String::new(), &sae).is_err());
    assert!(WifiPassword::new("a".repeat(64), &sae).is_err());
}

#[test]
fn enterprise_credentials_require_identity_password_and_certificate_domain() {
    let enterprise =
        WifiNetworkId::from_bytes(b"Company".to_vec(), WifiSecurity::Enterprise).unwrap();
    let credentials = WifiEnterpriseCredentials::new(
        "person@example.com".into(),
        "anonymous@example.com".into(),
        "RADIUS.Example.COM".into(),
        "private password".into(),
        &enterprise,
    )
    .unwrap();
    assert_eq!(credentials.identity, "person@example.com");
    assert_eq!(
        credentials.anonymous_identity.as_deref(),
        Some("anonymous@example.com")
    );
    assert_eq!(credentials.domain_suffix, "radius.example.com");
    assert_eq!(
        format!("{credentials:?}"),
        "WifiEnterpriseCredentials(<redacted>)"
    );

    assert!(matches!(
        WifiEnterpriseCredentials::new(
            " person@example.com".into(),
            String::new(),
            "radius.example.com".into(),
            "password".into(),
            &enterprise,
        ),
        Err(WifiEnterpriseCredentialsError::InvalidIdentity)
    ));
    assert!(matches!(
        WifiEnterpriseCredentials::new(
            "person@example.com".into(),
            " anonymous@example.com".into(),
            "radius.example.com".into(),
            "password".into(),
            &enterprise,
        ),
        Err(WifiEnterpriseCredentialsError::InvalidAnonymousIdentity)
    ));
    for invalid in [
        "",
        "localhost",
        ".example.com",
        "example.com.",
        "-radius.example.com",
        "radius..example.com",
        "radius_example.com",
        "rádius.example.com",
    ] {
        assert!(matches!(
            WifiEnterpriseCredentials::new(
                "person@example.com".into(),
                String::new(),
                invalid.into(),
                "password".into(),
                &enterprise,
            ),
            Err(WifiEnterpriseCredentialsError::InvalidDomain)
        ));
    }
    assert!(matches!(
        WifiEnterpriseCredentials::new(
            "person@example.com".into(),
            String::new(),
            "radius.example.com".into(),
            String::new(),
            &enterprise,
        ),
        Err(WifiEnterpriseCredentialsError::InvalidPassword)
    ));
    let personal = WifiNetworkId::from_bytes(
        b"Home".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    assert!(matches!(
        WifiEnterpriseCredentials::new(
            "person@example.com".into(),
            String::new(),
            "radius.example.com".into(),
            "password".into(),
            &personal,
        ),
        Err(WifiEnterpriseCredentialsError::UnsupportedSecurity)
    ));
}

#[test]
fn new_enterprise_network_routes_to_setup_while_known_profiles_activate_directly() {
    let id = WifiNetworkId::from_bytes(b"Company".to_vec(), WifiSecurity::Enterprise).unwrap();
    let mut network = WifiNetwork {
        id,
        ssid: "Company".into(),
        strength: 80,
        security: WifiSecurity::Enterprise,
        known: false,
        connected: false,
    };
    assert!(network.can_connect());
    assert!(network.needs_enterprise_setup());
    assert!(!network.needs_password());

    network.known = true;
    assert!(network.can_connect());
    assert!(!network.needs_enterprise_setup());
}

#[test]
fn transition_access_points_match_saved_personal_profiles() {
    let transition = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Transition),
    )
    .unwrap();
    let psk = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let sae = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Sae),
    )
    .unwrap();
    assert!(transition.matches_profile(&psk));
    assert!(transition.matches_profile(&sae));
}

#[test]
fn saved_networks_are_deduplicated_by_exact_identity_and_sorted_by_recency() {
    let studio_psk = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let studio_open = WifiNetworkId::from_bytes(b"Studio".to_vec(), WifiSecurity::Open).unwrap();
    let cafe = WifiNetworkId::from_bytes(b"Cafe".to_vec(), WifiSecurity::Open).unwrap();
    let saved = normalize_saved_networks([
        (studio_psk.clone(), 2),
        (studio_open.clone(), 1),
        (cafe.clone(), 5),
        (studio_psk.clone(), 9),
    ]);

    assert_eq!(saved.len(), 3);
    assert_eq!(saved[0].id, studio_psk);
    assert_eq!(saved[1].id, cafe);
    assert_eq!(saved[2].id, studio_open);
    assert_eq!(saved[0].ssid, "Studio");
}

#[test]
fn saved_profile_matching_never_crosses_ssid_or_security_identity() {
    let selected = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let other_ssid = WifiNetworkId::from_bytes(
        b"Visitor".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let open = WifiNetworkId::from_bytes(b"Studio".to_vec(), WifiSecurity::Open).unwrap();

    assert!(selected.matches_profile(&selected));
    assert!(!selected.matches_profile(&other_ssid));
    assert!(!selected.matches_profile(&open));
}

#[test]
fn saved_wifi_profiles_preserve_exact_ssid_security_and_recency() {
    let settings = HashMap::from([
        (
            "connection".to_string(),
            HashMap::from([
                ("type".to_string(), string("802-11-wireless")),
                ("timestamp".to_string(), owned(42_u64)),
            ]),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([("ssid".to_string(), owned(b"Studio".to_vec()))]),
        ),
        (
            "802-11-wireless-security".to_string(),
            HashMap::from([("key-mgmt".to_string(), string("wpa-psk"))]),
        ),
    ]);

    let (id, timestamp) = wifi_profile_identity(&settings).unwrap();
    assert_eq!(id.ssid, b"Studio");
    assert!(id.is_secure());
    assert_eq!(timestamp, 42);
}

#[test]
fn failed_join_cleanup_requires_exact_network_and_generated_profile_uuid() {
    let selected = WifiNetworkId::from_bytes(
        b"Studio".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    let uuid = "ae023b9a-d681-468d-8f9c-4e544ae8f569";
    let settings = secret_agent::personal_connection_template(&selected, uuid).unwrap();
    assert!(exact_created_wifi_profile(&settings, uuid, &selected));
    assert!(!exact_created_wifi_profile(
        &settings,
        "291cd974-5401-4fc5-af29-b2c460878429",
        &selected
    ));
    let other = WifiNetworkId::from_bytes(
        b"Visitor".to_vec(),
        WifiSecurity::Personal(WifiPersonalMode::Psk),
    )
    .unwrap();
    assert!(!exact_created_wifi_profile(&settings, uuid, &other));
}

#[test]
fn non_wifi_profiles_are_not_treated_as_known_networks() {
    let settings = HashMap::from([
        (
            "connection".to_string(),
            HashMap::from([("type".to_string(), string("802-3-ethernet"))]),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([("ssid".to_string(), owned(b"Studio".to_vec()))]),
        ),
    ]);

    assert!(wifi_profile_identity(&settings).is_none());
}

#[test]
fn errors_preserve_operation_context() {
    let error = Error::new("read Wi-Fi state", "service unavailable");
    assert_eq!(
        error.to_string(),
        "could not read Wi-Fi state: service unavailable"
    );
}

#[test]
fn network_manager_owner_changes_distinguish_outage_and_recovery() {
    assert_eq!(
        network_manager_owner_availability("org.freedesktop.NetworkManager", ""),
        Some(false)
    );
    assert_eq!(
        network_manager_owner_availability("org.freedesktop.NetworkManager", ":1.42"),
        Some(true)
    );
    assert_eq!(
        network_manager_owner_availability("org.example.Other", ":1.42"),
        None
    );
}

#[test]
fn network_manager_values_map_to_stable_ui_states() {
    assert_eq!(connectivity_from_network_manager(4), Connectivity::Full);
    assert_eq!(connectivity_from_network_manager(99), Connectivity::Unknown);
    assert_eq!(
        device_state_from_network_manager(70),
        DeviceState::Connecting
    );
    assert_eq!(
        device_state_from_network_manager(100),
        DeviceState::Connected
    );
    assert_eq!(device_state_from_network_manager(120), DeviceState::Failed);
}

#[test]
fn address_prefix_is_preserved_when_available() {
    assert_eq!(format_address("192.0.2.4", Some(24)), "192.0.2.4/24");
    assert_eq!(format_address("2001:db8::1", None), "2001:db8::1");
}

#[test]
fn devices_are_sorted_by_primary_connection_and_state() {
    let make = |interface: &str, kind, state, primary| NetworkDevice {
        interface: interface.to_string(),
        kind,
        state,
        connection: None,
        primary,
        addresses: Vec::new(),
        gateway: None,
        dns: Vec::new(),
        hardware_address: None,
        configuration: None,
        configuration_error: None,
    };
    let mut devices = vec![
        make("wlan0", DeviceKind::WiFi, DeviceState::Connected, false),
        make(
            "eth1",
            DeviceKind::Ethernet,
            DeviceState::Disconnected,
            false,
        ),
        make("eth0", DeviceKind::Ethernet, DeviceState::Connected, true),
    ];
    sort_devices(&mut devices);
    assert_eq!(devices[0].interface, "eth0");
    assert_eq!(devices[1].interface, "wlan0");
    assert_eq!(devices[2].interface, "eth1");
}

#[test]
fn vpn_types_states_and_services_are_normalized() {
    assert!(is_vpn_connection_type("vpn"));
    assert!(is_vpn_connection_type("wireguard"));
    assert!(!is_vpn_connection_type("802-3-ethernet"));
    assert_eq!(vpn_state_from_network_manager(1), VpnState::Connecting);
    assert_eq!(vpn_state_from_network_manager(2), VpnState::Connected);
    assert_eq!(vpn_state_from_vpn_connection(2), VpnState::Connecting);
    assert_eq!(vpn_state_from_vpn_connection(5), VpnState::Connected);
    assert_eq!(vpn_state_from_vpn_connection(6), VpnState::Failed);
    assert_eq!(vpn_state_from_vpn_connection(7), VpnState::Disconnected);
    assert_eq!(vpn_state_from_vpn_connection(99), VpnState::Unknown);
    assert_eq!(
        vpn_service_label("vpn", Some("org.freedesktop.NetworkManager.openvpn")),
        "OpenVPN"
    );
    assert_eq!(vpn_service_label("wireguard", None), "WireGuard");
}

#[test]
fn vpn_profile_identity_is_opaque_and_cancellation_is_shared() {
    let id = VpnProfileId {
        object_path: "/org/freedesktop/NetworkManager/Settings/42".to_string(),
        uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
    };
    let debug = format!("{id:?}");
    assert_eq!(
        debug,
        "VpnProfileId { object: \"<redacted>\", uuid: \"<redacted>\" }"
    );
    assert!(!debug.contains("Settings/42"));
    assert!(!debug.contains("12345678"));

    let cancellation = VpnCancellation::new();
    let observer = cancellation.clone();
    assert!(!observer.is_cancelled());
    cancellation.cancel();
    assert!(observer.is_cancelled());
}

#[test]
fn macos_vpn_fixture_is_parsed_and_active_profiles_sort_first() {
    let profiles = parse_macos_vpn_profiles(
        "Available network connection services in the current set (*=enabled):\n\
             * (Disconnected) 11111111-1111-1111-1111-111111111111 Office [VPN:IPSec]\n\
             * (Connected) 22222222-2222-2222-2222-222222222222 Home Tunnel [VPN:L2TP]",
    );
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].name, "Home Tunnel");
    assert_eq!(profiles[0].state, VpnState::Connected);
    assert_eq!(profiles[0].service, "VPN:L2TP");
    assert_eq!(profiles[1].name, "Office");
}
