//! Expanding an archive next to itself.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{symlink, OpenOptionsExt as _};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::naming::{archive_stem, format_of, Format};
use crate::staging::{Counted, Meter, Scratch};
use crate::{Error, Progress};

const MAX_LINK_BYTES: u64 = 4096;
const DISK_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Expand `archive` beside itself and return what appeared: the single
/// top-level item, or a folder named after the archive holding several.
///
/// Nothing is visible under its final name until the whole archive has
/// expanded; on error or cancellation the private staging folder is removed.
pub fn expand(
    archive: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Error> {
    let format = format_of(archive).ok_or(Error::Unsupported)?;
    let metadata = fs::metadata(archive)?;
    if !metadata.is_file() {
        return Err(Error::Unsupported);
    }
    if !has_signature(archive, format)? {
        return Err(Error::Unsupported);
    }
    let parent = match archive.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let output_limit = expansion_budget(&parent, format)?;
    let staging = Scratch::directory(&parent, archive, "expanding")?;
    let length = metadata.len();
    let result = match format {
        Format::Zip => {
            let file = Counted::new(File::open(archive)?, Meter::new(cancel, progress, length));
            expand_zip(file, &staging.path, output_limit)
        }
        Format::Tar => {
            let file = Counted::new(File::open(archive)?, Meter::new(cancel, progress, length));
            expand_tar(BufReader::new(file), &staging.path, output_limit)
        }
        Format::TarGz => {
            let file = Counted::new(File::open(archive)?, Meter::new(cancel, progress, length));
            expand_tar(
                flate2::read::MultiGzDecoder::new(BufReader::new(file)),
                &staging.path,
                output_limit,
            )
        }
        Format::TarBz2 => {
            let file = Counted::new(File::open(archive)?, Meter::new(cancel, progress, length));
            expand_tar(
                bzip2::read::MultiBzDecoder::new(BufReader::new(file)),
                &staging.path,
                output_limit,
            )
        }
        Format::TarXz => expand_tar_xz(
            archive,
            &parent,
            &staging.path,
            length,
            output_limit,
            cancel,
            progress,
        ),
        Format::Gz | Format::Bz2 | Format::Xz => {
            let file = Counted::new(File::open(archive)?, Meter::new(cancel, progress, length));
            decompress_single(
                file,
                format,
                &staging.path.join(archive_stem(archive)),
                output_limit,
            )
        }
    };
    match result {
        Ok(()) if cancel.load(Ordering::Acquire) => Err(Error::Cancelled),
        Ok(()) => staging.finish(&parent, &archive_stem(archive)),
        Err(_) if cancel.load(Ordering::Acquire) => Err(Error::Cancelled),
        Err(error) => Err(error),
    }
}

fn expand_zip(file: Counted<'_, File>, staging: &Path, limit: u64) -> Result<(), Error> {
    let mut archive = zip::ZipArchive::new(file).map_err(|error| zip_error(error, true))?;
    let mut links = Vec::new();
    let mut expanded = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| zip_error(error, false))?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        if skipped(&relative) {
            continue;
        }
        let entry_size = entry.size();
        charge(&mut expanded, entry_size, limit)?;
        let target = staging.join(&relative);
        let mode = entry.unix_mode();
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if mode.is_some_and(|mode| mode & 0o170_000 == 0o120_000) {
            let mut link = String::new();
            (&mut entry)
                .take(MAX_LINK_BYTES)
                .read_to_string(&mut link)
                .map_err(data_error)?;
            links.push((target, PathBuf::from(link)));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode.map_or(0o644, |mode| mode & 0o777))
            .custom_flags(libc::O_NOFOLLOW)
            .open(&target)?;
        io::copy(&mut entry, &mut LimitedWriter::new(output, entry_size)).map_err(data_error)?;
    }
    // Links last, so no file entry can be written through one. A link can
    // still sit under an earlier link (`x -> ../../.config/autostart`, then
    // `x/evil.desktop`), so every folder above a link must be a real folder
    // inside the staging folder, never followed through a link.
    for (target, link) in links {
        real_parent_folders(staging, &target)?;
        symlink(link, target)?;
    }
    Ok(())
}

/// Create the folders between `staging` and `target`'s parent one component
/// at a time, refusing any component that already exists as anything but a
/// real folder (in particular a symbolic link from an earlier entry).
fn real_parent_folders(staging: &Path, target: &Path) -> Result<(), Error> {
    let relative = target.strip_prefix(staging).map_err(|_| Error::Damaged)?;
    let mut folder = staging.to_path_buf();
    let Some(parent) = relative.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let Component::Normal(name) = component else {
            return Err(Error::Damaged);
        };
        folder.push(name);
        match fs::symlink_metadata(&folder) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Err(Error::Damaged),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&folder)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn expand_tar<R: Read>(reader: R, staging: &Path, limit: u64) -> Result<(), Error> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);
    archive.set_unpack_xattrs(false);
    let mut first = true;
    let mut expanded = 0;
    let entries = archive.entries().map_err(|error| tar_error(error, true))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| tar_error(error, first))?;
        first = false;
        let kind = entry.header().entry_type();
        if kind.is_character_special()
            || kind.is_block_special()
            || kind.is_fifo()
            || kind.is_pax_global_extensions()
            || kind.is_pax_local_extensions()
            || kind.is_gnu_longname()
            || kind.is_gnu_longlink()
        {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| tar_error(error, false))?
            .into_owned();
        if skipped(&path) {
            continue;
        }
        let size = entry
            .header()
            .size()
            .map_err(|error| tar_error(error, false))?;
        charge(&mut expanded, size, limit)?;
        // `unpack_in` refuses absolute paths, `..` and writes through links
        // that leave the staging folder.
        entry
            .unpack_in(staging)
            .map_err(|error| tar_error(error, false))?;
    }
    Ok(())
}

fn expand_tar_xz(
    archive: &Path,
    parent: &Path,
    staging: &Path,
    length: u64,
    limit: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), Error> {
    // lzma-rs decodes xz from a reader into a writer, so the tar stream is
    // first written to a private file beside the staging folder.
    let payload = Scratch::file(parent, archive, "payload")?;
    {
        let mut input = BufReader::new(Counted::new(
            File::open(archive)?,
            Meter::new(cancel, progress, length.saturating_mul(2)),
        ));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&payload.path)?;
        lzma_rs::xz_decompress(&mut input, &mut LimitedWriter::new(&mut output, limit))
            .map_err(xz_error)?;
    }
    let payload_length = fs::metadata(&payload.path)?.len().max(1);
    let mut meter = Meter::new(cancel, progress, length.saturating_mul(2));
    meter.base = length;
    meter.numerator = length;
    meter.denominator = payload_length;
    expand_tar(
        BufReader::new(Counted::new(File::open(&payload.path)?, meter)),
        staging,
        limit,
    )
}

fn decompress_single(
    file: Counted<'_, File>,
    format: Format,
    target: &Path,
    limit: u64,
) -> Result<(), Error> {
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(target)?;
    let mut input = BufReader::new(file);
    let mut output = LimitedWriter::new(output, limit);
    match format {
        Format::Gz => {
            io::copy(&mut flate2::read::MultiGzDecoder::new(input), &mut output)
                .map_err(data_error)?;
        }
        Format::Bz2 => {
            io::copy(&mut bzip2::read::MultiBzDecoder::new(input), &mut output)
                .map_err(data_error)?;
        }
        _ => lzma_rs::xz_decompress(&mut input, &mut output).map_err(xz_error)?,
    }
    Ok(())
}

/// Keep one GiB available on the filesystem and cap any one expansion at
/// 16 GiB. tar.xz also needs a decoded tar scratch file while extracting, so
/// each of those two files gets half the available budget.
fn expansion_budget(parent: &Path, format: Format) -> Result<u64, Error> {
    let path = CString::new(parent.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `path` is NUL-terminated and `space` is valid writable storage.
    let mut space: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers remain valid for the duration of the call.
    if unsafe { libc::statvfs(path.as_ptr(), &mut space) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let available = (space.f_bavail as u64).saturating_mul(space.f_frsize as u64);
    let after_reserve = available.saturating_sub(DISK_RESERVE_BYTES);
    let budget = if format == Format::TarXz {
        (after_reserve / 2).min(MAX_EXPANDED_BYTES / 2)
    } else {
        after_reserve.min(MAX_EXPANDED_BYTES)
    };
    if budget == 0 {
        Err(io::Error::other(crate::EXPANSION_LIMIT).into())
    } else {
        Ok(budget)
    }
}

fn charge(expanded: &mut u64, bytes: u64, limit: u64) -> Result<(), Error> {
    if bytes > limit.saturating_sub(*expanded) {
        return Err(io::Error::other(crate::EXPANSION_LIMIT).into());
    }
    *expanded += bytes;
    Ok(())
}

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other(crate::EXPANSION_LIMIT));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Leftovers the Mac writes into zips for its own resource forks.
fn skipped(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(first)) if first == "__MACOSX"
    )
}

fn has_signature(path: &Path, format: Format) -> io::Result<bool> {
    let mut head = [0_u8; 512];
    let mut file = File::open(path)?;
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    let head = &head[..filled];
    Ok(match format {
        Format::Zip => {
            head.starts_with(b"PK\x03\x04")
                || head.starts_with(b"PK\x05\x06")
                || head.starts_with(b"PK\x07\x08")
        }
        Format::TarGz | Format::Gz => head.starts_with(&[0x1f, 0x8b]),
        Format::TarBz2 | Format::Bz2 => head.starts_with(b"BZh"),
        Format::TarXz | Format::Xz => head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]),
        Format::Tar => is_tar_header(head),
    })
}

/// A POSIX/GNU tar starts with a 512-byte header whose checksum field is the
/// sum of the header's bytes with that field counted as spaces.
fn is_tar_header(head: &[u8]) -> bool {
    if head.len() < 512 {
        return false;
    }
    if &head[257..262] == b"ustar" {
        return true;
    }
    let field = &head[148..156];
    let digits: String = field
        .iter()
        .map(|byte| *byte as char)
        .filter(|character| ('0'..='7').contains(character))
        .collect();
    let Ok(expected) = u64::from_str_radix(&digits, 8) else {
        return false;
    };
    let sum: u64 = head[..512]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    !digits.is_empty() && sum == expected
}

fn zip_error(error: zip::result::ZipError, opening: bool) -> Error {
    match error {
        zip::result::ZipError::Io(error) => data_error(error),
        _ if opening => Error::Unsupported,
        _ => Error::Damaged,
    }
}

fn tar_error(error: io::Error, first: bool) -> Error {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof | io::ErrorKind::Other
            if first =>
        {
            Error::Unsupported
        }
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => Error::Damaged,
        _ => Error::from(error),
    }
}

fn xz_error(error: lzma_rs::error::Error) -> Error {
    match error {
        lzma_rs::error::Error::IoError(error) => data_error(error),
        _ => Error::Damaged,
    }
}

fn data_error(error: io::Error) -> Error {
    match error.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => Error::Damaged,
        _ => Error::from(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_budget_charge_refuses_overflow() {
        let mut expanded = 0;
        charge(&mut expanded, 7, 10).unwrap();
        charge(&mut expanded, 3, 10).unwrap();
        assert!(matches!(
            charge(&mut expanded, 1, 10),
            Err(Error::ExpansionLimit)
        ));
    }

    #[test]
    fn limited_writer_never_writes_past_its_budget() {
        let mut bytes = Vec::new();
        let mut writer = LimitedWriter::new(&mut bytes, 3);
        writer.write_all(b"abc").unwrap();
        assert_eq!(
            writer.write_all(b"d").unwrap_err().to_string(),
            crate::EXPANSION_LIMIT
        );
        assert_eq!(bytes, b"abc");
    }

    #[test]
    fn zip_expansion_refuses_an_oversized_entry_and_leaves_no_output() {
        let root = scratch("zip-limit");
        let archive = root.join("Bundle.zip");
        let staging = root.join("staging");
        fs::create_dir(&staging).unwrap();
        zip_with(&archive, &[("large.txt", "four")]);
        let cancel = AtomicBool::new(false);
        let mut progress = |_| {};
        let file = Counted::new(
            File::open(&archive).unwrap(),
            Meter::new(
                &cancel,
                &mut progress,
                fs::metadata(&archive).unwrap().len(),
            ),
        );
        assert!(matches!(
            expand_zip(file, &staging, 3),
            Err(Error::ExpansionLimit)
        ));
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_expansion_refuses_an_oversized_entry() {
        let root = scratch("tar-limit");
        let archive = root.join("Bundle.tar");
        let staging = root.join("staging");
        fs::create_dir(&staging).unwrap();
        {
            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "large.txt", &b"four"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        assert!(matches!(
            expand_tar(File::open(&archive).unwrap(), &staging, 3),
            Err(Error::ExpansionLimit)
        ));
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_gzip_expansion_refuses_an_oversized_output() {
        let root = scratch("gzip-limit");
        let archive = root.join("large.txt.gz");
        let target = root.join("large.txt");
        {
            let mut encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            encoder.write_all(b"four").unwrap();
            encoder.finish().unwrap();
        }
        let cancel = AtomicBool::new(false);
        let mut progress = |_| {};
        let file = Counted::new(
            File::open(&archive).unwrap(),
            Meter::new(
                &cancel,
                &mut progress,
                fs::metadata(&archive).unwrap().len(),
            ),
        );
        assert!(matches!(
            decompress_single(file, Format::Gz, &target, 3),
            Err(Error::ExpansionLimit)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rmac-archive-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn zip_with(path: &Path, entries: &[(&str, &str)]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, body) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    fn run(path: &Path) -> Result<PathBuf, Error> {
        let cancel = AtomicBool::new(false);
        expand(path, &cancel, &mut |_| {})
    }

    #[test]
    fn several_items_expand_into_a_folder_named_after_the_archive() {
        let root = scratch("several");
        let archive = root.join("Bundle.zip");
        zip_with(&archive, &[("notes.txt", "one"), ("code.rs", "two")]);
        let first = run(&archive).unwrap();
        assert_eq!(first, root.join("Bundle"));
        assert_eq!(fs::read_to_string(first.join("notes.txt")).unwrap(), "one");
        assert_eq!(run(&archive).unwrap(), root.join("Bundle 2"));
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_item_expands_to_itself_with_a_mac_number() {
        let root = scratch("single");
        fs::write(root.join("notes.txt"), "existing").unwrap();
        let archive = root.join("Bundle.tar.gz");
        {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "notes.txt", &b"new"[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let placed = run(&archive).unwrap();
        assert_eq!(placed, root.join("notes 2.txt"));
        assert_eq!(fs::read_to_string(placed).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn garbage_is_an_unsupported_format_and_leaves_nothing() {
        let root = scratch("garbage");
        let archive = root.join("Broken.zip");
        fs::write(&archive, "garbage\n").unwrap();
        assert!(matches!(run(&archive), Err(Error::Unsupported)));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn traversal_entries_are_never_written_outside() {
        let root = scratch("traversal");
        let inner = root.join("inner");
        fs::create_dir_all(&inner).unwrap();
        let archive = inner.join("Evil.zip");
        zip_with(&archive, &[("../escaped.txt", "x"), ("safe.txt", "y")]);
        let placed = run(&archive).unwrap();
        assert!(!root.join("escaped.txt").exists());
        assert_eq!(placed, inner.join("safe.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_link_is_never_created_through_an_earlier_link() {
        let root = scratch("link-through-link");
        let outside = root.join("outside");
        let inner = root.join("inner");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&inner).unwrap();
        let archive = inner.join("Evil.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("payload.txt", options).unwrap();
            writer.write_all(b"x").unwrap();
            // Staging is inner/.Evil…, so `../../outside` is root/outside.
            writer.add_symlink("x", "../../outside", options).unwrap();
            writer
                .add_symlink("x/planted", "../inner/payload.txt", options)
                .unwrap();
            writer.finish().unwrap();
        }
        assert!(matches!(run(&archive), Err(Error::Damaged)));
        assert!(fs::symlink_metadata(outside.join("planted")).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        let leftovers = fs::read_dir(&inner)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn links_inside_real_folders_still_expand() {
        let root = scratch("nested-link");
        let archive = root.join("Bundle.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("docs/readme.txt", options).unwrap();
            writer.write_all(b"hi").unwrap();
            writer
                .add_symlink("links/deep/readme", "../../docs/readme.txt", options)
                .unwrap();
            writer.finish().unwrap();
        }
        let placed = run(&archive).unwrap();
        assert_eq!(
            fs::read_to_string(placed.join("links/deep/readme")).unwrap(),
            "hi"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelling_removes_the_staging_folder() {
        let root = scratch("cancel");
        let archive = root.join("Bundle.zip");
        zip_with(&archive, &[("a.txt", "a"), ("b.txt", "b")]);
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            expand(&archive, &cancel, &mut |_| {}),
            Err(Error::Cancelled)
        ));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_gzip_file_drops_its_suffix() {
        let root = scratch("gzip");
        let archive = root.join("notes.txt.gz");
        {
            let mut encoder = flate2::write::GzEncoder::new(
                File::create(&archive).unwrap(),
                flate2::Compression::default(),
            );
            encoder.write_all(b"hello").unwrap();
            encoder.finish().unwrap();
        }
        let placed = run(&archive).unwrap();
        assert_eq!(placed, root.join("notes.txt"));
        assert_eq!(fs::read_to_string(placed).unwrap(), "hello");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tar_signature_accepts_ustar_and_rejects_text() {
        let mut header = [0_u8; 512];
        header[257..262].copy_from_slice(b"ustar");
        assert!(is_tar_header(&header));
        assert!(!is_tar_header(&[b'a'; 512]));
        assert!(!is_tar_header(b"short"));
    }
}
