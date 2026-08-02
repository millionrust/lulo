use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rmac_storage::Backend as _;

use crate::filesystem::*;
use crate::{Consent, Error, ErrorKind, Inner, Operation, Outcome, Prepared, Staged};

#[derive(Clone)]
pub struct Importer {
    inner: Arc<Inner>,
}

impl fmt::Debug for Importer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Importer(<redacted>)")
    }
}

impl Importer {
    pub fn from_environment() -> Result<Self, Error> {
        let settings = rmac_shell_settings::ShellSettingsStore::from_environment()
            .map_err(|error| settings_error(Operation::EstablishAuthority, error))?;
        Self::new(
            managed_root_from_environment()?,
            settings.path().to_path_buf(),
        )
    }

    pub fn new(managed_root: PathBuf, settings_path: PathBuf) -> Result<Self, Error> {
        validate_absolute_path(&managed_root)?;
        validate_absolute_path(&settings_path)?;
        rmac_storage::create_dir_all_private(&managed_root).map_err(|error| {
            io_error(
                Operation::EstablishAuthority,
                error,
                "create the private wallpaper import directory",
            )
        })?;
        let lease = acquire_lease(&managed_root.join(LOCK_FILE))?;
        let authority = next_identity(&AUTHORITY_SEQUENCE, Operation::EstablishAuthority)?;
        let importer = Self {
            inner: Arc::new(Inner {
                authority,
                managed_root,
                settings_path,
                _lease: lease,
                commit: Mutex::new(()),
            }),
        };
        importer.recover_orphans()?;
        Ok(importer)
    }

    /// Validate and decode one local document before any user confirmation.
    /// The returned image is the mandatory preview input. Dropping the request
    /// removes its private staging file as a best-effort safety net.
    pub fn prepare(&self, request: rmac_wallpaper::portal::Request) -> Result<Prepared, Error> {
        let plan = rmac_wallpaper::portal::evaluate(request).map_err(|kind| {
            failure(
                Operation::AdmitRequest,
                ErrorKind::Portal(kind),
                "the request did not satisfy wallpaper portal policy",
            )
        })?;
        let rmac_wallpaper::Source::File(path) = plan.source else {
            return Err(failure(
                Operation::AdmitRequest,
                ErrorKind::InvalidAuthority,
                "portal admission produced a non-file source",
            ));
        };
        let asset = rmac_wallpaper_system::open_file(&path).map_err(|error| {
            failure(
                Operation::OpenSource,
                ErrorKind::Source(error.kind),
                error.detail(),
            )
        })?;
        let format = asset.format;
        let id = next_request_id()?;
        let staging = self.inner.managed_root.join(format!(
            "{INCOMING_PREFIX}{}-{}",
            std::process::id(),
            id.0
        ));
        let mut source = asset.into_file();
        let fingerprint = rmac_storage::FileSystem
            .write_new_private_stream(
                &staging,
                &mut source,
                rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
            )
            .map_err(|error| {
                io_error(
                    Operation::StageSource,
                    error,
                    "stage the validated wallpaper source",
                )
            })?;
        let decoded = (|| {
            let staged_source =
                rmac_wallpaper_system::resolve(&rmac_wallpaper::Source::File(staging.clone()))
                    .map_err(|error| {
                        failure(
                            Operation::OpenSource,
                            ErrorKind::Source(error.kind),
                            error.detail(),
                        )
                    })?;
            rmac_wallpaper_image::Cache::new(0)
                .get_or_decode(
                    staged_source,
                    rmac_compositor::PhysicalSize {
                        width: 1,
                        height: 1,
                    },
                )
                .map_err(|error| {
                    failure(
                        Operation::DecodeSource,
                        ErrorKind::Decode(error.kind),
                        error.detail(),
                    )
                })
        })();
        let image = match decoded {
            Ok(image) => image,
            Err(error) => {
                remove_exact(&staging, Operation::RemoveStaging)?;
                return Err(error);
            }
        };
        Ok(Prepared {
            id,
            authority: self.inner.authority,
            app_id: plan.app_id,
            staged: Some(Staged {
                path: staging,
                fingerprint,
                format,
            }),
            image,
        })
    }

    /// Consume a prepared request exactly once. Decline and cancellation never
    /// read or write shell settings. Acceptance serializes the durable import
    /// and whole-desktop authority mutation with other accepted portal calls.
    pub fn finish(&self, mut prepared: Prepared, consent: Consent) -> Result<Outcome, Error> {
        if prepared.authority != self.inner.authority {
            return Err(failure(
                Operation::AdmitRequest,
                ErrorKind::WrongAuthority,
                "the prepared request belongs to another importer",
            ));
        }
        let staged = prepared.staged.take().ok_or_else(|| {
            failure(
                Operation::AdmitRequest,
                ErrorKind::WrongAuthority,
                "the prepared request no longer owns staged content",
            )
        })?;
        if consent != Consent::Accept {
            remove_exact(&staged.path, Operation::RemoveStaging)?;
            return Ok(Outcome::Cancelled);
        }
        let _commit = self.commit_guard();
        let result = self.commit(&staged);
        if result.is_err() {
            let _ = remove_if_present(&staged.path, Operation::RemoveStaging);
        }
        result
    }

    fn commit(&self, staged: &Staged) -> Result<Outcome, Error> {
        let destination = self
            .inner
            .managed_root
            .join(managed_name(staged.fingerprint, staged.format));
        let created = match std::fs::hard_link(&staged.path, &destination) {
            Ok(()) => {
                if let Err(error) =
                    sync_directory(&self.inner.managed_root, Operation::CreateImport)
                {
                    let _ = remove_exact(&destination, Operation::RemoveImport);
                    return Err(error);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = rmac_storage::fingerprint_bounded_no_follow(
                    &destination,
                    rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
                )
                .map_err(|read_error| {
                    io_error(
                        Operation::CreateImport,
                        read_error,
                        "validate an existing content-addressed import",
                    )
                })?;
                if existing != staged.fingerprint {
                    return Err(failure(
                        Operation::CreateImport,
                        ErrorKind::ReadbackMismatch,
                        "an existing content-addressed import did not match",
                    ));
                }
                false
            }
            Err(error) => {
                return Err(io_error(
                    Operation::CreateImport,
                    error,
                    "create the durable wallpaper import",
                ));
            }
        };
        if let Err(error) = remove_exact(&staged.path, Operation::RemoveStaging) {
            if created {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
            return Err(error);
        }
        let imported = match rmac_storage::fingerprint_bounded_no_follow(
            &destination,
            rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                if created {
                    let _ = remove_exact(&destination, Operation::RemoveImport);
                }
                return Err(io_error(
                    Operation::CreateImport,
                    error,
                    "verify the durable wallpaper import",
                ));
            }
        };
        if imported != staged.fingerprint {
            if created {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
            return Err(failure(
                Operation::CreateImport,
                ErrorKind::ReadbackMismatch,
                "the durable import did not match the reviewed source",
            ));
        }
        let result = self.commit_settings(&destination);
        if let Err(error) = &result {
            let definitely_pre_write = error.operation == Operation::ReadSettings;
            if created
                && (definitely_pre_write || !self.destination_may_be_referenced(&destination))
            {
                let _ = remove_exact(&destination, Operation::RemoveImport);
            }
        }
        result?;
        let cleanup_pending = self.remove_unreferenced_imports().is_err();
        Ok(Outcome::Applied { cleanup_pending })
    }

    fn commit_settings(&self, destination: &Path) -> Result<(), Error> {
        let source = destination.to_str().ok_or_else(|| {
            failure(
                Operation::SaveSettings,
                ErrorKind::InvalidAuthority,
                "the managed wallpaper path is not valid UTF-8",
            )
        })?;
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let mut settings = store
            .load()
            .map_err(|error| settings_error(Operation::ReadSettings, error))?
            .settings;
        settings.wallpaper = rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some(source.to_owned()),
                fit: rmac_shell_settings::WallpaperFit::Fill,
            },
            per_output: Default::default(),
        };
        store
            .save(&settings)
            .map_err(|error| settings_error(Operation::SaveSettings, error))?;
        let readback = store
            .load()
            .map_err(|error| settings_error(Operation::VerifySettings, error))?;
        if readback.settings.wallpaper != settings.wallpaper {
            return Err(failure(
                Operation::VerifySettings,
                ErrorKind::ReadbackMismatch,
                "the wallpaper authority changed before readback",
            ));
        }
        Ok(())
    }

    fn recover_orphans(&self) -> Result<usize, Error> {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let Ok(snapshot) = store.load() else {
            return Ok(0);
        };
        self.remove_unreferenced(&snapshot.settings)
    }

    fn remove_unreferenced_imports(&self) -> Result<usize, Error> {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let snapshot = store
            .load()
            .map_err(|error| settings_error(Operation::RecoverImports, error))?;
        self.remove_unreferenced(&snapshot.settings)
    }

    fn destination_may_be_referenced(&self, destination: &Path) -> bool {
        let store = rmac_shell_settings::ShellSettingsStore::new(self.inner.settings_path.clone());
        let Ok(snapshot) = store.load() else {
            return true;
        };
        referenced_managed_paths(&snapshot.settings.wallpaper, &self.inner.managed_root)
            .contains(destination)
    }

    fn remove_unreferenced(
        &self,
        settings: &rmac_shell_settings::ShellSettings,
    ) -> Result<usize, Error> {
        let referenced = referenced_managed_paths(&settings.wallpaper, &self.inner.managed_root);
        let entries = std::fs::read_dir(&self.inner.managed_root).map_err(|error| {
            io_error(
                Operation::RecoverImports,
                error,
                "enumerate managed wallpaper imports",
            )
        })?;
        let mut removed = 0;
        for entry in entries {
            let entry = entry.map_err(|error| {
                io_error(
                    Operation::RecoverImports,
                    error,
                    "read a managed wallpaper directory entry",
                )
            })?;
            let path = entry.path();
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let removable = name.starts_with(INCOMING_PREFIX)
                || (is_managed_name(name) && !referenced.contains(&path));
            if removable {
                remove_exact(&path, Operation::RecoverImports)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn commit_guard(&self) -> MutexGuard<'_, ()> {
        self.inner
            .commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
