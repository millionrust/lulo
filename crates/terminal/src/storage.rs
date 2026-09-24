use std::fmt;
use std::io;
use std::path::Path;

pub(crate) use rmac_storage::{Backend as Storage, FileSystem as RealStorage};
pub(crate) type Failure = rmac_storage::Failure<Operation>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    CreateConfigDirectory,
    LoadProfile,
    LoadSetting,
    ResolveConfigPath,
    SaveProfile,
    SaveSetting,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CreateConfigDirectory => "create the preferences directory",
            Self::LoadProfile => "load the terminal profile",
            Self::LoadSetting => "load a terminal setting",
            Self::ResolveConfigPath => "resolve the preferences path",
            Self::SaveProfile => "save the terminal profile",
            Self::SaveSetting => "save a terminal setting",
        })
    }
}

pub(crate) fn load_optional(
    storage: &impl Storage,
    path: &Path,
    operation: Operation,
) -> Result<Option<String>, Failure> {
    match storage.read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Failure::from_io(operation, path, error)),
    }
}

pub(crate) fn save(
    storage: &impl Storage,
    path: &Path,
    contents: impl AsRef<[u8]>,
    operation: Operation,
) -> Result<(), Failure> {
    let parent = path.parent().ok_or_else(|| {
        Failure::message(
            Operation::ResolveConfigPath,
            path,
            "preferences path has no parent directory",
        )
    })?;
    storage
        .create_dir_all(parent)
        .map_err(|error| Failure::from_io(Operation::CreateConfigDirectory, parent, error))?;
    storage
        .write_atomic(path, contents.as_ref())
        .map_err(|error| Failure::from_io(operation, path, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingStorage {
        create_succeeds: bool,
    }

    impl Storage for FailingStorage {
        fn read_to_string(&self, _path: &Path) -> io::Result<String> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected storage failure",
            ))
        }

        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            if self.create_succeeds {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected storage failure",
                ))
            }
        }

        fn write_atomic(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected storage failure",
            ))
        }
    }

    #[test]
    fn injected_failures_preserve_domain_operations() {
        let load_failure = load_optional(
            &FailingStorage {
                create_succeeds: false,
            },
            Path::new("profile.txt"),
            Operation::LoadProfile,
        )
        .unwrap_err();
        let directory_failure = save(
            &FailingStorage {
                create_succeeds: false,
            },
            Path::new("config/profile.txt"),
            "Basic",
            Operation::SaveProfile,
        )
        .unwrap_err();
        let write_failure = save(
            &FailingStorage {
                create_succeeds: true,
            },
            Path::new("config/profile.txt"),
            "Basic",
            Operation::SaveProfile,
        )
        .unwrap_err();

        assert_eq!(load_failure.operation, Operation::LoadProfile);
        assert_eq!(
            directory_failure.operation,
            Operation::CreateConfigDirectory
        );
        assert_eq!(write_failure.operation, Operation::SaveProfile);
    }
}
