//! Platform-neutral user-place and Trash state for shell surfaces.

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Place {
    pub path: PathBuf,
    pub exists: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrashSnapshot {
    pub available: bool,
    pub empty: bool,
    pub item_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadsResolution {
    pub path: PathBuf,
    /// True only when a valid XDG_DOWNLOAD_DIR distinct from HOME was read.
    pub configured: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub home: Place,
    pub downloads: Place,
    pub downloads_configured: bool,
    pub trash: TrashSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    HomeNotAbsolute,
    InvalidDownloadValue { detail: String },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeNotAbsolute => formatter.write_str("HOME must be an absolute path"),
            Self::InvalidDownloadValue { detail } => {
                write!(formatter, "invalid XDG_DOWNLOAD_DIR: {detail}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Resolve `XDG_DOWNLOAD_DIR` without invoking a shell or expanding arbitrary
/// variables. Missing configuration falls back to HOME/Downloads; an explicit
/// HOME value means the special directory was disabled.
pub fn resolve_downloads(
    home: &Path,
    user_dirs_contents: Option<&str>,
) -> Result<DownloadsResolution, Error> {
    if !home.is_absolute() {
        return Err(Error::HomeNotAbsolute);
    }
    let fallback = DownloadsResolution {
        path: home.join("Downloads"),
        configured: false,
    };
    let Some(contents) = user_dirs_contents else {
        return Ok(fallback);
    };
    let Some(raw) = contents.lines().find_map(|line| {
        let line = line.trim();
        (!line.starts_with('#'))
            .then(|| line.strip_prefix("XDG_DOWNLOAD_DIR="))
            .flatten()
    }) else {
        return Ok(fallback);
    };
    let value = parse_quoted_value(raw)?;
    if value.contains('`') || value.contains('\0') {
        return Err(Error::InvalidDownloadValue {
            detail: "unsupported expansion or character".into(),
        });
    }
    let expanded = if let Some(suffix) = value.strip_prefix("$HOME") {
        validate_home_suffix(suffix, "$HOME")?;
        append_home(home, suffix)
    } else if let Some(suffix) = value.strip_prefix("${HOME}") {
        validate_home_suffix(suffix, "${HOME}")?;
        append_home(home, suffix)
    } else {
        if value.contains('$') {
            return Err(Error::InvalidDownloadValue {
                detail: "unsupported variable expansion".into(),
            });
        }
        PathBuf::from(&value)
    };
    if !expanded.is_absolute() {
        return Err(Error::InvalidDownloadValue {
            detail: "path must be absolute".into(),
        });
    }
    Ok(DownloadsResolution {
        configured: expanded != home,
        path: expanded,
    })
}

fn validate_home_suffix(suffix: &str, variable: &str) -> Result<(), Error> {
    if suffix.contains('$') {
        return Err(Error::InvalidDownloadValue {
            detail: "unsupported nested variable expansion".into(),
        });
    }
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return Err(Error::InvalidDownloadValue {
            detail: format!("{variable} must be followed by a path separator"),
        });
    }
    Ok(())
}

fn append_home(home: &Path, suffix: &str) -> PathBuf {
    suffix
        .strip_prefix('/')
        .filter(|suffix| !suffix.is_empty())
        .map_or_else(|| home.to_path_buf(), |suffix| home.join(suffix))
}

fn parse_quoted_value(raw: &str) -> Result<String, Error> {
    let raw = raw.trim();
    if raw.len() < 2 || !raw.starts_with('"') || !raw.ends_with('"') {
        return Err(Error::InvalidDownloadValue {
            detail: "value must be double quoted".into(),
        });
    }
    let mut value = String::new();
    let mut characters = raw[1..raw.len() - 1].chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            value.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => value.push('\\'),
            Some('"') => value.push('"'),
            Some('$') => value.push('$'),
            Some(character) => {
                return Err(Error::InvalidDownloadValue {
                    detail: format!("unsupported escape \\{character}"),
                });
            }
            None => {
                return Err(Error::InvalidDownloadValue {
                    detail: "trailing escape".into(),
                });
            }
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_configuration_uses_the_conventional_home_fallback() {
        assert_eq!(
            resolve_downloads(Path::new("/home/alex"), None).expect("valid HOME"),
            DownloadsResolution {
                path: PathBuf::from("/home/alex/Downloads"),
                configured: false,
            }
        );
    }

    #[test]
    fn xdg_home_and_absolute_paths_resolve_without_shell_expansion() {
        let home = Path::new("/home/alex");
        assert_eq!(
            resolve_downloads(home, Some("XDG_DOWNLOAD_DIR=\"$HOME/Transfers\"\n"))
                .expect("HOME path"),
            DownloadsResolution {
                path: PathBuf::from("/home/alex/Transfers"),
                configured: true,
            }
        );
        assert_eq!(
            resolve_downloads(home, Some("XDG_DOWNLOAD_DIR=\"/data/downloads\"\n"))
                .expect("absolute path")
                .path,
            Path::new("/data/downloads")
        );
    }

    #[test]
    fn explicit_home_disables_the_special_directory() {
        let resolution = resolve_downloads(
            Path::new("/home/alex"),
            Some("XDG_DOWNLOAD_DIR=\"${HOME}\"\n"),
        )
        .expect("disabled directory");
        assert_eq!(resolution.path, Path::new("/home/alex"));
        assert!(!resolution.configured);
    }

    #[test]
    fn malformed_or_dynamic_values_are_rejected_not_executed() {
        let home = Path::new("/home/alex");
        for contents in [
            "XDG_DOWNLOAD_DIR=relative\n",
            "XDG_DOWNLOAD_DIR=\"relative\"\n",
            "XDG_DOWNLOAD_DIR=\"$OTHER/downloads\"\n",
            "XDG_DOWNLOAD_DIR=\"`touch /tmp/bad`\"\n",
            "XDG_DOWNLOAD_DIR=\"$HOME\\q\"\n",
        ] {
            assert!(
                resolve_downloads(home, Some(contents)).is_err(),
                "{contents}"
            );
        }
    }

    #[test]
    fn relative_home_is_never_accepted() {
        assert_eq!(
            resolve_downloads(Path::new("home/alex"), None),
            Err(Error::HomeNotAbsolute)
        );
    }
}
