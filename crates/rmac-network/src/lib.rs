//! Cross-platform Wi-Fi state and radio control.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(not(target_os = "macos"))]
use std::time::Duration;

#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(any(not(target_os = "macos"), test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod network_editor;
mod normalize;
#[cfg(any(not(target_os = "macos"), test))]
mod secret_agent;
mod vpn_delete;
mod vpn_editor;
mod vpn_import;
mod vpn_secrets;

#[cfg(any(not(target_os = "macos"), test))]
use linux::*;
#[cfg(target_os = "macos")]
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

#[cfg(not(target_os = "macos"))]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    linux_network_snapshot()
}

#[cfg(target_os = "macos")]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    macos_network_snapshot()
}

#[cfg(not(target_os = "macos"))]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    linux_vpn_snapshot()
}

#[cfg(target_os = "macos")]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    macos_vpn_snapshot()
}

#[cfg(not(target_os = "macos"))]
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    linux_set_vpn_enabled(id, enabled, cancellation)
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
async fn system_watch_wifi(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    sender
        .send(WifiWatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch Wi-Fi changes", "the event consumer closed"))
}

#[cfg(test)]
mod tests;
