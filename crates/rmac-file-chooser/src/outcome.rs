//! Turning what the user chose into portal results — and nothing else.
//!
//! The panel is the only producer of a [`Selection`]. These checks re-verify
//! the choice at the moment of return: open targets must still exist with the
//! requested kind, save targets must be one valid name inside an existing
//! folder. Paths are returned as `file://` URIs; the portal frontend then
//! grants the sandboxed caller access to exactly those files.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::request::valid_file_name;

/// Portal response codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Success = 0,
    Cancelled = 1,
    Other = 2,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub paths: Vec<PathBuf>,
    /// `(choice id, selected option)` for every application choice.
    pub choices: Vec<(String, String)>,
    /// Index into the request's filters.
    pub current_filter: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Cancelled,
    Chosen(Selection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Empty,
    TooMany,
    NotAbsolute,
    Missing,
    WrongKind,
    InvalidName,
    NotAFolder,
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "Nothing is selected.",
            Self::TooMany => "Only one item can be chosen.",
            Self::NotAbsolute => "The location is not a full path.",
            Self::Missing => "The item no longer exists.",
            Self::WrongKind => "The item cannot be opened here.",
            Self::InvalidName => "The name can’t contain “/” and can’t be empty.",
            Self::NotAFolder => "The destination folder no longer exists.",
        })
    }
}

impl std::error::Error for Rejection {}

fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

/// Re-check an Open selection: absolute, present, and of the requested kind.
pub fn confirm_open(
    paths: &[PathBuf],
    directory: bool,
    multiple: bool,
) -> Result<Vec<PathBuf>, Rejection> {
    if paths.is_empty() {
        return Err(Rejection::Empty);
    }
    if paths.len() > 1 && !multiple {
        return Err(Rejection::TooMany);
    }
    let mut unique = BTreeSet::new();
    let mut confirmed = Vec::with_capacity(paths.len());
    for path in paths {
        if !plain_absolute(path) {
            return Err(Rejection::NotAbsolute);
        }
        let metadata = std::fs::metadata(path).map_err(|_| Rejection::Missing)?;
        if metadata.is_dir() != directory {
            return Err(Rejection::WrongKind);
        }
        if unique.insert(path.clone()) {
            confirmed.push(path.clone());
        }
    }
    Ok(confirmed)
}

/// The one file a Save writes: `folder/name`.
pub fn save_target(folder: &Path, name: &str) -> Result<PathBuf, Rejection> {
    if !valid_file_name(name) {
        return Err(Rejection::InvalidName);
    }
    if !plain_absolute(folder) {
        return Err(Rejection::NotAbsolute);
    }
    if !folder.is_dir() {
        return Err(Rejection::NotAFolder);
    }
    Ok(folder.join(name))
}

/// `stem n.ext`, the Finder numbering for a name that is already taken.
pub fn numbered_name(name: &str, number: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} {number}{}", &name[..dot], &name[dot..]),
        _ => format!("{name} {number}"),
    }
}

/// SaveFiles: one target per requested name inside `folder`. Names that
/// already exist (or repeat) are numbered, as the portal spec allows.
pub fn save_files_targets(folder: &Path, names: &[String]) -> Result<Vec<PathBuf>, Rejection> {
    if names.is_empty() {
        return Err(Rejection::Empty);
    }
    if !plain_absolute(folder) {
        return Err(Rejection::NotAbsolute);
    }
    if !folder.is_dir() {
        return Err(Rejection::NotAFolder);
    }
    let mut taken = BTreeSet::new();
    let mut targets = Vec::with_capacity(names.len());
    for name in names {
        if !valid_file_name(name) {
            return Err(Rejection::InvalidName);
        }
        let mut candidate = name.clone();
        let mut number = 2;
        while taken.contains(&candidate) || folder.join(&candidate).symlink_metadata().is_ok() {
            candidate = numbered_name(name, number);
            number += 1;
            if number > 10_000 {
                return Err(Rejection::InvalidName);
            }
        }
        taken.insert(candidate.clone());
        targets.push(folder.join(candidate));
    }
    Ok(targets)
}

pub fn file_uri(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

/// Portal results for a confirmed selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Results {
    pub uris: Vec<String>,
    pub choices: Vec<(String, String)>,
    pub current_filter: Option<usize>,
}

pub fn results(selection: &Selection) -> Option<Results> {
    let uris = selection
        .paths
        .iter()
        .map(|path| file_uri(path))
        .collect::<Option<Vec<_>>>()?;
    Some(Results {
        uris,
        choices: selection.choices.clone(),
        current_filter: selection.current_filter,
    })
}
