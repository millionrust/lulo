use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use rmac_focus::{Config, ManualActivation, Mode, ModeId};

use crate::model::MAX_FILE_BYTES;
use crate::wire::StoredFile;
use crate::{Error, ErrorKind, Operation, Recovery, Snapshot, Store};

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        Ok(Self::at(config_home.join("rmac/focus.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Snapshot, Error> {
        match self.read(&self.path) {
            Ok((config, manual)) => Ok(Snapshot {
                config,
                manual,
                recovery: Recovery::None,
            }),
            Err(error) if error.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                match self.read(&self.backup_path()) {
                    Ok((config, manual)) => Ok(Snapshot {
                        config,
                        manual,
                        recovery: Recovery::LastGood,
                    }),
                    Err(backup) if backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                        Ok(Snapshot {
                            config: default_config()?,
                            manual: None,
                            recovery: Recovery::None,
                        })
                    }
                    Err(backup) => Err(backup),
                }
            }
            Err(error) if recoverable(error) => match self.read(&self.backup_path()) {
                Ok((config, manual)) => Ok(Snapshot {
                    config,
                    manual,
                    recovery: Recovery::LastGood,
                }),
                Err(backup)
                    if recoverable(backup)
                        || backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) =>
                {
                    Ok(Snapshot {
                        config: default_config()?,
                        manual: None,
                        recovery: Recovery::Defaults,
                    })
                }
                Err(backup) => Err(backup),
            },
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, config: &Config, manual: Option<&ManualActivation>) -> Result<(), Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        set_private_directory(parent)?;
        let file = StoredFile::from_domain(config, manual);
        let bytes = serde_json::to_vec(&file)
            .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.backup_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn read(&self, path: &Path) -> Result<(Config, Option<ManualActivation>), Error> {
        let metadata =
            std::fs::metadata(path).map_err(|error| Error::io(Operation::Read, error))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Read, ErrorKind::Limit));
        }
        let bytes = std::fs::read(path).map_err(|error| Error::io(Operation::Read, error))?;
        let file: StoredFile = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        file.into_domain()
    }

    pub(crate) fn backup_path(&self) -> PathBuf {
        self.path.with_extension("last-good.json")
    }
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid | ErrorKind::UnsupportedVersion | ErrorKind::Limit
    )
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(Operation::CreateDirectory, error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), Error> {
    Ok(())
}

pub fn default_config() -> Result<Config, Error> {
    Config::new(
        vec![
            Mode::new(
                ModeId::parse("do-not-disturb").map_err(domain_error)?,
                "Do Not Disturb",
                BTreeSet::new(),
                false,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("personal").map_err(domain_error)?,
                "Personal",
                BTreeSet::new(),
                true,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("work").map_err(domain_error)?,
                "Work",
                BTreeSet::new(),
                true,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("sleep").map_err(domain_error)?,
                "Sleep",
                BTreeSet::new(),
                false,
            )
            .map_err(domain_error)?,
        ],
        Vec::new(),
    )
    .map_err(domain_error)
}

pub(crate) fn domain_error(_error: rmac_focus::Error) -> Error {
    Error::new(Operation::Validate, ErrorKind::Invalid)
}
