use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(not(target_os = "macos"))]
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

use super::{wifi_profile_identity, WifiNetworkId, WifiPassword, WifiPersonalMode, WifiSecurity};

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
    password: Option<WifiPassword>,
    canceled: bool,
}

struct OneShotSecretAgent {
    state: Mutex<AgentState>,
}

impl OneShotSecretAgent {
    fn new(network: WifiNetworkId, password: WifiPassword) -> Self {
        Self {
            state: Mutex::new(AgentState {
                network,
                password: Some(password),
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
        if setting_name != SECURITY_SETTING {
            return Err(SecretAgentError::NoSecrets(
                "rmac provides only Wi-Fi Personal secrets".to_string(),
            ));
        }
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
        let password = state.password.take().ok_or_else(|| {
            SecretAgentError::NoSecrets("the one-shot Wi-Fi secret was already consumed".into())
        })?;
        let password = password.expose(str::to_owned);
        Ok(HashMap::from([(
            SECURITY_SETTING.to_string(),
            HashMap::from([
                (
                    "name".to_string(),
                    OwnedValue::from(Str::from(SECURITY_SETTING.to_string())),
                ),
                ("psk".to_string(), OwnedValue::from(Str::from(password))),
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
        state.password = None;
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
    pub(super) fn register(network: WifiNetworkId, password: WifiPassword) -> zbus::Result<Self> {
        let connection = Builder::system()?
            .serve_at(AGENT_PATH, OneShotSecretAgent::new(network, password))?
            .build()?;
        agent_manager(&connection)?
            .call::<_, _, ()>("RegisterWithCapabilities", &(AGENT_IDENTIFIER, 0_u32))?;
        Ok(Self { connection })
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

pub(super) fn connection_template(
    network: &WifiNetworkId,
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
            HashMap::from([(
                "type".to_string(),
                OwnedValue::from(Str::from("802-11-wireless".to_string())),
            )]),
        ),
        (
            "802-11-wireless".to_string(),
            HashMap::from([("ssid".to_string(), ssid)]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn network(name: &[u8]) -> WifiNetworkId {
        WifiNetworkId::from_bytes(name.to_vec(), WifiSecurity::Personal(WifiPersonalMode::Psk))
            .unwrap()
    }

    #[test]
    fn one_shot_agent_returns_only_the_matching_personal_secret_once() {
        let selected = network(b"Studio");
        let agent = OneShotSecretAgent::new(
            selected.clone(),
            WifiPassword::new("correct-horse".to_string(), &selected).unwrap(),
        );
        let template = connection_template(&selected).unwrap();
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
        let agent = OneShotSecretAgent::new(
            selected.clone(),
            WifiPassword::new("correct-horse".to_string(), &selected).unwrap(),
        );
        let path = OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/2").unwrap();
        assert!(matches!(
            agent.get_secrets(
                connection_template(&other).unwrap(),
                path,
                SECURITY_SETTING.to_string(),
                Vec::new(),
                0,
            ),
            Err(SecretAgentError::InvalidConnection(_))
        ));
    }
}
