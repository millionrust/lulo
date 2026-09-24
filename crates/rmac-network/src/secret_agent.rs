use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(not(target_os = "macos"))]
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

use super::{
    property_string, wifi_profile_identity, WifiEnterpriseCredentials, WifiNetworkId, WifiPassword,
    WifiPersonalMode, WifiSecurity,
};

#[cfg(not(target_os = "macos"))]
const SERVICE: &str = "org.freedesktop.NetworkManager";
#[cfg(not(target_os = "macos"))]
const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
#[cfg(not(target_os = "macos"))]
const AGENT_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/AgentManager";
#[cfg(not(target_os = "macos"))]
const AGENT_MANAGER_INTERFACE: &str = "org.freedesktop.NetworkManager.AgentManager";
#[cfg(not(target_os = "macos"))]
const AGENT_IDENTIFIER: &str = "org.rmac.SystemSettings.Wifi";
const SECURITY_SETTING: &str = "802-11-wireless-security";
const ENTERPRISE_SETTING: &str = "802-1x";

pub(super) type SettingsMap = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Debug, PartialEq, zbus::DBusError)]
#[zbus(
    prefix = "org.freedesktop.NetworkManager.SecretManager",
    impl_display = true
)]
enum SecretAgentError {
    Failed(String),
    InvalidConnection(String),
    AgentCanceled(String),
    NoSecrets(String),
}

struct AgentState {
    network: WifiNetworkId,
    profile_uuid: String,
    secret: Option<WifiSecret>,
    canceled: bool,
}

enum WifiSecret {
    Personal(WifiPassword),
    Enterprise(WifiEnterpriseCredentials),
}

struct OneShotSecretAgent {
    state: Mutex<AgentState>,
}

impl OneShotSecretAgent {
    fn new(network: WifiNetworkId, profile_uuid: String, secret: WifiSecret) -> Self {
        Self {
            state: Mutex::new(AgentState {
                network,
                profile_uuid,
                secret: Some(secret),
                canceled: false,
            }),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl OneShotSecretAgent {
    fn get_secrets(
        &self,
        connection: SettingsMap,
        _connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        _flags: u32,
    ) -> Result<SettingsMap, SecretAgentError> {
        let Some((requested, _)) = wifi_profile_identity(&connection) else {
            return Err(SecretAgentError::InvalidConnection(
                "the requested connection is not a valid Wi-Fi profile".to_string(),
            ));
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| SecretAgentError::Failed("the one-shot agent state was lost".into()))?;
        if state.canceled {
            return Err(SecretAgentError::AgentCanceled(
                "the secret request was canceled".to_string(),
            ));
        }
        if !state.network.matches_profile(&requested) {
            return Err(SecretAgentError::InvalidConnection(
                "the secret request did not match the selected network".to_string(),
            ));
        }
        let requested_uuid = connection
            .get("connection")
            .and_then(|settings| property_string(settings, "uuid"));
        if requested_uuid.as_deref() != Some(state.profile_uuid.as_str()) {
            return Err(SecretAgentError::InvalidConnection(
                "the secret request did not match the prepared profile".to_string(),
            ));
        }
        let expected_setting = match state.secret.as_ref() {
            Some(WifiSecret::Personal(_)) => SECURITY_SETTING,
            Some(WifiSecret::Enterprise(_)) => ENTERPRISE_SETTING,
            None => {
                return Err(SecretAgentError::NoSecrets(
                    "the one-shot Wi-Fi secret was already consumed".into(),
                ));
            }
        };
        if setting_name != expected_setting {
            return Err(SecretAgentError::NoSecrets(
                "Lulo OS has no secret for the requested setting".to_string(),
            ));
        }
        let secret = state.secret.take().ok_or_else(|| {
            SecretAgentError::NoSecrets("the one-shot Wi-Fi secret was already consumed".into())
        })?;
        let password = match &secret {
            WifiSecret::Personal(password) => password.expose(str::to_owned),
            WifiSecret::Enterprise(credentials) => credentials.expose_password(str::to_owned),
        };
        Ok(HashMap::from([(
            expected_setting.to_string(),
            HashMap::from([
                (
                    "name".to_string(),
                    OwnedValue::from(Str::from(expected_setting.to_string())),
                ),
                (
                    match secret {
                        WifiSecret::Personal(_) => "psk",
                        WifiSecret::Enterprise(_) => "password",
                    }
                    .to_string(),
                    OwnedValue::from(Str::from(password)),
                ),
            ]),
        )]))
    }

    fn cancel_get_secrets(
        &self,
        _connection_path: OwnedObjectPath,
        _setting_name: String,
    ) -> Result<(), SecretAgentError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SecretAgentError::Failed("the one-shot agent state was lost".into()))?;
        state.canceled = true;
        state.secret = None;
        Ok(())
    }

    fn save_secrets(
        &self,
        _connection: SettingsMap,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), SecretAgentError> {
        // rmac requests system-owned storage. NetworkManager, not this
        // ephemeral agent, persists that secret according to its policy.
        Ok(())
    }

    fn delete_secrets(
        &self,
        _connection: SettingsMap,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), SecretAgentError> {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) struct RegisteredSecretAgent {
    connection: Connection,
}

#[cfg(not(target_os = "macos"))]
impl RegisteredSecretAgent {
    fn register(
        network: WifiNetworkId,
        profile_uuid: String,
        secret: WifiSecret,
    ) -> zbus::Result<Self> {
        let connection = Builder::system()?
            .serve_at(
                AGENT_PATH,
                OneShotSecretAgent::new(network, profile_uuid, secret),
            )?
            .build()?;
        agent_manager(&connection)?
            .call::<_, _, ()>("RegisterWithCapabilities", &(AGENT_IDENTIFIER, 0_u32))?;
        Ok(Self { connection })
    }

    pub(super) fn register_personal(
        network: WifiNetworkId,
        profile_uuid: String,
        password: WifiPassword,
    ) -> zbus::Result<Self> {
        Self::register(network, profile_uuid, WifiSecret::Personal(password))
    }

    pub(super) fn register_enterprise(
        network: WifiNetworkId,
        profile_uuid: String,
        credentials: WifiEnterpriseCredentials,
    ) -> zbus::Result<Self> {
        Self::register(network, profile_uuid, WifiSecret::Enterprise(credentials))
    }

    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for RegisteredSecretAgent {
    fn drop(&mut self) {
        if let Ok(proxy) = agent_manager(&self.connection) {
            let _ = proxy.call::<_, _, ()>("Unregister", &());
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn agent_manager(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(
        connection,
        SERVICE,
        AGENT_MANAGER_PATH,
        AGENT_MANAGER_INTERFACE,
    )
}

pub(super) fn personal_connection_template(
    network: &WifiNetworkId,
    profile_uuid: &str,
) -> Result<SettingsMap, zbus::zvariant::Error> {
    let key_management = match network.security {
        WifiSecurity::Personal(WifiPersonalMode::Psk | WifiPersonalMode::Transition) => "wpa-psk",
        WifiSecurity::Personal(WifiPersonalMode::Sae) => "sae",
        _ => return Ok(SettingsMap::new()),
    };
    let ssid = OwnedValue::try_from(Value::new(network.ssid.clone()))?;
    Ok(HashMap::from([
        (
            "connection".to_string(),
            connection_setting(network, profile_uuid),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([
                ("ssid".to_string(), ssid),
                (
                    "security".to_string(),
                    OwnedValue::from(Str::from(SECURITY_SETTING.to_string())),
                ),
            ]),
        ),
        (
            SECURITY_SETTING.to_string(),
            HashMap::from([
                (
                    "key-mgmt".to_string(),
                    OwnedValue::from(Str::from(key_management.to_string())),
                ),
                // System-owned storage delegates persistence to NetworkManager
                // and avoids inventing an rmac credential store.
                ("psk-flags".to_string(), OwnedValue::from(0_u32)),
            ]),
        ),
    ]))
}

pub(super) fn enterprise_connection_template(
    network: &WifiNetworkId,
    profile_uuid: &str,
    credentials: &WifiEnterpriseCredentials,
) -> Result<SettingsMap, zbus::zvariant::Error> {
    if network.security != WifiSecurity::Enterprise {
        return Ok(SettingsMap::new());
    }
    let ssid = OwnedValue::try_from(Value::new(network.ssid.clone()))?;
    let eap = OwnedValue::try_from(Value::new(vec!["peap".to_string()]))?;
    let mut enterprise = HashMap::from([
        ("eap".to_string(), eap),
        (
            "identity".to_string(),
            OwnedValue::from(Str::from(credentials.identity.clone())),
        ),
        (
            "phase2-auth".to_string(),
            OwnedValue::from(Str::from("mschapv2".to_string())),
        ),
        ("system-ca-certs".to_string(), OwnedValue::from(true)),
        (
            "domain-suffix-match".to_string(),
            OwnedValue::from(Str::from(credentials.domain_suffix.clone())),
        ),
        ("password-flags".to_string(), OwnedValue::from(0_u32)),
    ]);
    if let Some(anonymous_identity) = &credentials.anonymous_identity {
        enterprise.insert(
            "anonymous-identity".to_string(),
            OwnedValue::from(Str::from(anonymous_identity.clone())),
        );
    }
    Ok(HashMap::from([
        (
            "connection".to_string(),
            connection_setting(network, profile_uuid),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([
                ("ssid".to_string(), ssid),
                (
                    "security".to_string(),
                    OwnedValue::from(Str::from(SECURITY_SETTING.to_string())),
                ),
            ]),
        ),
        (
            SECURITY_SETTING.to_string(),
            HashMap::from([(
                "key-mgmt".to_string(),
                OwnedValue::from(Str::from("wpa-eap".to_string())),
            )]),
        ),
        (ENTERPRISE_SETTING.to_string(), enterprise),
    ]))
}

fn connection_setting(network: &WifiNetworkId, profile_uuid: &str) -> HashMap<String, OwnedValue> {
    HashMap::from([
        (
            "id".to_string(),
            OwnedValue::from(Str::from(super::display_ssid(&network.ssid))),
        ),
        (
            "uuid".to_string(),
            OwnedValue::from(Str::from(profile_uuid.to_string())),
        ),
        (
            "type".to_string(),
            OwnedValue::from(Str::from("802-11-wireless".to_string())),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::super::property;
    use super::*;

    fn network(name: &[u8]) -> WifiNetworkId {
        WifiNetworkId::from_bytes(name.to_vec(), WifiSecurity::Personal(WifiPersonalMode::Psk))
            .unwrap()
    }

    #[test]
    fn one_shot_agent_returns_only_the_matching_personal_secret_once() {
        let selected = network(b"Studio");
        let profile_uuid = "8245bbca-c835-42b6-a37f-1202eaa5e47c";
        let agent = OneShotSecretAgent::new(
            selected.clone(),
            profile_uuid.to_string(),
            WifiSecret::Personal(
                WifiPassword::new("correct-horse".to_string(), &selected).unwrap(),
            ),
        );
        let template = personal_connection_template(&selected, profile_uuid).unwrap();
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/1").unwrap();

        let secrets = agent
            .get_secrets(
                template.clone(),
                path.clone(),
                SECURITY_SETTING.to_string(),
                vec!["psk".to_string()],
                0x5,
            )
            .unwrap();
        let psk = <&str>::try_from(&secrets[SECURITY_SETTING]["psk"]).unwrap();
        assert_eq!(psk, "correct-horse");
        assert!(matches!(
            agent.get_secrets(template, path, SECURITY_SETTING.to_string(), Vec::new(), 0,),
            Err(SecretAgentError::NoSecrets(_))
        ));
    }

    #[test]
    fn one_shot_agent_rejects_a_different_network_without_consuming_secret() {
        let selected = network(b"Studio");
        let other = network(b"Visitor");
        let profile_uuid = "8b7f0b4d-7f96-4ca7-9dde-0d862e3996b2";
        let agent = OneShotSecretAgent::new(
            selected.clone(),
            profile_uuid.to_string(),
            WifiSecret::Personal(
                WifiPassword::new("correct-horse".to_string(), &selected).unwrap(),
            ),
        );
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/2").unwrap();
        assert!(matches!(
            agent.get_secrets(
                personal_connection_template(&other, profile_uuid).unwrap(),
                path,
                SECURITY_SETTING.to_string(),
                Vec::new(),
                0,
            ),
            Err(SecretAgentError::InvalidConnection(_))
        ));
    }

    #[test]
    fn one_shot_agent_binds_a_secret_to_the_prepared_profile_uuid() {
        let selected = network(b"Studio");
        let expected_uuid = "cd0458c8-12bc-43f9-beb4-cbc8fe8f95ad";
        let agent = OneShotSecretAgent::new(
            selected.clone(),
            expected_uuid.to_string(),
            WifiSecret::Personal(
                WifiPassword::new("correct-horse".to_string(), &selected).unwrap(),
            ),
        );
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/3").unwrap();
        assert!(matches!(
            agent.get_secrets(
                personal_connection_template(&selected, "71e4f01d-2aad-424a-b567-cea230284d54")
                    .unwrap(),
                path.clone(),
                SECURITY_SETTING.to_string(),
                Vec::new(),
                0,
            ),
            Err(SecretAgentError::InvalidConnection(_))
        ));
        assert!(agent
            .get_secrets(
                personal_connection_template(&selected, expected_uuid).unwrap(),
                path,
                SECURITY_SETTING.to_string(),
                Vec::new(),
                0,
            )
            .is_ok());
    }

    #[test]
    fn enterprise_template_requires_system_trust_and_exact_server_domain() {
        let selected =
            WifiNetworkId::from_bytes(b"Company".to_vec(), WifiSecurity::Enterprise).unwrap();
        let profile_uuid = "67d2cd48-83d8-4c13-8b3c-b21d86bcf532";
        let credentials = WifiEnterpriseCredentials::new(
            "person@example.com".into(),
            "anonymous@example.com".into(),
            "radius.example.com".into(),
            "private password".into(),
            &selected,
        )
        .unwrap();
        let template =
            enterprise_connection_template(&selected, profile_uuid, &credentials).unwrap();
        let enterprise = &template[ENTERPRISE_SETTING];
        assert_eq!(
            property_string(enterprise, "identity").as_deref(),
            Some("person@example.com")
        );
        assert_eq!(
            property_string(enterprise, "anonymous-identity").as_deref(),
            Some("anonymous@example.com")
        );
        assert_eq!(
            property_string(enterprise, "domain-suffix-match").as_deref(),
            Some("radius.example.com")
        );
        assert_eq!(
            property_string(enterprise, "phase2-auth").as_deref(),
            Some("mschapv2")
        );
        assert_eq!(property::<bool>(enterprise, "system-ca-certs"), Some(true));
        assert!(!enterprise.contains_key("password"));

        let agent = OneShotSecretAgent::new(
            selected,
            profile_uuid.to_string(),
            WifiSecret::Enterprise(credentials),
        );
        let secrets = agent
            .get_secrets(
                template,
                OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/4").unwrap(),
                ENTERPRISE_SETTING.to_string(),
                Vec::new(),
                0,
            )
            .unwrap();
        assert_eq!(
            <&str>::try_from(&secrets[ENTERPRISE_SETTING]["password"]).unwrap(),
            "private password"
        );
        assert!(!secrets[ENTERPRISE_SETTING].contains_key("psk"));
    }
}
