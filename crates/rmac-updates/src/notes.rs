//! Lulo OS release notes, read from the archive's signed package index.
//!
//! A release's notes travel as the `Lulo-Release-Notes` field of
//! `rmac-session`'s control file, so the Lulo OS archive's signed
//! `InRelease` covers them through the `Packages` index hash: APT only moves
//! an index into `/var/lib/apt/lists` after checking it against the signed
//! Release. Nothing here downloads anything or runs a command; it reads the
//! index APT already verified. See docs/release-process.md "Release notes".

use std::path::{Path, PathBuf};

use super::*;

pub const RELEASE_NOTES_FIELD: &str = "Lulo-Release-Notes";
pub const APT_LISTS_DIR: &str = "/var/lib/apt/lists";
/// The notes' own limit (the packaging contract refuses more).
pub const MAX_NOTES_BYTES: usize = 4096;
/// A Packages index is read up to this size; Lulo OS's has five packages.
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INRELEASE_BYTES: u64 = 1024 * 1024;
const MAX_LIST_FILES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesBlock {
    /// A `# ` section heading, drawn bold like the Mac's "Siri AI".
    Heading(String),
    Paragraph(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseNotes {
    pub blocks: Vec<NotesBlock>,
}

impl ReleaseNotes {
    /// Decode the field's value as `parse_paragraphs` joined it: one line
    /// per continuation line, the leading space removed, `.` for a blank
    /// line.
    pub fn from_field(value: &str) -> Option<Self> {
        if value.len() > MAX_NOTES_BYTES * 2 {
            return None;
        }
        let mut blocks = Vec::new();
        let mut paragraph = String::new();
        let flush = |paragraph: &mut String, blocks: &mut Vec<NotesBlock>| {
            let text = paragraph.trim();
            if !text.is_empty() {
                blocks.push(NotesBlock::Paragraph(bounded_text(text)));
            }
            paragraph.clear();
        };
        for line in value.lines() {
            let line = line.trim_end();
            if line.chars().any(char::is_control) {
                return None;
            }
            if line.is_empty() || line == "." {
                flush(&mut paragraph, &mut blocks);
            } else if let Some(heading) = line.strip_prefix("# ") {
                flush(&mut paragraph, &mut blocks);
                let heading = bounded_text(heading);
                if !heading.is_empty() {
                    blocks.push(NotesBlock::Heading(heading));
                }
            } else {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(line.trim());
            }
        }
        flush(&mut paragraph, &mut blocks);
        (!blocks.is_empty()).then_some(Self { blocks })
    }
}

/// deb822 paragraphs of an index, continuation lines joined with `\n`.
pub fn parse_paragraphs(text: &str) -> Vec<std::collections::BTreeMap<String, String>> {
    let mut paragraphs = Vec::new();
    let mut current = std::collections::BTreeMap::<String, String>::new();
    let mut field: Option<String> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            field = None;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(value) = field.as_ref().and_then(|name| current.get_mut(name)) {
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(rest);
            }
            continue;
        }
        match line.split_once(':') {
            Some((name, value)) if !name.is_empty() && !name.contains(' ') => {
                current.insert(name.to_owned(), value.trim().to_owned());
                field = Some(name.to_owned());
            }
            _ => field = None,
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

/// Notes for `package` at exactly `version` in one Packages index.
pub fn notes_in_index(index: &str, package: &str, version: &str) -> Option<ReleaseNotes> {
    parse_paragraphs(index)
        .into_iter()
        .find(|paragraph| {
            paragraph.get("Package").map(String::as_str) == Some(package)
                && paragraph.get("Version").map(String::as_str) == Some(version)
        })
        .and_then(|paragraph| {
            paragraph
                .get(RELEASE_NOTES_FIELD)
                .and_then(|value| ReleaseNotes::from_field(value))
        })
}

/// True when an InRelease names the Lulo OS archive.
pub fn is_lulo_release(inrelease: &str) -> bool {
    let fields = inrelease
        .lines()
        .take_while(|line| {
            !line.starts_with("SHA256:") && !line.starts_with("-----BEGIN PGP SIGNATURE")
        })
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim(), value.trim()))
        .collect::<Vec<_>>();
    fields.contains(&("Origin", "rmac")) && fields.contains(&("Label", "rmac"))
}

/// The Lulo OS release notes for the offered `rmac-session` update, read
/// from `lists` (normally [`APT_LISTS_DIR`]). Only indices whose sibling
/// InRelease names the Lulo OS archive are read.
pub fn lulo_release_notes(lists: &Path, update: &Update) -> Option<ReleaseNotes> {
    let architecture = update.package_id.split(';').nth(2)?;
    if architecture.is_empty() || !architecture.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let suffix = format!("_dists_resolute_main_binary-{architecture}_Packages");
    let entries = std::fs::read_dir(lists).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .take(MAX_LIST_FILES)
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&suffix))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().find_map(|index| {
        let name = index.file_name()?.to_str()?;
        let prefix = name.strip_suffix(&suffix)?;
        let inrelease = index.with_file_name(format!("{prefix}_dists_resolute_InRelease"));
        if !is_lulo_release(&read_bounded(&inrelease, MAX_INRELEASE_BYTES)?) {
            return None;
        }
        notes_in_index(
            &read_bounded(&index, MAX_INDEX_BYTES)?,
            &update.name,
            &update.version,
        )
    })
}

fn read_bounded(path: &Path, maximum: u64) -> Option<String> {
    use std::io::Read as _;

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > maximum {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(maximum)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}
