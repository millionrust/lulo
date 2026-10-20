//! Cross-platform Wi-Fi state and radio control.

#[cfg(any(target_os = "linux", windows, test))]
use std::collections::HashMap;
#[cfg(not(any(target_os = "linux", windows)))]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::time::Duration;

pub mod ask_to_join;
#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(not(any(target_os = "linux", windows)))]
mod macos;
mod model;
mod network_editor;
mod normalize;
#[cfg(any(target_os = "linux", test))]
mod secret_agent;
mod vpn_delete;
mod vpn_editor;
mod vpn_import;
mod vpn_secrets;
#[cfg(windows)]
mod win;

#[cfg(any(target_os = "linux", test))]
use linux::*;
#[cfg(not(any(target_os = "linux", windows)))]
use macos::*;
pub use model::*;
pub use network_editor::{
    IpAddress, IpConfiguration, IpFamily, IpMethod, NetworkConfiguration, NetworkConnectionId,
    NetworkEdit, NetworkValidationError, ProxyConfiguration, ProxyMethod,
};
use normalize::*;
pub use vpn_delete::{VpnDeletePreview, VpnDeletePreviewId};
pub use vpn_editor::{VpnEditValidationError, VpnProfileConfiguration, VpnProfileEdit};
pub use vpn_import::{
    VpnImportCapabilities, VpnImportCapability, VpnImportCapabilityId, VpnImportPreview,
    VpnImportPreviewId,
};
pub use vpn_secrets::{VpnSecretClearPreview, VpnSecretClearPreviewId};
#[cfg(windows)]
use win::*;

pub trait WifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error>;
    fn set_enabled(&self, enabled: bool) -> Result<(), Error>;
    fn request_scan(&self) -> Result<(), Error>;
    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error>;
    fn forget(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error>;
    fn connect_with_password(
        &self,
        network: &WifiNetworkId,
        password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error>;
    fn connect_enterprise(
        &self,
        network: &WifiNetworkId,
        credentials: WifiEnterpriseCredentials,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error>;
}

pub struct SystemWifiService;

pub fn snapshot() -> Result<WifiSnapshot, Error> {
    SystemWifiService.snapshot()
}

pub fn set_enabled(enabled: bool) -> Result<(), Error> {
    SystemWifiService.set_enabled(enabled)
}

pub fn request_scan() -> Result<(), Error> {
    SystemWifiService.request_scan()
}

pub fn connect(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    SystemWifiService.connect(network)
}

pub fn forget(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    SystemWifiService.forget(network)
}

pub async fn watch(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    system_watch_wifi(sender).await
}

pub fn connect_with_password(
    network: &WifiNetworkId,
    password: WifiPassword,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    SystemWifiService.connect_with_password(network, password, cancellation)
}

/// Turn Auto-Join on or off for a saved network (Wi-Fi ▸ Advanced… and the
/// known network's ⋯ menu): NetworkManager's `connection.autoconnect` on
/// Lulo OS, the profile's `connectionMode` on Windows.
pub fn set_auto_join(network: &WifiNetworkId, enabled: bool) -> Result<WifiSnapshot, Error> {
    #[cfg(target_os = "linux")]
    {
        linux_set_wifi_auto_join(network, enabled)
    }
    #[cfg(windows)]
    {
        win_set_wifi_auto_join(network, enabled)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (network, enabled);
        Err(Error::new(
            "change Auto-Join",
            "Auto-Join is not available here",
        ))
    }
}

/// Join a network that does not broadcast its name (Wi-Fi ▸ Other…): open
/// (`password` None) or Wi-Fi Personal. Lulo OS only.
pub fn join_hidden(
    network: &WifiNetworkId,
    password: Option<WifiPassword>,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    #[cfg(target_os = "linux")]
    {
        linux_join_hidden_wifi(network, password, cancellation)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (network, password, cancellation);
        Err(Error::new(
            "join the Wi-Fi network",
            "joining a hidden network is not available here",
        ))
    }
}

pub fn connect_enterprise(
    network: &WifiNetworkId,
    credentials: WifiEnterpriseCredentials,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    SystemWifiService.connect_enterprise(network, credentials, cancellation)
}

pub fn network_snapshot() -> Result<NetworkSnapshot, Error> {
    system_network_snapshot()
}

pub fn update_network_connection(edit: &NetworkEdit) -> Result<NetworkSnapshot, Error> {
    network_editor::system_update(edit)
}

pub fn vpn_snapshot() -> Result<VpnSnapshot, Error> {
    system_vpn_snapshot()
}

pub fn vpn_import_capabilities() -> VpnImportCapabilities {
    vpn_import::capabilities()
}

pub fn preview_vpn_import(
    capability: &VpnImportCapabilityId,
    path: &std::path::Path,
) -> Result<VpnImportPreview, Error> {
    vpn_import::preview(capability, path)
}

pub fn finish_vpn_import(preview: &VpnImportPreviewId, keep: bool) -> Result<VpnSnapshot, Error> {
    vpn_import::finish(preview, keep)
}

pub fn prepare_vpn_delete(id: &VpnProfileId) -> Result<VpnDeletePreview, Error> {
    vpn_delete::prepare(id)
}

pub fn delete_vpn_profile(preview: &VpnDeletePreviewId) -> Result<VpnSnapshot, Error> {
    vpn_delete::delete(preview)
}

pub fn vpn_profile_configuration(id: &VpnProfileId) -> Result<VpnProfileConfiguration, Error> {
    vpn_editor::configuration(id)
}

pub fn update_vpn_profile(edit: &VpnProfileEdit) -> Result<VpnSnapshot, Error> {
    vpn_editor::update(edit)
}

pub fn prepare_vpn_secret_clear(
    configuration: &VpnProfileConfiguration,
) -> Result<VpnSecretClearPreview, Error> {
    vpn_secrets::prepare(configuration.profile_id())
}

pub fn clear_vpn_profile_secrets(preview: &VpnSecretClearPreviewId) -> Result<VpnSnapshot, Error> {
    vpn_secrets::clear(preview)
}

pub fn set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    system_set_vpn_enabled(id, enabled, cancellation)
}

#[cfg(target_os = "linux")]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    linux_network_snapshot()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    macos_network_snapshot()
}

#[cfg(windows)]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    win_network_snapshot()
}

#[cfg(target_os = "linux")]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    linux_vpn_snapshot()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    macos_vpn_snapshot()
}

#[cfg(windows)]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    Ok(win_vpn_snapshot())
}

#[cfg(target_os = "linux")]
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    linux_set_vpn_enabled(id, enabled, cancellation)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    if enabled && cancellation.is_cancelled() {
        return Err(Error::cancelled("connect VPN"));
    }
    macos_set_vpn_enabled(id, enabled)
}

#[cfg(windows)]
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    if enabled && cancellation.is_cancelled() {
        return Err(Error::cancelled("connect VPN"));
    }
    win_set_vpn_enabled(id, enabled)
}

#[cfg(not(any(target_os = "linux", windows)))]
async fn system_watch_wifi(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    sender
        .send(WifiWatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch Wi-Fi changes", "the event consumer closed"))
}

#[cfg(test)]
mod tests;
