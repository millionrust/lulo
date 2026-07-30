//! Private, bounded OSC 7 working-directory state for one terminal session.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use url::Url;

pub(crate) const MAX_URI_BYTES: usize = 768;
const ROOT_LABEL: &str = "/";

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryContext {
    label: String,
    local_path: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub(super) struct SessionDirectory {
    value: Arc<Mutex<Option<DirectoryContext>>>,
}

impl SessionDirectory {
    pub(super) fn from_local(path: &Path) -> Self {
        Self {
            value: Arc::new(Mutex::new(local_context(path))),
        }
    }

    /// Accept a complete OSC 7 URI and report whether presentation changed.
    ///
    /// Invalid values leave the last trusted state intact.
    pub(super) fn set_uri(&self, raw: &str) -> bool {
        let Some(next) = parse_uri(raw) else {
            return false;
        };
        let Ok(mut current) = self.value.lock() else {
            return false;
        };
        if current.as_ref() == Some(&next) {
            return false;
        }
        *current = Some(next);
        true
    }

    pub(super) fn label(&self) -> Option<String> {
        self.value
            .lock()
            .ok()?
            .as_ref()
            .map(|value| value.label.clone())
    }

    /// Return a local directory only while it still exists as a directory.
    pub(super) fn live_local_path(&self) -> Option<PathBuf> {
        let path = self.value.lock().ok()?.as_ref()?.local_path.clone()?;
        path.is_dir().then_some(path)
    }
}

fn parse_uri(raw: &str) -> Option<DirectoryContext> {
    if raw.is_empty() || raw.len() > MAX_URI_BYTES || raw.chars().any(is_control_or_directional) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "file"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    let host = url.host_str().unwrap_or_default();
    if !host.is_empty() && host != "localhost" {
        return Some(DirectoryContext {
            label: format!("Remote — {host}"),
            local_path: None,
        });
    }

    let path = url.to_file_path().ok()?;
    let decoded = path.to_string_lossy();
    if decoded.chars().any(is_control_or_directional) {
        return None;
    }
    local_context(&path)
}

fn local_context(path: &Path) -> Option<DirectoryContext> {
    let label = path
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| ROOT_LABEL.into());
    if label.chars().any(is_control_or_directional) {
        return None;
    }
    Some(DirectoryContext {
        label,
        local_path: Some(path.to_path_buf()),
    })
}

fn is_control_or_directional(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_osc_7_is_private_bounded_and_session_local() {
        let first = SessionDirectory::default();
        let first_reader = first.clone();
        let second = SessionDirectory::default();

        assert!(first_reader.set_uri("file:///home/jacob/Projects/rmac"));
        assert_eq!(first.label().as_deref(), Some("rmac"));
        assert_eq!(
            first.value.lock().unwrap().as_ref().unwrap().local_path,
            Some(PathBuf::from("/home/jacob/Projects/rmac"))
        );
        assert_eq!(second.label(), None);
        assert!(!first_reader.set_uri("file:///home/jacob/Projects/rmac"));
        assert!(!first_reader.set_uri(&format!("file:///{}", "a".repeat(MAX_URI_BYTES))));
        assert_eq!(first.label().as_deref(), Some("rmac"));
    }

    #[test]
    fn remote_context_is_visible_but_never_a_local_spawn_path() {
        let directory = SessionDirectory::default();
        assert!(directory.set_uri("file://build.example.test/home/jacob/private"));
        assert_eq!(
            directory.label().as_deref(),
            Some("Remote — build.example.test")
        );
        assert_eq!(directory.live_local_path(), None);
    }

    #[test]
    fn rejects_spoofing_credentials_queries_and_non_file_schemes() {
        let directory = SessionDirectory::default();
        for uri in [
            "https://example.test/home/jacob",
            "file://user:password@localhost/home/jacob",
            "file:///home/jacob?token=secret",
            "file:///home/jacob#fragment",
            "file:///home/jacob/%0Aspoof",
            "file:///home/jacob/\u{202e}txt.exe",
        ] {
            assert!(!directory.set_uri(uri), "{uri}");
        }
        assert_eq!(directory.label(), None);
    }
}
