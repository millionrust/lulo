//! Finder's File ▸ Compress: one item → "name.zip", several → "Archive.zip".

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use zip::write::SimpleFileOptions;

use crate::staging::{place, Meter, Scratch};
use crate::{display_name, Error, Progress};

const CHUNK: usize = 256 * 1024;
/// Entries at or above this size need zip64 headers.
const ZIP64_THRESHOLD: u64 = u32::MAX as u64;

/// The name Finder gives the archive before any " 2" numbering.
pub fn compressed_name(items: &[PathBuf]) -> String {
    match items {
        [one] => format!("{}.zip", display_name(one)),
        _ => "Archive.zip".to_owned(),
    }
}

enum Kind {
    Directory,
    File { size: u64 },
    Link(String),
}

struct Planned {
    path: PathBuf,
    name: String,
    kind: Kind,
    mode: u32,
    modified: Option<SystemTime>,
}

/// Write the zip beside the first item and return its path. The archive is
/// built under a private name and appears only when complete.
pub fn compress(
    items: &[PathBuf],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Error> {
    let first = items
        .first()
        .ok_or_else(|| Error::Io(io::Error::from(io::ErrorKind::InvalidInput)))?;
    let parent = match first.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = compressed_name(items);

    let mut plan = Vec::new();
    let mut total = 0_u64;
    for item in items {
        plan_item(item, display_name(item), &mut plan, &mut total, cancel)?;
    }

    let mut scratch = Scratch::file(&parent, Path::new(&name), "compressing")?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&scratch.path)?;
    let mut writer = zip::ZipWriter::new(file);
    let mut meter = Meter::new(cancel, progress, total);
    let mut buffer = vec![0_u8; CHUNK];
    for entry in plan {
        meter.check()?;
        let mut options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(entry.mode);
        if let Some(time) = entry.modified.and_then(zip_time) {
            options = options.last_modified_time(time);
        }
        match entry.kind {
            Kind::Directory => writer
                .add_directory(format!("{}/", entry.name), options)
                .map_err(write_error)?,
            Kind::Link(target) => writer
                .add_symlink(entry.name, target, options)
                .map_err(write_error)?,
            Kind::File { size } => {
                writer
                    .start_file(entry.name, options.large_file(size >= ZIP64_THRESHOLD))
                    .map_err(write_error)?;
                let mut input = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&entry.path)?;
                loop {
                    meter.check()?;
                    let read = input.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    writer.write_all(&buffer[..read])?;
                    meter.add(read as u64);
                }
            }
        }
    }
    writer.finish().map_err(write_error)?.sync_all()?;
    let placed = place(&scratch.path, &parent, &name)?;
    scratch.kept = true;
    Ok(placed)
}

fn plan_item(
    path: &Path,
    name: String,
    plan: &mut Vec<Planned>,
    total: &mut u64,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    let metadata = fs::symlink_metadata(path)?;
    let mode = metadata.permissions().mode() & 0o7777;
    let modified = metadata.modified().ok();
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(path)?.to_string_lossy().into_owned();
        plan.push(Planned {
            path: path.to_path_buf(),
            name,
            kind: Kind::Link(target),
            mode,
            modified,
        });
    } else if file_type.is_dir() {
        plan.push(Planned {
            path: path.to_path_buf(),
            name: name.clone(),
            kind: Kind::Directory,
            mode,
            modified,
        });
        let mut children = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            let child_name = format!("{name}/{}", child.to_string_lossy());
            plan_item(&path.join(&child), child_name, plan, total, cancel)?;
        }
    } else if file_type.is_file() {
        *total = total.saturating_add(metadata.len());
        plan.push(Planned {
            path: path.to_path_buf(),
            name,
            kind: Kind::File {
                size: metadata.len(),
            },
            mode,
            modified,
        });
    }
    // Sockets, pipes and devices have no zip representation; Finder skips
    // them too.
    Ok(())
}

fn zip_time(time: SystemTime) -> Option<zip::DateTime> {
    use chrono::{Datelike as _, Timelike as _};
    let local: chrono::DateTime<chrono::Local> = time.into();
    zip::DateTime::from_date_and_time(
        u16::try_from(local.year()).ok()?,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .ok()
}

fn write_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::from(error),
        other => Error::Io(io::Error::other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rmac-archive-compress-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn names_follow_finder() {
        assert_eq!(
            compressed_name(&[PathBuf::from("/d/notes.txt")]),
            "notes.txt.zip"
        );
        assert_eq!(
            compressed_name(&[PathBuf::from("/d/Folder A")]),
            "Folder A.zip"
        );
        assert_eq!(
            compressed_name(&[PathBuf::from("/d/a"), PathBuf::from("/d/b")]),
            "Archive.zip"
        );
    }

    #[test]
    fn compress_then_expand_round_trips() {
        let root = scratch("round-trip");
        fs::write(root.join("notes.txt"), "one").unwrap();
        fs::create_dir(root.join("Folder A")).unwrap();
        fs::write(root.join("Folder A/a.txt"), "a").unwrap();
        let cancel = AtomicBool::new(false);
        let items = [root.join("notes.txt"), root.join("Folder A")];
        let first = compress(&items, &cancel, &mut |_| {}).unwrap();
        assert_eq!(first, root.join("Archive.zip"));
        let second = compress(&items, &cancel, &mut |_| {}).unwrap();
        assert_eq!(second, root.join("Archive 2.zip"));

        let expanded = crate::expand(&first, &cancel, &mut |_| {}).unwrap();
        assert_eq!(expanded, root.join("Archive"));
        assert_eq!(
            fs::read_to_string(expanded.join("notes.txt")).unwrap(),
            "one"
        );
        assert_eq!(
            fs::read_to_string(expanded.join("Folder A/a.txt")).unwrap(),
            "a"
        );

        let single = compress(&[root.join("notes.txt")], &cancel, &mut |_| {}).unwrap();
        assert_eq!(single, root.join("notes.txt.zip"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelled_compression_leaves_no_archive() {
        let root = scratch("cancel");
        fs::write(root.join("notes.txt"), "one").unwrap();
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            compress(&[root.join("notes.txt")], &cancel, &mut |_| {}),
            Err(Error::Cancelled)
        ));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
