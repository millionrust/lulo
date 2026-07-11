//! Cross-platform discovery and unmounting of user-visible volumes.

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub name: String,
    pub path: PathBuf,
    pub ejectable: bool,
}

#[derive(Debug)]
pub enum Error {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Command {
        program: &'static str,
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Command {
                program,
                path,
                message,
            } => write!(
                formatter,
                "{program} could not unmount {}: {message}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Command { .. } => None,
        }
    }
}

pub fn discover() -> Result<Vec<Mount>, Error> {
    #[cfg(target_os = "macos")]
    {
        discover_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let path = Path::new("/proc/self/mountinfo");
        let contents = std::fs::read_to_string(path).map_err(|source| Error::Io {
            operation: "read mount table",
            path: path.to_path_buf(),
            source,
        })?;
        Ok(parse_mountinfo(&contents))
    }
}

pub fn unmount(path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    let (program, arguments) = (
        "diskutil",
        vec!["eject".to_string(), path.to_string_lossy().into_owned()],
    );
    #[cfg(not(target_os = "macos"))]
    let (program, arguments) = {
        let uri = url::Url::from_directory_path(path)
            .map_err(|()| Error::Io {
                operation: "encode mount path",
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "mount path is not absolute"),
            })?
            .to_string();
        ("gio", vec!["mount".to_string(), "-u".to_string(), uri])
    };

    let output = Command::new(program)
        .args(&arguments)
        .output()
        .map_err(|source| Error::Io {
            operation: "start unmount helper",
            path: path.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exited with {}", output.status)
        };
        Err(Error::Command {
            program,
            path: path.to_path_buf(),
            message,
        })
    }
}

#[cfg(target_os = "macos")]
fn discover_macos() -> Result<Vec<Mount>, Error> {
    let root = Path::new("/Volumes");
    let entries = std::fs::read_dir(root).map_err(|source| Error::Io {
        operation: "read mounted volumes",
        path: root.to_path_buf(),
        source,
    })?;
    let mut mounts = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            (name != "Macintosh HD" && !name.starts_with('.')).then_some(Mount {
                name,
                path,
                ejectable: true,
            })
        })
        .collect::<Vec<_>>();
    sort_and_deduplicate(&mut mounts);
    Ok(mounts)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_mountinfo(contents: &str) -> Vec<Mount> {
    let mut mounts = contents
        .lines()
        .filter_map(parse_mountinfo_line)
        .filter(|mount| user_visible_mount(&mount.path))
        .collect::<Vec<_>>();
    sort_and_deduplicate(&mut mounts);
    mounts
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_mountinfo_line(line: &str) -> Option<Mount> {
    let (mount_fields, _filesystem_fields) = line.split_once(" - ")?;
    let encoded_path = mount_fields.split_whitespace().nth(4)?;
    let path = PathBuf::from(decode_mount_field(encoded_path)?);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())?;
    Some(Mount {
        name,
        path,
        ejectable: true,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn decode_mount_field(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let escape = [characters.next()?, characters.next()?, characters.next()?];
        decoded.push(match escape {
            ['0', '4', '0'] => ' ',
            ['0', '1', '1'] => '\t',
            ['0', '1', '2'] => '\n',
            ['1', '3', '4'] => '\\',
            _ => return None,
        });
    }
    Some(decoded)
}

#[cfg(any(not(target_os = "macos"), test))]
fn user_visible_mount(path: &Path) -> bool {
    let is_descendant = |root: &str| path.starts_with(root) && path != Path::new(root);
    if is_descendant("/media") || is_descendant("/run/media") || is_descendant("/mnt") {
        return true;
    }
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    components.len() > 5
        && components[1] == "run"
        && components[2] == "user"
        && components[3]
            .chars()
            .all(|character| character.is_ascii_digit())
        && components[4] == "gvfs"
}

fn sort_and_deduplicate(mounts: &mut Vec<Mount>) {
    mounts.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut seen = HashSet::new();
    mounts.retain(|mount| seen.insert(mount.path.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_decodes_and_filters_user_visible_mounts() {
        let contents = "36 25 8:1 / /media/alice/My\\040Drive rw,nosuid - vfat /dev/sdb1 rw\n\
                        37 25 0:42 / /run/user/1000/gvfs/smb-share:server=nas,share=docs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
                        38 25 0:5 / /proc rw - proc proc rw\n\
                        39 25 8:2 / /mnt/Backup rw shared:7 - ext4 /dev/sdc1 rw\n";

        let mounts = parse_mountinfo(contents);

        assert_eq!(mounts.len(), 3);
        assert!(mounts
            .iter()
            .any(|mount| mount.path == Path::new("/media/alice/My Drive")));
        assert!(mounts
            .iter()
            .any(|mount| mount.name == "smb-share:server=nas,share=docs"));
        assert!(!mounts.iter().any(|mount| mount.path == Path::new("/proc")));
    }

    #[test]
    fn malformed_and_unknown_mount_escapes_are_rejected() {
        assert_eq!(decode_mount_field("My\\040Drive"), Some("My Drive".into()));
        assert_eq!(decode_mount_field("bad\\999escape"), None);
        assert!(parse_mountinfo_line("not mountinfo").is_none());
    }

    #[test]
    fn duplicate_mount_points_are_removed() {
        let line = "36 25 8:1 / /media/alice/Drive rw - vfat /dev/sdb1 rw\n";
        let mounts = parse_mountinfo(&format!("{line}{line}"));
        assert_eq!(mounts.len(), 1);
    }
}
