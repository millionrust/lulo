use rmac_sharing::{Error, ErrorKind, Service, Snapshot};

use crate::system::{
    disable_remote_login_socket, restore_service, system_set_service, system_snapshot,
    wait_for_state,
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
        // Turning Remote Login off must also close a socket-activated port 22.
        let socket = if enabled {
            None
        } else {
            match disable_remote_login_socket() {
                Ok(previous) => previous,
                Err(error) => return Err(error),
            }
        };
        let restore = || {
            let _ = restore_service(
                unit,
                current.remote_login.active,
                current.remote_login.enabled_at_boot,
            );
            if let Some((was_active, was_enabled)) = socket {
                let _ = restore_service("ssh.socket", was_active, was_enabled);
            }
        };
        if let Err(error) = system_set_service(unit, enabled, "SSH") {
            restore();
            return Err(error);
        }
        match wait_for_state(ManagedService::RemoteLogin, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                restore();
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
        if let Err(error) = system_set_service(unit, enabled, "SMB") {
            let _ = restore_service(
                unit,
                current.file_sharing.active,
                current.file_sharing.enabled_at_boot,
            );
            return Err(error);
        }
        match wait_for_state(ManagedService::FileSharing, enabled) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                let _ = restore_service(
                    unit,
                    current.file_sharing.active,
                    current.file_sharing.enabled_at_boot,
                );
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
