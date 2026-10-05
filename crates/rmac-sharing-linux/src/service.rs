use rmac_sharing::{Error, ErrorKind, Service, Snapshot};

use crate::system::{
    remote_login_socket_present, remote_login_units, system_set_service, system_snapshot,
    verify_state,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedService {
    RemoteLogin,
    FileSharing,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_remote_login(&self, enabled: bool) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let unit =
            current.remote_login.unit.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Unavailable, "OpenSSH server is not installed")
            })?;
        if current.remote_login.active == enabled && current.remote_login.enabled_at_boot == enabled
        {
            return Ok(current);
        }
        // Turning Remote Login off must also close a socket-activated port
        // 22 (SR-30): `ssh.socket` moves before `ssh.service` through the
        // same Enable/Disable + Reload call rather than its own separate
        // round trip, so a toggle no longer re-authorizes and reloads the
        // daemon once per unit.
        let units = remote_login_units(unit, enabled, remote_login_socket_present()?);
        if let Err(error) = system_set_service(&units, enabled, "SSH") {
            return Err(error);
        }
        match verify_state(ManagedService::RemoteLogin, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                // The mutation itself reported success; only the final
                // authoritative readback disagreed. Best-effort undo it.
                let _ = system_set_service(&units, !enabled, "SSH");
                Err(error)
            }
        }
    }

    fn set_file_sharing(&self, enabled: bool) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let unit = current.file_sharing.unit.as_deref().ok_or_else(|| {
            Error::new(ErrorKind::Unavailable, "Samba file server is not installed")
        })?;
        if current.file_sharing.active == enabled && current.file_sharing.enabled_at_boot == enabled
        {
            return Ok(current);
        }
        let units = [unit];
        if let Err(error) = system_set_service(&units, enabled, "SMB") {
            return Err(error);
        }
        match verify_state(ManagedService::FileSharing, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                let _ = system_set_service(&units, !enabled, "SMB");
                Err(error)
            }
        }
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

pub fn set_remote_login(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_remote_login(enabled)
}

pub fn set_file_sharing(enabled: bool) -> Result<Snapshot, Error> {
    SystemService.set_file_sharing(enabled)
}
