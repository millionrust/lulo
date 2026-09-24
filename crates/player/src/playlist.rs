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

/// Most files one window's playlist holds. Opening more (from Finder or an
/// MPRIS client's `OpenUri`) is ignored rather than growing without bound.
pub const MAX_ITEMS: usize = 10_000;
/// Longest URI `OpenUri` accepts.
pub const MAX_URI_BYTES: usize = 8 * 1024;

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
            if items.len() >= MAX_ITEMS {
                break;
            }
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
    pub fn advance(&mut self) -> Option<&Path> {
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
            if self.items.len() >= MAX_ITEMS {
                break;
            }
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

/// The local file an MPRIS `OpenUri` names, or `None` when the request is not
/// one Media Player plays. MPRIS peers include sandboxed apps, so only a
/// bounded `file:///` URI with no host, whose decoded path is absolute, has
/// no `..`, has a playable extension and is an existing regular file, is
/// accepted.
pub fn path_from_open_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;

    if uri.len() > MAX_URI_BYTES {
        return None;
    }
    let encoded = uri.strip_prefix("file://")?;
    if !encoded.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(encoded);
    if decoded.contains(&0) {
        return None;
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(decoded));
    let plain = path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, std::path::Component::ParentDir));
    if !plain || kind(&path).is_none() {
        return None;
    }
    std::fs::metadata(&path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map(|_| path)
}

/// Decode `%XX` escapes; a malformed escape is kept as written.
pub fn percent_decode(text: &str) -> Vec<u8> {
    let hex = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                out.push(high << 4 | low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
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
        assert_eq!(list.advance(), Some(Path::new("2.mp3")));
        assert_eq!(list.advance(), Some(Path::new("3.mp3")));
        assert_eq!(list.advance(), None);
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

    #[test]
    fn file_uris_decode() {
        assert_eq!(
            percent_decode("/home/me/My%20Song.mp3"),
            b"/home/me/My Song.mp3"
        );
        assert_eq!(percent_decode("/a%2"), b"/a%2");
        assert_eq!(percent_decode("/%E2%9C%93"), "/\u{2713}".as_bytes());
    }

    #[test]
    fn open_uri_accepts_only_bounded_existing_local_media() {
        let directory =
            std::env::temp_dir().join(format!("rmac-player-uri-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let song = directory.join("My Song.mp3");
        std::fs::write(&song, b"id3").unwrap();
        let text = directory.join("notes.txt");
        std::fs::write(&text, b"x").unwrap();
        let uri = |path: &Path| format!("file://{}", path.display()).replace(' ', "%20");

        assert_eq!(path_from_open_uri(&uri(&song)), Some(song.clone()));
        // Not local, not absolute, not playable, missing, or escaping.
        assert_eq!(
            path_from_open_uri(&format!("file://host{}", song.display())),
            None
        );
        assert_eq!(path_from_open_uri("https://example.com/a.mp3"), None);
        assert_eq!(path_from_open_uri("file://relative.mp3"), None);
        assert_eq!(path_from_open_uri(&uri(&text)), None);
        assert_eq!(path_from_open_uri(&uri(&directory.join("gone.mp3"))), None);
        assert_eq!(
            path_from_open_uri(&format!(
                "file://{}/../x/My%20Song.mp3",
                directory.display()
            )),
            None
        );
        assert_eq!(path_from_open_uri("file:///a%00b.mp3"), None);
        let long = format!("file:///{}.mp3", "a".repeat(MAX_URI_BYTES));
        assert_eq!(path_from_open_uri(&long), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_playlist_never_grows_past_its_bound() {
        let many = (0..MAX_ITEMS + 5).map(|index| PathBuf::from(format!("/m/{index}.mp3")));
        let mut list = Playlist::new(many);
        assert_eq!(list.len(), MAX_ITEMS);
        assert_eq!(list.append(paths(&["/m/extra.mp3"])), None);
        assert_eq!(list.len(), MAX_ITEMS);
    }
}
