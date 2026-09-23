//! Archive formats and the Mac's naming rules.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    /// A single gzip-compressed file ("notes.txt.gz").
    Gz,
    Bz2,
    Xz,
}

/// Suffixes longest first so ".tar.gz" wins over ".gz".
const SUFFIXES: &[(&str, Format)] = &[
    (".tar.gz", Format::TarGz),
    (".tar.bz2", Format::TarBz2),
    (".tar.xz", Format::TarXz),
    (".tgz", Format::TarGz),
    (".tbz2", Format::TarBz2),
    (".tbz", Format::TarBz2),
    (".txz", Format::TarXz),
    (".zip", Format::Zip),
    (".tar", Format::Tar),
    (".gz", Format::Gz),
    (".bz2", Format::Bz2),
    (".xz", Format::Xz),
];

/// The archive format a file name claims, if any (case-insensitive).
pub fn format_of(path: &Path) -> Option<Format> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| name.len() > suffix.len() && name.ends_with(suffix))
        .map(|(_, format)| *format)
}

/// The archive's name without its archive suffix: "Bundle.tar.gz" →
/// "Bundle", "notes.txt.gz" → "notes.txt".
pub fn archive_stem(path: &Path) -> String {
    let name = crate::display_name(path);
    let lower = name.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.len() > suffix.len() && lower.ends_with(suffix))
        .map(|(suffix, _)| name[..name.len() - suffix.len()].to_owned())
        .unwrap_or(name)
}

/// `directory/name`, or the Mac's next free variant: "notes 2.txt",
/// "Bundle 2", "Archive 3.zip". A leading dot never counts as an extension.
pub fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let first = directory.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, extension) = split_extension(name);
    (2..)
        .map(|number| directory.join(numbered(stem, extension, number)))
        .find(|candidate| !exists(candidate))
        .expect("an unbounded range yields a free name")
}

pub(crate) fn numbered(stem: &str, extension: Option<&str>, number: u64) -> String {
    match extension {
        Some(extension) => format!("{stem} {number}.{extension}"),
        None => format!("{stem} {number}"),
    }
}

pub(crate) fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(index) if index > 0 && index + 1 < name.len() => {
            (&name[..index], Some(&name[index + 1..]))
        }
        _ => (name, None),
    }
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_come_from_the_whole_suffix() {
        assert_eq!(format_of(Path::new("a/Bundle.zip")), Some(Format::Zip));
        assert_eq!(format_of(Path::new("Bundle.TAR.GZ")), Some(Format::TarGz));
        assert_eq!(format_of(Path::new("x.tgz")), Some(Format::TarGz));
        assert_eq!(format_of(Path::new("x.tar.xz")), Some(Format::TarXz));
        assert_eq!(format_of(Path::new("x.txz")), Some(Format::TarXz));
        assert_eq!(format_of(Path::new("x.tar.bz2")), Some(Format::TarBz2));
        assert_eq!(format_of(Path::new("notes.txt.gz")), Some(Format::Gz));
        assert_eq!(format_of(Path::new("x.tar")), Some(Format::Tar));
        assert_eq!(format_of(Path::new(".zip")), None);
        assert_eq!(format_of(Path::new("notes.txt")), None);
    }

    #[test]
    fn stems_drop_only_the_archive_suffix() {
        assert_eq!(archive_stem(Path::new("/d/Bundle.tar.gz")), "Bundle");
        assert_eq!(archive_stem(Path::new("/d/Bundle.zip")), "Bundle");
        assert_eq!(archive_stem(Path::new("/d/notes.txt.gz")), "notes.txt");
        assert_eq!(archive_stem(Path::new("/d/plain")), "plain");
    }

    #[test]
    fn unique_names_follow_the_mac() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-archive-naming-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(unique_path(&directory, "Bundle"), directory.join("Bundle"));
        std::fs::create_dir(directory.join("Bundle")).unwrap();
        assert_eq!(
            unique_path(&directory, "Bundle"),
            directory.join("Bundle 2")
        );
        std::fs::write(directory.join("notes.txt"), "x").unwrap();
        assert_eq!(
            unique_path(&directory, "notes.txt"),
            directory.join("notes 2.txt")
        );
        std::fs::write(directory.join("Archive.zip"), "x").unwrap();
        std::fs::write(directory.join("Archive 2.zip"), "x").unwrap();
        assert_eq!(
            unique_path(&directory, "Archive.zip"),
            directory.join("Archive 3.zip")
        );
        std::fs::write(directory.join(".hidden"), "x").unwrap();
        assert_eq!(
            unique_path(&directory, ".hidden"),
            directory.join(".hidden 2")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
