use std::collections::HashSet;
use std::io;
#[cfg(target_os = "linux")]
use std::io::Read as _;
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "macos", test))]
use crate::model::MAX_DISPLAY_NAME_BYTES;
#[cfg(target_os = "linux")]
use crate::model::MAX_MOUNTINFO_BYTES;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use crate::model::MAX_MOUNTS;
use crate::{Error, Mount, Usage, Volume};

pub fn discover() -> Result<Vec<Mount>, Error> {
    #[cfg(target_os = "macos")]
    {
        discover_macos()
    }
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "windows")]
    {
        discover_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Ok(Vec::new())
    }
}

/// Every lettered drive (`GetLogicalDrives`), labelled with its volume name
/// (`GetVolumeInformationW`) when it has one, else "Local Disk (C:)" or "CD
/// Drive (D:)" to match Explorer's own wording. A drive with no media (an
/// empty optical or card reader) is skipped rather than shown as a broken
/// entry (ADR 0023 phase 4's Locations mapping).
#[cfg(target_os = "windows")]
fn discover_windows() -> Result<Vec<Mount>, Error> {
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };

    // `GetDriveTypeW`'s return value, from `fileapi.h`; windows-rs does not
    // wrap these as named constants (they are raw preprocessor `#define`s,
    // not a typed enum), so they are spelled out here.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_CDROM: u32 = 5;

    let mut mounts = Vec::new();
    // SAFETY: no pointers; returns a bitmask with no error state.
    let bitmask = unsafe { GetLogicalDrives() };
    for letter in 0u32..26 {
        if bitmask & (1 << letter) == 0 {
            continue;
        }
        let drive_letter = (b'A' + letter as u8) as char;
        let root = format!("{drive_letter}:\\");
        let root_wide = windows::core::HSTRING::from(root.as_str());
        // SAFETY: `root_wide` is a valid null-terminated wide string; the
        // function only reads it.
        let drive_type = unsafe { GetDriveTypeW(&root_wide) };
        let mut label_buffer = [0u16; 256];
        // SAFETY: `label_buffer` is a valid, sufficiently sized buffer; every
        // other output parameter is `None`, which the API accepts.
        let has_media = unsafe {
            GetVolumeInformationW(&root_wide, Some(&mut label_buffer), None, None, None, None)
        }
        .is_ok();
        if !has_media {
            // No media in a removable/optical drive, or another transient
            // failure; skip it rather than show a broken entry.
            continue;
        }
        let label_len = label_buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(label_buffer.len());
        let label = String::from_utf16_lossy(&label_buffer[..label_len]);
        let kind = if drive_type == DRIVE_REMOVABLE {
            "Removable Disk"
        } else if drive_type == DRIVE_CDROM {
            "CD Drive"
        } else {
            "Local Disk"
        };
        let name = if label.is_empty() {
            format!("{kind} ({drive_letter}:)")
        } else {
            format!("{label} ({drive_letter}:)")
        };
        mounts.push(Mount {
            identity: format!("windows:{drive_letter}"),
            name,
            path: PathBuf::from(root),
            ejectable: drive_type == DRIVE_REMOVABLE || drive_type == DRIVE_CDROM,
        });
    }
    Ok(mounts)
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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "windows"))]
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

#[cfg(target_os = "windows")]
pub(crate) fn volume_usage(path: &Path) -> io::Result<Usage> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = HSTRING::from(path.as_os_str());
    let mut free_bytes_available = 0u64;
    let mut total_bytes = 0u64;
    // SAFETY: `wide` is a valid wide string; both output pointers are valid
    // for the duration of the call.
    unsafe {
        GetDiskFreeSpaceExW(
            &wide,
            Some(&mut free_bytes_available),
            Some(&mut total_bytes),
            None,
        )
    }
    .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(Usage {
        total: total_bytes,
        used: total_bytes.saturating_sub(free_bytes_available),
        available: free_bytes_available,
    })
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

#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", target_os = "macos", test))]
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
