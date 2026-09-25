use rmac_updates::{
    Cancellation, Error, InstallPlan, InstallProgress, InstallResult, Request, Snapshot,
    SnapshotFuture, Source,
};

use crate::transaction::{
    packagekit_details_snapshot, packagekit_install, packagekit_prepare,
    packagekit_prepare_offline, packagekit_prepare_selection, packagekit_snapshot,
};

#[cfg(any(target_os = "linux", test))]
pub(crate) const PACKAGEKIT_DESTINATION: &str = "org.freedesktop.PackageKit";
#[cfg(target_os = "linux")]
pub(crate) const PACKAGEKIT_PATH: &str = "/org/freedesktop/PackageKit";
#[cfg(target_os = "linux")]
pub(crate) const PACKAGEKIT_INTERFACE: &str = "org.freedesktop.PackageKit";
#[cfg(target_os = "linux")]
pub(crate) const TRANSACTION_INTERFACE: &str = "org.freedesktop.PackageKit.Transaction";
#[cfg(any(target_os = "linux", test))]
pub(crate) const FILTER_NONE: u64 = 1 << 1;
#[cfg(any(target_os = "linux", test))]
pub(crate) const FLAG_ONLY_TRUSTED: u64 = 1 << 1;
#[cfg(any(target_os = "linux", test))]
pub(crate) const FLAG_SIMULATE: u64 = 1 << 2;
#[cfg(any(target_os = "linux", test))]
pub(crate) const FLAG_ONLY_DOWNLOAD: u64 = 1 << 3;
#[cfg(target_os = "linux")]
pub(crate) const OFFLINE_INTERFACE: &str = "org.freedesktop.PackageKit.Offline";
/// `pk-offline-update` reboots once the prepared update is installed.
#[cfg(any(target_os = "linux", test))]
pub(crate) const OFFLINE_ACTION_REBOOT: &str = "reboot";
#[cfg(any(target_os = "linux", test))]
pub(crate) const ROLE_UPDATE_PACKAGES: u64 = 1 << 22;
#[cfg(target_os = "linux")]
pub(crate) const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(target_os = "linux")]
pub(crate) const SIMULATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
#[cfg(target_os = "linux")]
pub(crate) const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4 * 60 * 60);
#[cfg(target_os = "linux")]
pub(crate) const INSTALL_STALL_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(10 * 60);
#[cfg(target_os = "linux")]
pub(crate) const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(target_os = "linux")]
pub(crate) const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(any(target_os = "linux", test))]
pub(crate) fn is_progress_change_member(member: &str) -> bool {
    matches!(member, "PropertiesChanged" | "Changed")
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSource;

impl Source for SystemSource {
    fn snapshot(&self, request: Request) -> SnapshotFuture<'_> {
        Box::pin(packagekit_snapshot(request))
    }
}

/// The update set plus what Software Update shows around it: download
/// sizes, the Lulo OS release notes, and the update prepared for the next
/// restart. Only the update set itself is required; the rest is best effort.
pub async fn snapshot(request: Request) -> Result<Snapshot, Error> {
    packagekit_details_snapshot(request).await
}

/// Refresh, resolve `selection` (the Lulo OS item and/or package IDs, plus
/// anything already prepared for restart), and simulate it with
/// `ONLY_TRUSTED`. No package has been downloaded or changed on return.
pub async fn prepare_selection(
    selection: Vec<String>,
    cancellation: Cancellation,
) -> Result<(Snapshot, InstallPlan), Error> {
    packagekit_prepare_selection(selection, cancellation).await
}

/// Revalidate `plan`, download exactly its requested updates with
/// `ONLY_TRUSTED | ONLY_DOWNLOAD` (no authorization needed), then trigger
/// PackageKit's offline update so `pk-offline-update` installs them from
/// `system-update.target` on the next restart.
pub async fn prepare_offline(
    plan: InstallPlan,
    cancellation: Cancellation,
    sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    packagekit_prepare_offline(plan, cancellation, sender).await
}

pub async fn prepare(cancellation: Cancellation) -> Result<(Snapshot, InstallPlan), Error> {
    packagekit_prepare(cancellation).await
}

pub async fn install(
    plan: InstallPlan,
    cancellation: Cancellation,
    sender: async_channel::Sender<InstallProgress>,
) -> Result<InstallResult, Error> {
    packagekit_install(plan, cancellation, sender).await
}
