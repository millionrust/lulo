use std::collections::HashSet;
use std::io;
#[cfg(not(target_os = "macos"))]
use std::io::Read as _;
use std::path::{Path, PathBuf};

#[cfg(not(target_os = "macos"))]
use crate::model::MAX_MOUNTINFO_BYTES;
use crate::model::{MAX_DISPLAY_NAME_BYTES, MAX_MOUNTS};
use crate::{Error, Mount, Usage, Volume};

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

pub(crate) fn revalidated_mount(expected: &Mount, current: Vec<Mount>) -> Option<Mount> {
    current.into_iter().find(|candidate| {
        candidate.identity == expected.identity
            && candidate.path == expected.path
            && candidate.ejectable == expected.ejectable
    })
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

pub(crate) fn volume_usage(path: &Path) -> io::Result<Usage> {
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
pub(crate) fn parse_mountinfo(contents: &str) -> (Vec<Mount>, bool) {
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
pub(crate) fn parse_mountinfo_line(line: &str) -> Option<Mount> {
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
pub(crate) fn decode_mount_field(value: &str) -> Option<String> {
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

pub(crate) fn sanitize_display_name(value: &str) -> String {
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
