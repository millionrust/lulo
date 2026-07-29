use std::fs::OpenOptions;
use std::io::{self, Read as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_DIRECTORY_ITEMS: usize = 10_000;
const MAX_LINK_CHARACTERS: usize = 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Image { preview: PathBuf },
    Text { text: String, truncated: bool },
    Folder { items: usize, truncated: bool },
    Link { target: String },
    Unsupported,
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

pub fn load(path: &Path) -> io::Result<Content> {
    let metadata = std::fs::symlink_metadata(path)?;
    let identity = Identity::capture(&metadata);
    if metadata.is_dir() {
        return load_folder(path, identity);
    }
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        if !identity.still_matches_path(path)? {
            return Err(changed());
        }
        return Ok(Content::Link {
            target: bounded_display(&target.to_string_lossy(), MAX_LINK_CHARACTERS),
        });
    }
    if !metadata.is_file() {
        return Ok(Content::Unsupported);
    }
    if rmac_thumbnails::is_supported(path) {
        let preview = rmac_thumbnails::generate_preview(path).map_err(io::Error::other)?;
        if !identity.still_matches_path(path)? {
            return Err(changed());
        }
        return Ok(Content::Image { preview });
    }

    load_regular_text(path, identity)
}

fn load_folder(path: &Path, identity: Identity) -> io::Result<Content> {
    let mut items = 0usize;
    let mut truncated = false;
    for entry in std::fs::read_dir(path)? {
        entry?;
        if items == MAX_DIRECTORY_ITEMS {
            truncated = true;
            break;
        }
        items += 1;
    }
    if !identity.still_matches_path(path)? {
        return Err(changed());
    }
    Ok(Content::Folder { items, truncated })
}

fn load_regular_text(path: &Path, expected: Identity) -> io::Result<Content> {
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
    if Identity::capture(&file.metadata()?) != expected || !expected.still_matches_path(path)? {
        return Err(changed());
    }
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    bytes.truncate(MAX_TEXT_BYTES);
    let Some(text) = text_preview(&bytes) else {
        return Ok(Content::Unsupported);
    };
    Ok(Content::Text { text, truncated })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let root = temporary_directory("text");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("large.txt");
        std::fs::write(&path, vec![b'a'; MAX_TEXT_BYTES + 16]).unwrap();

        assert!(matches!(
            load(&path).unwrap(),
            Content::Text {
                truncated: true,
                ..
            }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symbolic_link_is_described_without_following_its_target() {
        let root = temporary_directory("link");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target.txt");
        std::fs::write(&target, "private contents").unwrap();
        let link = root.join("link.txt");
        symlink("target.txt", &link).unwrap();

        assert_eq!(
            load(&link).unwrap(),
            Content::Link {
                target: "target.txt".into()
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-quick-look-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
