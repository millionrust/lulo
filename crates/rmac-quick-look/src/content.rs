//! Loading what Quick Look shows for one item. Everything here blocks and
//! runs on a background thread; every path is re-checked against the
//! identity it had when loading began, so a file swapped mid-load is never
//! shown under the old name.

use std::fs::OpenOptions;
use std::io::{self, Read as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{ImageSource, RenderImage};
use rmac_preview::document::Kind;
use rmac_preview::layout::Rotation;
use rmac_preview::render;

use crate::metrics;

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_FOLDER_WALK: usize = 20_000;
const MAX_LINK_CHARACTERS: usize = 1_024;

/// Picture data that can cross threads (GPUI's `ImageSource` cannot).
#[derive(Clone)]
pub enum Picture {
    Pixels(Arc<RenderImage>),
    File(PathBuf),
}

impl Picture {
    pub fn source(&self) -> ImageSource {
        match self {
            Self::Pixels(image) => ImageSource::from(image.clone()),
            Self::File(path) => ImageSource::from(path.clone()),
        }
    }
}

#[derive(Clone)]
pub enum Content {
    /// A picture: an image, a video's poster frame or an audio waveform.
    Image {
        picture: Picture,
        /// Natural size in points (pixels of the picture).
        size: (f32, f32),
    },
    /// A PDF: the pages rendered so far, and the first page's size in points.
    Pdf {
        path: PathBuf,
        pages: Vec<Arc<RenderImage>>,
        page_sizes: Vec<(f32, f32)>,
        pixel_scale: f32,
    },
    Text {
        text: String,
        truncated: bool,
    },
    /// Folders, archives and everything without a preview: icon + facts.
    Summary(Summary),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub folder: bool,
    pub archive: bool,
    /// "364 bytes" · "2 bytes, 2 items" · "Symbolic link to …"
    pub detail: String,
    /// "Last modified on 23 Sep 2026 at 3:38:45 PM"
    pub modified: Option<String>,
}

impl Content {
    /// Natural content size for sizing the panel (`None` for a summary).
    pub fn natural_size(&self) -> Option<(f32, f32)> {
        match self {
            Self::Image { size, .. } => Some(*size),
            Self::Pdf { page_sizes, .. } => page_sizes.first().copied(),
            Self::Text { .. } => Some(metrics::TEXT_CONTENT),
            Self::Summary(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl Identity {
    fn capture(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        }
    }

    fn still_matches_path(self, path: &Path) -> io::Result<bool> {
        Ok(Self::capture(&std::fs::symlink_metadata(path)?) == self)
    }
}

/// Load `path` for a panel whose content may be up to `limits` (points).
pub fn load(path: &Path, limits: (f32, f32), cancel: &AtomicBool) -> io::Result<Content> {
    check_cancelled(cancel)?;
    let metadata = std::fs::symlink_metadata(path)?;
    let identity = Identity::capture(&metadata);
    let modified = metadata.modified().ok().map(modified_line);
    if metadata.is_dir() {
        return load_folder(path, identity, modified, cancel);
    }
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        if !identity.still_matches_path(path)? {
            return Err(changed());
        }
        return Ok(Content::Summary(Summary {
            folder: false,
            archive: false,
            detail: format!(
                "Symbolic link to {}",
                bounded_display(&target.to_string_lossy(), MAX_LINK_CHARACTERS)
            ),
            modified,
        }));
    }
    let file_summary = |archive: bool| {
        Content::Summary(Summary {
            folder: false,
            archive,
            detail: rmac_archive::size_label(metadata.len()),
            modified: modified.clone(),
        })
    };
    if !metadata.is_file() {
        return Ok(file_summary(false));
    }
    if rmac_archive::format_of(path).is_some() {
        return Ok(file_summary(true));
    }

    // Preview's own decoders first: the same pixels and pages Preview shows.
    match render::sniff_path(path) {
        Ok(Kind::Image(_)) => {
            let loaded = render::load(path).map_err(io::Error::other)?;
            check_cancelled(cancel)?;
            if !identity.still_matches_path(path)? {
                return Err(changed());
            }
            if let render::Content::Image(image) = loaded.content {
                let size = (image.size.0 as f32, image.size.1 as f32);
                let pixels = Arc::unwrap_or_clone(image.pixels);
                return Ok(Content::Image {
                    picture: Picture::Pixels(render::to_render_image(pixels)),
                    size,
                });
            }
        }
        Ok(Kind::Pdf) => return load_pdf(path, identity, limits, cancel),
        Err(_) => {}
    }

    if rmac_thumbnails::is_supported(path) {
        let preview = rmac_thumbnails::generate_preview(path).map_err(io::Error::other)?;
        check_cancelled(cancel)?;
        if !identity.still_matches_path(path)? {
            return Err(changed());
        }
        return Ok(picture(preview));
    }
    if rmac_thumbnails::media_kind(path).is_some() {
        return match rmac_thumbnails::generate_media_preview(path, cancel) {
            Ok(media) => {
                if !identity.still_matches_path(path)? {
                    return Err(changed());
                }
                Ok(picture(media.preview))
            }
            // Without FFmpeg the Mac-style fallback is the icon and facts.
            Err(rmac_thumbnails::Error::ConverterUnavailable { .. }) => Ok(file_summary(false)),
            Err(rmac_thumbnails::Error::Cancelled) => Err(cancelled()),
            Err(error) => Err(io::Error::other(error)),
        };
    }

    match load_regular_text(path, identity, cancel)? {
        Some(text) => Ok(text),
        None => Ok(file_summary(false)),
    }
}

/// A picture file from the thumbnail cache.
fn picture(preview: PathBuf) -> Content {
    let size = render::image_dimensions(&preview)
        .map(|(width, height)| (width as f32, height as f32))
        .unwrap_or(metrics::TEXT_CONTENT);
    Content::Image {
        picture: Picture::File(preview),
        size,
    }
}

/// First page now; the panel asks for the rest with [`render_pdf_page`].
fn load_pdf(
    path: &Path,
    identity: Identity,
    limits: (f32, f32),
    cancel: &AtomicBool,
) -> io::Result<Content> {
    let loaded = render::load(path).map_err(io::Error::other)?;
    let render::Content::Pdf(info) = loaded.content else {
        return Err(io::Error::other("not a PDF"));
    };
    let page_sizes: Vec<(f32, f32)> = info.pages.iter().map(|page| page.displayed()).collect();
    let Some(first) = page_sizes.first().copied() else {
        return Err(io::Error::other("The PDF has no pages."));
    };
    let shown = metrics::fit(first, limits);
    // Device pixels per point: HiDPI (2×) at the size the page is shown.
    let pixel_scale = 2.0 * (shown.0 / first.0.max(1.0));
    check_cancelled(cancel)?;
    let page = render_pdf_page(path, 0, first, pixel_scale)?;
    if !identity.still_matches_path(path)? {
        return Err(changed());
    }
    Ok(Content::Pdf {
        path: path.to_path_buf(),
        pages: vec![page],
        page_sizes,
        pixel_scale,
    })
}

/// Rasterise one PDF page with Preview's poppler renderer.
pub fn render_pdf_page(
    path: &Path,
    index: usize,
    page_size: (f32, f32),
    pixel_scale: f32,
) -> io::Result<Arc<RenderImage>> {
    render::render_page(path, index, page_size, pixel_scale, Rotation::default())
        .map(render::to_render_image)
        .map_err(io::Error::other)
}

fn load_folder(
    path: &Path,
    identity: Identity,
    modified: Option<String>,
    cancel: &AtomicBool,
) -> io::Result<Content> {
    let mut items = 0usize;
    for entry in std::fs::read_dir(path)? {
        check_cancelled(cancel)?;
        entry?;
        items += 1;
    }
    // Total size of everything inside, as Finder's Quick Look shows it;
    // bounded so a huge tree never stalls the panel.
    let mut bytes = 0u64;
    let mut visited = 0usize;
    let mut pending = vec![path.to_path_buf()];
    let mut complete = true;
    'walk: while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            check_cancelled(cancel)?;
            visited += 1;
            if visited > MAX_FOLDER_WALK {
                complete = false;
                break 'walk;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    if !identity.still_matches_path(path)? {
        return Err(changed());
    }
    let count = format!("{items} item{}", if items == 1 { "" } else { "s" });
    Ok(Content::Summary(Summary {
        folder: true,
        archive: false,
        detail: if complete {
            format!("{}, {count}", rmac_archive::size_label(bytes))
        } else {
            count
        },
        modified,
    }))
}

fn load_regular_text(
    path: &Path,
    expected: Identity,
    cancel: &AtomicBool,
) -> io::Result<Option<Content>> {
    check_cancelled(cancel)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if Identity::capture(&file.metadata()?) != expected {
        return Err(changed());
    }
    let expected_size = usize::try_from(expected.size).unwrap_or(MAX_TEXT_BYTES);
    let mut bytes = Vec::with_capacity(MAX_TEXT_BYTES.min(expected_size));
    file.by_ref()
        .take(MAX_TEXT_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    check_cancelled(cancel)?;
    if Identity::capture(&file.metadata()?) != expected || !expected.still_matches_path(path)? {
        return Err(changed());
    }
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    bytes.truncate(MAX_TEXT_BYTES);
    Ok(text_preview(&bytes).map(|text| Content::Text { text, truncated }))
}

fn text_preview(bytes: &[u8]) -> Option<String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(error) if error.error_len().is_none() && error.valid_up_to() != 0 => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .ok()?
                .to_string()
        }
        Err(_) => return None,
    };
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return None;
    }
    Some(text)
}

/// "Last modified on 23 Sep 2026 at 3:38:45 PM".
pub fn modified_line(time: SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = time.into();
    format!(
        "Last modified on {} at {}",
        local.format("%-d %b %Y"),
        local.format("%-I:%M:%S %p")
    )
}

fn bounded_display(value: &str, maximum: usize) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in value.chars().enumerate() {
        if index == maximum {
            truncated = true;
            break;
        }
        output.push(if character.is_control() {
            '\u{fffd}'
        } else {
            character
        });
    }
    if truncated {
        output.push('…');
    }
    output
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "the item changed while its preview was loading",
    )
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "preview cancelled")
}

pub(crate) fn check_cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

/// A private-safe sentence for a failed load.
pub fn error_message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "The item can’t be found.".to_owned(),
        io::ErrorKind::PermissionDenied => "You don’t have permission to see this item.".to_owned(),
        io::ErrorKind::WouldBlock => "The item changed while its preview was loading.".to_owned(),
        io::ErrorKind::Other => {
            // Preview's renderer already words its messages for people
            // (for example the missing poppler-utils notice).
            let text = error.to_string();
            if text.is_empty() {
                "No preview is available.".to_owned()
            } else {
                text
            }
        }
        _ => "No preview is available.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::UNIX_EPOCH;

    fn run(path: &Path) -> io::Result<Content> {
        let cancel = AtomicBool::new(false);
        load(path, (1148.0, 721.0), &cancel)
    }

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rmac-quick-look-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn text_preview_accepts_unicode_and_refuses_binary_controls() {
        assert_eq!(
            text_preview("Hello, 世界\n".as_bytes()).as_deref(),
            Some("Hello, 世界\n")
        );
        assert!(text_preview(b"hello\0world").is_none());
        assert!(text_preview(&[0xff, 0xfe]).is_none());
    }

    #[test]
    fn regular_text_is_bounded_and_reports_truncation() {
        let root = scratch("text");
        let path = root.join("large.txt");
        std::fs::write(&path, vec![b'a'; MAX_TEXT_BYTES + 16]).unwrap();
        assert!(matches!(
            run(&path).unwrap(),
            Content::Text {
                truncated: true,
                ..
            }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folders_summarise_size_and_count_like_the_mac() {
        let root = scratch("folder");
        let folder = root.join("Folder A");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "a").unwrap();
        std::fs::write(folder.join("b.txt"), "b").unwrap();
        let Content::Summary(summary) = run(&folder).unwrap() else {
            panic!("a folder is summarised");
        };
        assert!(summary.folder);
        assert_eq!(summary.detail, "2 bytes, 2 items");
        assert!(summary
            .modified
            .is_some_and(|line| line.starts_with("Last modified on ")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archives_are_summarised_for_uncompress() {
        let root = scratch("archive");
        let path = root.join("Bundle.zip");
        std::fs::write(&path, vec![0_u8; 364]).unwrap();
        let Content::Summary(summary) = run(&path).unwrap() else {
            panic!("an archive is summarised");
        };
        assert!(summary.archive);
        assert_eq!(summary.detail, "364 bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symbolic_link_is_described_without_following_its_target() {
        let root = scratch("link");
        std::fs::write(root.join("target.txt"), "private contents").unwrap();
        let link = root.join("link.txt");
        symlink("target.txt", &link).unwrap();
        let Content::Summary(summary) = run(&link).unwrap() else {
            panic!("a link is summarised");
        };
        assert_eq!(summary.detail, "Symbolic link to target.txt");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_wins_before_path_or_converter_access() {
        let cancel = AtomicBool::new(true);
        assert_eq!(
            load(Path::new("/missing/movie.mp4"), (800.0, 600.0), &cancel)
                .err()
                .map(|error| error.kind()),
            Some(io::ErrorKind::Interrupted)
        );
    }
}
