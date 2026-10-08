//! Windows' own Recent items (`%APPDATA%\Microsoft\Windows\Recent`) for
//! Files' Recents on Windows (ADR 0023): the shortcuts Explorer, Office and
//! every other app leave there when a file is opened. Each `.lnk` is read
//! with a small parser of its `LinkInfo` block ([MS-SHLLINK] 2.3) instead
//! of COM's `IShellLink`, so this stays a plain, bounded read on the
//! background thread Recents already runs on: no index, no scan.

use std::path::{Path, PathBuf};

/// At most this many shortcuts are read, newest first (Windows itself
/// keeps about 150).
const MAX_LINKS: usize = 400;
/// A shortcut larger than this is not one Windows wrote.
const MAX_LINK_BYTES: u64 = 64 * 1024;

/// The files Windows' Recent folder points at, newest first, that still
/// exist (folders are left out, as the Mac's Recents lists documents).
#[cfg(windows)]
pub(crate) fn recent_files(limit: usize) -> Vec<PathBuf> {
    let Some(folder) = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|appdata| appdata.join(r"Microsoft\Windows\Recent"))
    else {
        return Vec::new();
    };
    recent_files_in(&folder, limit)
}

/// [`recent_files`] for the shortcuts in `folder`.
pub(crate) fn recent_files_in(folder: &Path, limit: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut links = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file() && metadata.len() <= MAX_LINK_BYTES)
                .then(|| (metadata.modified().ok(), entry.path()))
        })
        .take(4 * MAX_LINKS)
        .collect::<Vec<_>>();
    links.sort_by(|left, right| right.0.cmp(&left.0));
    let mut seen = std::collections::HashSet::new();
    links
        .into_iter()
        .take(MAX_LINKS)
        .filter_map(|(_, link)| std::fs::read(link).ok())
        .filter_map(|bytes| link_target(&bytes))
        .map(PathBuf::from)
        .filter(|target| target.is_file())
        .filter(|target| seen.insert(target.clone()))
        .take(limit)
        .collect()
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// A NUL-terminated string at `offset`: UTF-16 when `wide`, else the
/// system code page, read here as Latin-1 (paths with other characters
/// carry the Unicode form as well).
fn string_at(bytes: &[u8], offset: usize, wide: bool) -> Option<String> {
    let rest = bytes.get(offset..)?;
    if wide {
        let units = rest
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect::<Vec<_>>();
        String::from_utf16(&units).ok()
    } else {
        Some(
            rest.iter()
                .take_while(|&&byte| byte != 0)
                .map(|&byte| char::from(byte))
                .collect(),
        )
    }
}

/// The local path a shell link points at, from its `LinkInfo`, or `None`
/// for a link without one (a network share, a shell item).
pub(crate) fn link_target(bytes: &[u8]) -> Option<String> {
    const HEADER_SIZE: u32 = 0x4C;
    const HAS_LINK_TARGET_ID_LIST: u32 = 0x1;
    const HAS_LINK_INFO: u32 = 0x2;
    const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x1;
    if u32_at(bytes, 0)? != HEADER_SIZE {
        return None;
    }
    let flags = u32_at(bytes, 0x14)?;
    let mut offset = HEADER_SIZE as usize;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        offset += 2 + usize::from(u16_at(bytes, offset)?);
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }
    let size = u32_at(bytes, offset)? as usize;
    let info = bytes.get(offset..offset.checked_add(size)?)?;
    let header_size = u32_at(info, 4)?;
    if u32_at(info, 8)? & VOLUME_ID_AND_LOCAL_BASE_PATH == 0 {
        return None;
    }
    let (base, suffix) = if header_size >= 0x24 {
        (
            string_at(info, u32_at(info, 0x1C)? as usize, true)?,
            string_at(info, u32_at(info, 0x20)? as usize, true).unwrap_or_default(),
        )
    } else {
        (
            string_at(info, u32_at(info, 0x10)? as usize, false)?,
            string_at(info, u32_at(info, 0x18)? as usize, false).unwrap_or_default(),
        )
    };
    let path = format!("{base}{suffix}");
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell link with an ID list to skip and a `LinkInfo` naming
    /// `path` (ANSI, and UTF-16 when `wide`).
    fn link(path: &str, wide: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x4C];
        bytes[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&0x3u32.to_le_bytes());
        // An ID list of three bytes the parser must skip.
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&[9, 9, 9]);
        let header_size: u32 = if wide { 0x24 } else { 0x1C };
        let mut info = vec![0u8; header_size as usize];
        let ansi_offset = info.len() as u32;
        info.extend(path.bytes().chain([0]));
        let ansi_suffix = info.len() as u32;
        info.push(0);
        let (mut wide_offset, mut wide_suffix) = (0, 0);
        if wide {
            wide_offset = info.len() as u32;
            for unit in path.encode_utf16().chain([0]) {
                info.extend_from_slice(&unit.to_le_bytes());
            }
            wide_suffix = info.len() as u32;
            info.extend_from_slice(&[0, 0]);
        }
        let size = info.len() as u32;
        info[0..4].copy_from_slice(&size.to_le_bytes());
        info[4..8].copy_from_slice(&header_size.to_le_bytes());
        info[8..12].copy_from_slice(&1u32.to_le_bytes());
        info[0x10..0x14].copy_from_slice(&ansi_offset.to_le_bytes());
        info[0x18..0x1C].copy_from_slice(&ansi_suffix.to_le_bytes());
        if wide {
            info[0x1C..0x20].copy_from_slice(&wide_offset.to_le_bytes());
            info[0x20..0x24].copy_from_slice(&wide_suffix.to_le_bytes());
        }
        bytes.extend(info);
        bytes
    }

    #[test]
    fn shell_links_name_their_local_target() {
        assert_eq!(
            link_target(&link(r"C:\Users\me\Report.docx", false)).as_deref(),
            Some(r"C:\Users\me\Report.docx")
        );
        assert_eq!(
            link_target(&link(r"C:\Users\me\Café – notes.txt", true)).as_deref(),
            Some(r"C:\Users\me\Café – notes.txt")
        );
        assert_eq!(link_target(b"not a link"), None);
        assert_eq!(link_target(&[]), None);
        let mut truncated = link(r"C:\x.txt", true);
        truncated.truncate(0x60);
        assert_eq!(link_target(&truncated), None);
    }

    #[test]
    fn recent_files_follow_links_to_files_that_exist() {
        let root = std::env::temp_dir().join(format!(
            "rmac-windows-recent-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let recent = root.join("Recent");
        std::fs::create_dir_all(&recent).unwrap();
        let file = root.join("kept.txt");
        std::fs::write(&file, b"x").unwrap();
        let path = file.to_string_lossy().into_owned();
        std::fs::write(recent.join("kept.txt.lnk"), link(&path, true)).unwrap();
        std::fs::write(
            recent.join("folder.lnk"),
            link(&root.to_string_lossy(), true),
        )
        .unwrap();
        std::fs::write(recent.join("gone.lnk"), link("/nonexistent/gone.txt", true)).unwrap();
        std::fs::write(recent.join("notes.txt"), b"not a link").unwrap();
        assert_eq!(recent_files_in(&recent, 10), [file]);
        assert!(recent_files_in(&root.join("missing"), 10).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
