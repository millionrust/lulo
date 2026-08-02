use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::decode::io_error;
use crate::{
    decode_file, CacheStats, DecodeRequest, DecodedIcon, Error, ErrorKind,
    DEFAULT_ICON_CACHE_BYTES, MAX_CACHED_ICONS,
};

#[derive(Clone, Eq, Hash, PartialEq)]
struct FileIdentity {
    byte_len: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    modified_seconds: i64,
    #[cfg(unix)]
    modified_nanoseconds: i64,
    #[cfg(not(unix))]
    modified_nanoseconds: Option<u128>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct CacheKey {
    path: PathBuf,
    identity: FileIdentity,
    edge: u32,
}

struct CacheEntry {
    icon: Arc<DecodedIcon>,
    last_used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<CacheKey, CacheEntry>,
    sequence: u64,
    bytes: usize,
    decodes: u64,
}

/// Thread-safe, byte- and entry-bounded LRU cache for worker-side decoding.
///
/// File identity is checked before and after every miss. Decodes are serialized
/// so even accidental concurrent callers cannot multiply the decoder's bounded
/// allocation. Paths are retained only inside the bounded private key set.
pub struct Cache {
    state: Mutex<CacheState>,
    decode_lock: Mutex<()>,
    byte_budget: usize,
}

impl Cache {
    pub fn new(byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(CacheState::default()),
            decode_lock: Mutex::new(()),
            byte_budget,
        }
    }

    pub fn get_or_decode(
        &self,
        path: &Path,
        request: DecodeRequest,
    ) -> Result<Arc<DecodedIcon>, Error> {
        let _decoder = self
            .decode_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = file_identity(path)?;
        let key = CacheKey {
            path: path.to_path_buf(),
            identity: before.clone(),
            edge: request.edge,
        };
        if let Some(icon) = self.lookup(&key) {
            return Ok(icon);
        }

        let decoded = Arc::new(decode_file(path, request)?);
        if file_identity(path)? != before {
            return Err(Error::new(ErrorKind::Changed));
        }
        let bytes = decoded.rgba.len();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.decodes = state.decodes.saturating_add(1);
        state
            .entries
            .retain(|candidate, _| candidate.path != path || candidate.identity == before);
        state.bytes = state
            .entries
            .values()
            .map(|entry| entry.icon.rgba.len())
            .sum();
        if self.byte_budget == 0 || bytes > self.byte_budget {
            return Ok(decoded);
        }
        state.sequence = state.sequence.saturating_add(1);
        let last_used = state.sequence;
        if let Some(previous) = state.entries.insert(
            key,
            CacheEntry {
                icon: decoded.clone(),
                last_used,
            },
        ) {
            state.bytes = state.bytes.saturating_sub(previous.icon.rgba.len());
        }
        state.bytes = state.bytes.saturating_add(bytes);
        while state.bytes > self.byte_budget || state.entries.len() > MAX_CACHED_ICONS {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = state.entries.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(removed.icon.rgba.len());
            }
        }
        Ok(decoded)
    }

    /// Remove every rendered size for one private source path.
    pub fn invalidate_path(&self, path: &Path) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.retain(|key, _| key.path != path);
        state.bytes = state
            .entries
            .values()
            .map(|entry| entry.icon.rgba.len())
            .sum();
    }

    pub fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.clear();
        state.bytes = 0;
    }

    pub fn stats(&self) -> CacheStats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            decodes: state.decodes,
        }
    }

    fn lookup(&self, key: &CacheKey) -> Option<Arc<DecodedIcon>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        let last_used = state.sequence;
        let entry = state.entries.get_mut(key)?;
        entry.last_used = last_used;
        Some(entry.icon.clone())
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(DEFAULT_ICON_CACHE_BYTES)
    }
}

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("byte_budget", &self.byte_budget)
            .field("stats", &self.stats())
            .finish()
    }
}

fn file_identity(path: &Path) -> Result<FileIdentity, Error> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(Error::new(ErrorKind::Unsupported));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        Ok(FileIdentity {
            byte_len: metadata.len(),
            device: metadata.dev(),
            inode: metadata.ino(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        })
    }
    #[cfg(not(unix))]
    {
        use std::time::UNIX_EPOCH;

        Ok(FileIdentity {
            byte_len: metadata.len(),
            modified_nanoseconds: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos()),
        })
    }
}
