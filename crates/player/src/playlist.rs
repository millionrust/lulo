//! The files a Media Player window plays, in order.

use std::path::{Path, PathBuf};

/// Extensions Media Player opens; they match the MIME types its desktop
/// entry claims. libmpv decodes far more, but claiming them would route
/// files here that the Mac would not open in QuickTime Player either.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "flac", "ogg", "oga", "opus", "wav"];
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mpg", "mpeg", "ogv",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Audio,
    Video,
}

pub fn kind(path: &Path) -> Option<Kind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Some(Kind::Audio)
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    items: Vec<PathBuf>,
    index: usize,
}

impl Playlist {
    /// Keep the playable files in the order given; the first is current.
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut items = Vec::new();
        for path in paths {
            if kind(&path).is_some() && !items.contains(&path) {
                items.push(path);
            }
        }
        Self { items, index: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn current(&self) -> Option<&Path> {
        self.items.get(self.index).map(PathBuf::as_path)
    }

    pub fn has_next(&self) -> bool {
        self.index + 1 < self.items.len()
    }

    pub fn has_previous(&self) -> bool {
        self.index > 0
    }

    /// Move to the next item; `None` at the end.
    pub fn next(&mut self) -> Option<&Path> {
        if !self.has_next() {
            return None;
        }
        self.index += 1;
        self.current()
    }

    /// Previous item, following the Mac: within the first three seconds go
    /// back a track, later restart the current one (returns `None`).
    pub fn previous(&mut self, position_seconds: f64) -> Option<&Path> {
        if position_seconds > 3.0 || !self.has_previous() {
            return None;
        }
        self.index -= 1;
        self.current()
    }

    /// Add files (Open… while playing) and jump to the first new one.
    pub fn append(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> Option<&Path> {
        let first_new = self.items.len();
        for path in paths {
            if kind(&path).is_some() && !self.items.contains(&path) {
                self.items.push(path);
            }
        }
        if self.items.len() > first_new {
            self.index = first_new;
            self.current()
        } else {
            None
        }
    }
}

/// The window title for a file: its name without the extension.
pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind(Path::new("a.MP3")), Some(Kind::Audio));
        assert_eq!(kind(Path::new("b.mkv")), Some(Kind::Video));
        assert_eq!(kind(Path::new("c.txt")), None);
        assert_eq!(kind(Path::new("noext")), None);
    }

    #[test]
    fn keeps_playable_files_once_in_order() {
        let list = Playlist::new(paths(&["a.mp3", "notes.txt", "b.mp4", "a.mp3"]));
        assert_eq!(list.len(), 2);
        assert_eq!(list.current(), Some(Path::new("a.mp3")));
        assert!(Playlist::new(paths(&["x.pdf"])).is_empty());
    }

    #[test]
    fn next_and_previous() {
        let mut list = Playlist::new(paths(&["1.mp3", "2.mp3", "3.mp3"]));
        assert!(!list.has_previous());
        assert_eq!(list.next(), Some(Path::new("2.mp3")));
        assert_eq!(list.next(), Some(Path::new("3.mp3")));
        assert_eq!(list.next(), None);
        assert_eq!(list.index(), 2);
        // Late in a track, Previous restarts it.
        assert_eq!(list.previous(12.0), None);
        assert_eq!(list.index(), 2);
        assert_eq!(list.previous(1.0), Some(Path::new("2.mp3")));
        assert_eq!(list.previous(0.0), Some(Path::new("1.mp3")));
        assert_eq!(list.previous(0.0), None);
    }

    #[test]
    fn appending_jumps_to_the_new_files() {
        let mut list = Playlist::new(paths(&["1.mp3"]));
        assert_eq!(list.append(paths(&["1.mp3"])), None);
        assert_eq!(
            list.append(paths(&["2.mov", "3.flac"])),
            Some(Path::new("2.mov"))
        );
        assert_eq!(list.len(), 3);
        assert!(list.has_next());
    }

    #[test]
    fn names_drop_the_extension() {
        assert_eq!(
            display_name(Path::new("/m/Holiday Film.mp4")),
            "Holiday Film"
        );
        assert_eq!(display_name(Path::new("/m/.hidden")), ".hidden");
    }
}
