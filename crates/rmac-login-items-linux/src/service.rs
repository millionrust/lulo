//! Transactional login-item service implementation.

use super::*;

#[derive(Clone, Debug)]
pub struct SystemService {
    pub(super) environment: Environment,
}

impl Default for SystemService {
    fn default() -> Self {
        Self {
            environment: Environment::current(),
        }
    }
}

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let mut snapshot = discover(&self.environment)?;
        match systemd_background_services(&self.environment) {
            Ok((services, truncated)) => {
                snapshot.background_services = services;
                snapshot.background_services_truncated = truncated;
            }
            Err(error) => snapshot.background_services_error = Some(error.to_string()),
        }
        Ok(snapshot)
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error> {
        rmac_login_items::validate_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists"))?
            .clone();
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this system entry cannot be enabled without a user-owned override",
            ));
        }
        let source_before = read_entry_bytes(&item.source)?;
        let confirmed = self.snapshot()?;
        let confirmed_item = confirmed
            .items
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::Conflict, "autostart entry changed before save")
            })?;
        if confirmed_item != &item || read_entry_bytes(&item.source)? != source_before {
            return Err(Error::new(
                ErrorKind::Conflict,
                "autostart entry changed before save; refresh and try again",
            ));
        }
        let user_path = self.environment.config_home.join("autostart").join(id);
        let expected_contents = if enabled && item.managed_override && item.source == user_path {
            std::fs::remove_file(&user_path).map_err(mutation_error)?;
            sync_parent(&user_path)?;
            None
        } else {
            let source = String::from_utf8(source_before)
                .map_err(|_| Error::new(ErrorKind::InvalidEntry, "autostart entry is not UTF-8"))?;
            let managed = !item.user_owned;
            let updated = rmac_login_items::with_hidden(&source, !enabled, managed)?;
            std::fs::create_dir_all(
                user_path
                    .parent()
                    .expect("an autostart entry always has a parent"),
            )
            .map_err(mutation_error)?;
            rmac_storage::atomic_write(&user_path, updated.as_bytes()).map_err(mutation_error)?;
            Some(updated.into_bytes())
        };
        let refreshed = self.snapshot()?;
        let confirmed_state = refreshed
            .items
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| {
                candidate.enabled == enabled
                    && if expected_contents.is_some() {
                        candidate.source == user_path
                            && candidate.user_owned
                            && candidate.managed_override != item.user_owned
                    } else {
                        candidate.source != user_path && !candidate.managed_override
                    }
            });
        let exact_file = match expected_contents {
            Some(expected) => read_entry_bytes(&user_path)? == expected,
            None => read_optional_entry_bytes(&user_path)?.is_none(),
        };
        if confirmed_state && exact_file {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the autostart inventory did not confirm the requested state",
            ))
        }
    }

    fn set_background_enabled(&self, id: &str, enabled: bool) -> Result<Snapshot, Error> {
        rmac_login_items::validate_service_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .background_services
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::new(ErrorKind::InvalidEntry, "user service no longer exists"))?
            .clone();
        if item.enabled == enabled {
            return Ok(current);
        }
        if !item.can_toggle {
            return Err(Error::new(
                ErrorKind::Mutation,
                "this user service does not have a safe persistent transition",
            ));
        }
        let confirmed = self.snapshot()?;
        if confirmed
            .background_services
            .iter()
            .find(|candidate| candidate.id == id)
            != Some(&item)
        {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the user service changed before save; refresh and try again",
            ));
        }
        systemd_set_enabled(id, enabled)?;
        let refreshed = self.snapshot()?;
        let changed = refreshed
            .background_services
            .iter()
            .find(|item| item.id == id)
            .is_some_and(|candidate| {
                candidate.enabled == enabled
                    && candidate.user_owned
                    && candidate.source == item.source
            });
        if changed {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the user manager did not confirm the requested unit-file state",
            ))
        }
    }

    fn prepare_add(&self, source: &Path) -> Result<AddPreview, Error> {
        prepare_add(&self.environment, source)
    }

    fn add(&self, preview: &AddPreview) -> Result<Snapshot, Error> {
        let current_source = read_entry_bytes(&preview.source)?;
        if current_source != preview.source_contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the selected desktop entry changed after review",
            ));
        }
        let contents = String::from_utf8(current_source)
            .map_err(|_| Error::new(ErrorKind::InvalidEntry, "desktop entry is not UTF-8"))?;
        let contents = rmac_login_items::with_hidden(&contents, false, false)?;
        let target = self
            .environment
            .config_home
            .join("autostart")
            .join(&preview.id);
        let current_target = read_optional_entry_bytes(&target)?;
        if current_target.as_deref() != preview.target_contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the destination login item changed after review",
            ));
        }
        std::fs::create_dir_all(target.parent().expect("autostart target has a parent"))
            .map_err(mutation_error)?;
        rmac_storage::atomic_write(&target, contents.as_bytes()).map_err(mutation_error)?;
        let refreshed = self.snapshot()?;
        let exact_inventory = refreshed.items.iter().any(|item| {
            item.id == preview.id
                && item.enabled
                && item.source == target
                && item.name == preview.name
        });
        let exact_file = read_entry_bytes(&target)? == contents.as_bytes();
        if exact_inventory && exact_file {
            Ok(refreshed)
        } else {
            Err(Error::new(
                ErrorKind::Mismatch,
                "the autostart inventory did not confirm the installed login item",
            ))
        }
    }

    fn prepare_remove(&self, id: &str) -> Result<RemovePreview, Error> {
        rmac_login_items::validate_id(id)?;
        let current = self.snapshot()?;
        let item = current
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidEntry, "autostart entry no longer exists")
            })?;
        if !item.user_owned || item.managed_override {
            return Err(Error::new(
                ErrorKind::Mutation,
                "only user-owned application entries can be moved to Trash",
            ));
        }
        let contents = read_entry_bytes(&item.source)?;
        Ok(RemovePreview::new(
            item.id.clone(),
            item.name.clone(),
            item.source.clone(),
            contents,
        ))
    }

    fn remove(&self, preview: &RemovePreview) -> Result<Snapshot, Error> {
        let current = self.prepare_remove(&preview.id)?;
        if current != *preview || read_entry_bytes(&preview.source)? != preview.contents() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the login item changed after removal was confirmed",
            ));
        }
        trash::delete(&preview.source).map_err(|_| {
            Error::new(
                ErrorKind::Mutation,
                "the login item could not be moved to Trash",
            )
        })?;
        let refreshed = self.snapshot()?;
        preserve_disabled_after_removal(self, preview, refreshed)
    }
}
