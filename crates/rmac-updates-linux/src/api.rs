use rmac_updates::{
    Cancellation, Error, InstallPlan, InstallProgress, InstallResult, Request, Snapshot,
    SnapshotFuture, Source,
};

use crate::transaction::{packagekit_install, packagekit_prepare, packagekit_snapshot};

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

pub async fn snapshot(request: Request) -> Result<Snapshot, Error> {
    SystemSource.snapshot(request).await
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
