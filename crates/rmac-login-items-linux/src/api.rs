//! Convenience API over the system login-item service.

use super::*;

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService::default().snapshot()
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<Snapshot, Error> {
    SystemService::default().set_enabled(id, enabled)
}

pub fn set_background_enabled(id: &str, enabled: bool) -> Result<Snapshot, Error> {
    SystemService::default().set_background_enabled(id, enabled)
}

pub fn prepare_add_source(source: &Path) -> Result<AddPreview, Error> {
    SystemService::default().prepare_add(source)
}

pub fn add_source(preview: &AddPreview) -> Result<Snapshot, Error> {
    SystemService::default().add(preview)
}

pub fn prepare_remove_autostart(id: &str) -> Result<RemovePreview, Error> {
    SystemService::default().prepare_remove(id)
}

pub fn remove_autostart(preview: &RemovePreview) -> Result<Snapshot, Error> {
    SystemService::default().remove(preview)
}

pub fn autostart_source(id: &str) -> Result<PathBuf, Error> {
    rmac_login_items::validate_id(id)?;
    SystemService::default()
        .snapshot()?
        .items
        .into_iter()
        .find(|item| item.id == id)
        .map(|item| item.source)
        .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists"))
}

pub fn background_service_source(id: &str) -> Result<PathBuf, Error> {
    rmac_login_items::validate_service_id(id)?;
    SystemService::default()
        .snapshot()?
        .background_services
        .into_iter()
        .find(|item| item.id == id)
        .and_then(|item| item.source)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "the user service file is unavailable",
            )
        })
}
