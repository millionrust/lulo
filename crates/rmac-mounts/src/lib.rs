//! Cross-platform discovery and unmounting of user-visible volumes.

use std::collections::HashSet;
use std::fmt;
use std::io;
#[cfg(not(target_os = "macos"))]
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(not(target_os = "macos"))]
const MAX_MOUNTINFO_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MOUNTS: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 256;
#[cfg(target_os = "linux")]
const MOUNT_WATCH_TIMEOUT_SECONDS: i64 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// Opaque identity used only to revalidate a selection against the current
    /// mount namespace. It is not a filesystem path or a display label.
    pub identity: String,
    pub name: String,
    pub path: PathBuf,
    pub ejectable: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

impl Usage {
    pub fn used_fraction(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn is_low_space(self) -> bool {
        self.total > 0 && (self.available < 5_000_000_000 || self.available < self.total / 20)
    }

    fn from_blocks(block_size: u64, blocks: u64, available_blocks: u64) -> Self {
        let total = blocks.saturating_mul(block_size);
        let available = available_blocks.saturating_mul(block_size).min(total);
        Self {
            total,
            used: total.saturating_sub(available),
            available,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub mount: Mount,
    pub usage: Option<Usage>,
    pub usage_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
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
    TooLarge {
        path: PathBuf,
        limit: u64,
    },
    TooManyMounts {
        limit: usize,
    },
    Stale {
        name: String,
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
            Self::TooLarge { path, limit } => write!(
                formatter,
                "{} exceeds the {limit}-byte safety limit",
                path.display()
            ),
            Self::TooManyMounts { limit } => {
                write!(formatter, "more than {limit} mounted volumes were reported")
            }
            Self::Stale { name } => {
                write!(
                    formatter,
                    "the mounted volume “{name}” is no longer available"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Command { .. }
            | Self::TooLarge { .. }
            | Self::TooManyMounts { .. }
            | Self::Stale { .. } => None,
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
        let contents = read_bounded_text(path, MAX_MOUNTINFO_BYTES)?;
        let (mounts, truncated) = parse_mountinfo(&contents);
        if truncated {
            Err(Error::TooManyMounts { limit: MAX_MOUNTS })
        } else {
            Ok(mounts)
        }
    }
}

/// Return the system volume plus user-visible mounted volumes with independent
/// capacity results. A broken or disconnected mount never hides healthy ones.
pub fn volumes() -> Result<Vec<Volume>, Error> {
    let mut mounts = vec![Mount {
        identity: "system:/".into(),
        name: "System Volume".into(),
        path: PathBuf::from("/"),
        ejectable: false,
    }];
    mounts.extend(discover()?);
    sort_and_deduplicate(&mut mounts);
    // Keep the system volume first regardless of localized external names.
    mounts.sort_by_key(|mount| mount.ejectable);
    Ok(mounts
        .into_iter()
        .map(|mount| match volume_usage(&mount.path) {
            Ok(usage) => Volume {
                mount,
                usage: Some(usage),
                usage_error: None,
            },
            Err(error) => Volume {
                mount,
                usage: None,
                usage_error: Some(format!("Could not read capacity: {error}")),
            },
        })
        .collect())
}

/// Re-read the current mount namespace and return the exact still-mounted
/// volume selected by the UI. Paths and visible names are never identities.
pub fn revalidate(expected: &Mount) -> Result<Mount, Error> {
    if expected.identity == "system:/" && expected.path == Path::new("/") && !expected.ejectable {
        return Ok(Mount {
            identity: "system:/".into(),
            name: "System Volume".into(),
            path: PathBuf::from("/"),
            ejectable: false,
        });
    }
    revalidated_mount(expected, discover()?).ok_or_else(|| Error::Stale {
        name: expected.name.clone(),
    })
}

fn revalidated_mount(expected: &Mount, current: Vec<Mount>) -> Option<Mount> {
    current.into_iter().find(|candidate| {
        candidate.identity == expected.identity
            && candidate.path == expected.path
            && candidate.ejectable == expected.ejectable
    })
}

/// Watch the caller's Linux mount namespace. `/proc/self/mounts` implements
/// `POLLPRI` for mount and unmount changes; each event is only a hint and
/// consumers must take a complete fresh `volumes()` snapshot.
pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        let worker_sender = sender.clone();
        let result = blocking::unblock(move || watch_mount_changes(&worker_sender)).await;
        if let Err(error) = result {
            let _ = sender.send(WatchEvent::Unavailable).await;
            return Err(error);
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = sender.send(WatchEvent::Unavailable).await;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn watch_mount_changes(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};

    let path = Path::new("/proc/self/mounts");
    let file = std::fs::File::open(path).map_err(|source| Error::Io {
        operation: "watch mount table",
        path: path.to_path_buf(),
        source,
    })?;
    let _ = sender.try_send(WatchEvent::Changed);
    let timeout = Timespec {
        tv_sec: MOUNT_WATCH_TIMEOUT_SECONDS,
        tv_nsec: 0,
    };
    while !sender.is_closed() {
        let mut descriptors = [PollFd::new(&file, PollFlags::PRI | PollFlags::ERR)];
        let ready = poll(&mut descriptors, Some(&timeout)).map_err(|source| Error::Io {
            operation: "watch mount table",
            path: path.to_path_buf(),
            source: io::Error::from(source),
        })?;
        if ready > 0
            && descriptors[0]
                .revents()
                .intersects(PollFlags::PRI | PollFlags::ERR)
        {
            let _ = sender.try_send(WatchEvent::Changed);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn read_bounded_text(path: &Path, limit: u64) -> Result<String, Error> {
    let file = std::fs::File::open(path).map_err(|source| Error::Io {
        operation: "open mount table",
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| Error::Io {
            operation: "read mount table",
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|source| Error::Io {
        operation: "decode mount table",
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

fn volume_usage(path: &Path) -> io::Result<Usage> {
    let status = rustix::fs::statvfs(path).map_err(io::Error::from)?;
    let block_size = if status.f_frsize > 0 {
        status.f_frsize
    } else {
        status.f_bsize
    };
    Ok(Usage::from_blocks(
        block_size,
        status.f_blocks,
        status.f_bavail,
    ))
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
            let name = sanitize_display_name(&entry.file_name().to_string_lossy());
            (!name.is_empty() && name != "Macintosh HD" && !name.starts_with('.')).then_some(
                Mount {
                    identity: format!("macos:{}", path.to_string_lossy()),
                    name,
                    path,
                    ejectable: true,
                },
            )
        })
        .collect::<Vec<_>>();
    sort_and_deduplicate(&mut mounts);
    if mounts.len() > MAX_MOUNTS {
        return Err(Error::TooManyMounts { limit: MAX_MOUNTS });
    }
    Ok(mounts)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_mountinfo(contents: &str) -> (Vec<Mount>, bool) {
    let mut mounts = contents
        .lines()
        .filter_map(parse_mountinfo_line)
        .filter(|mount| user_visible_mount(&mount.path))
        .collect::<Vec<_>>();
    sort_and_deduplicate(&mut mounts);
    let truncated = mounts.len() > MAX_MOUNTS;
    mounts.truncate(MAX_MOUNTS);
    (mounts, truncated)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_mountinfo_line(line: &str) -> Option<Mount> {
    let (mount_fields, _filesystem_fields) = line.split_once(" - ")?;
    let mut fields = mount_fields.split_whitespace();
    let mount_id = fields.next()?;
    if mount_id.is_empty() || !mount_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let encoded_path = fields.nth(3)?;
    let path = PathBuf::from(decode_mount_field(encoded_path)?);
    let name = mount_display_name(&path)?;
    Some(Mount {
        identity: format!("linux:{mount_id}"),
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

#[cfg(any(not(target_os = "macos"), test))]
fn mount_display_name(path: &Path) -> Option<String> {
    let raw = path.file_name()?.to_string_lossy();
    if path.starts_with("/run/user")
        && path
            .components()
            .any(|component| component.as_os_str() == "gvfs")
    {
        if let Some(share) = raw
            .split(',')
            .find_map(|field| field.strip_prefix("share="))
            .map(sanitize_display_name)
            .filter(|name| !name.is_empty())
        {
            return Some(share);
        }
        return Some("Remote Volume".into());
    }
    let name = sanitize_display_name(&raw);
    (!name.is_empty()).then_some(name)
}

fn sanitize_display_name(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_DISPLAY_NAME_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
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

        let (mounts, truncated) = parse_mountinfo(contents);

        assert!(!truncated);
        assert_eq!(mounts.len(), 3);
        assert!(mounts
            .iter()
            .any(|mount| mount.path == Path::new("/media/alice/My Drive")));
        assert!(mounts.iter().any(|mount| mount.name == "docs"));
        assert!(mounts
            .iter()
            .all(|mount| mount.identity.starts_with("linux:")));
        assert!(!mounts.iter().any(|mount| mount.path == Path::new("/proc")));
    }

    #[test]
    fn malformed_and_unknown_mount_escapes_are_rejected() {
        assert_eq!(decode_mount_field("My\\040Drive"), Some("My Drive".into()));
        assert_eq!(decode_mount_field("bad\\999escape"), None);
        assert!(parse_mountinfo_line("not mountinfo").is_none());
        assert!(
            parse_mountinfo_line("bad-id 25 8:1 / /media/alice/Drive rw - vfat /dev/sdb1 rw")
                .is_none()
        );
    }

    #[test]
    fn duplicate_mount_points_are_removed() {
        let line = "36 25 8:1 / /media/alice/Drive rw - vfat /dev/sdb1 rw\n";
        let (mounts, truncated) = parse_mountinfo(&format!("{line}{line}"));
        assert_eq!(mounts.len(), 1);
        assert!(!truncated);
    }

    #[test]
    fn usage_math_is_saturating_and_flags_low_space() {
        let healthy = Usage::from_blocks(4096, 10_000_000, 5_000_000);
        assert_eq!(healthy.total, 40_960_000_000);
        assert_eq!(healthy.used, 20_480_000_000);
        assert!(!healthy.is_low_space());

        let low = Usage::from_blocks(4096, 10_000_000, 100_000);
        assert!(low.is_low_space());
        assert!(low.used_fraction() > 0.9);

        let inconsistent = Usage::from_blocks(u64::MAX, u64::MAX, u64::MAX);
        assert_eq!(inconsistent.available, inconsistent.total);
        assert_eq!(inconsistent.used, 0);
    }

    #[test]
    fn live_root_volume_has_bounded_capacity_relationships() {
        let usage = volume_usage(Path::new("/")).unwrap();
        assert!(usage.total > 0);
        assert!(usage.available <= usage.total);
        assert_eq!(usage.used, usage.total - usage.available);
    }

    #[test]
    fn display_names_are_bounded_and_cannot_inject_controls_or_remote_identity() {
        let control =
            parse_mountinfo_line("36 25 8:1 / /media/alice/Evil\\012Name rw - vfat /dev/sdb1 rw")
                .unwrap();
        assert_eq!(control.name, "Evil Name");
        assert!(!control.name.chars().any(char::is_control));

        let remote = parse_mountinfo_line(
            "37 25 0:42 / /run/user/1000/gvfs/google-drive:host=example,user=private rw - fuse.gvfsd-fuse gvfsd-fuse rw",
        )
        .unwrap();
        assert_eq!(remote.name, "Remote Volume");
        assert!(!remote.name.contains("private"));

        let long = sanitize_display_name(&"x".repeat(MAX_DISPLAY_NAME_BYTES + 50));
        assert_eq!(long.len(), MAX_DISPLAY_NAME_BYTES);
    }

    #[test]
    fn discovery_is_bounded_to_the_supported_visible_volume_count() {
        let contents = (0..MAX_MOUNTS + 20)
            .map(|index| {
                format!(
                    "{} 25 8:1 / /media/alice/Drive-{index} rw - vfat /dev/sdb1 rw",
                    index + 1
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let (mounts, truncated) = parse_mountinfo(&contents);
        assert_eq!(mounts.len(), MAX_MOUNTS);
        assert!(truncated);
    }

    #[test]
    fn revalidation_requires_identity_path_and_mount_class() {
        let expected = Mount {
            identity: "linux:42".into(),
            name: "Backup".into(),
            path: PathBuf::from("/media/alice/Backup"),
            ejectable: true,
        };
        let replacement = Mount {
            identity: "linux:43".into(),
            name: "Backup".into(),
            path: expected.path.clone(),
            ejectable: true,
        };
        assert!(revalidated_mount(&expected, vec![replacement]).is_none());

        let exact = expected.clone();
        assert_eq!(
            revalidated_mount(&expected, vec![exact.clone()]),
            Some(exact)
        );
    }
}
