//! The in-memory cache of open locked-note keys for one Notes session.
//!
//! Unlocking a locked note with its password derives that password
//! generation's key once and keeps it here, so every note locked with the
//! same password opens without asking again (as on macOS). Application ▸
//! Close All Locked Notes, inactivity, sleep and the lock screen call
//! [`NotesKeyring::close_all`]; each key zeroizes its bytes when its last
//! holder drops it. Keys never leave memory.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use rmac_notes_store::LockKey;

#[derive(Clone, Default)]
pub struct NotesKeyring {
    keys: Arc<Mutex<BTreeMap<u32, Arc<LockKey>>>>,
}

impl NotesKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: LockKey) {
        self.insert_shared(Arc::new(key));
    }

    pub fn insert_shared(&self, key: Arc<LockKey>) {
        let replaced = self.lock().insert(key.key_id(), key);
        drop(replaced);
    }

    pub fn get(&self, key_id: u32) -> Option<Arc<LockKey>> {
        self.lock().get(&key_id).cloned()
    }

    pub fn contains(&self, key_id: u32) -> bool {
        self.lock().contains_key(&key_id)
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn open_key_ids(&self) -> BTreeSet<u32> {
        self.lock().keys().copied().collect()
    }

    pub fn remove(&self, key_id: u32) {
        let removed = self.lock().remove(&key_id);
        drop(removed);
    }

    /// Forget every open key. Returns whether any key was open.
    pub fn close_all(&self) -> bool {
        let closed = std::mem::take(&mut *self.lock());
        let any = !closed.is_empty();
        drop(closed);
        any
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u32, Arc<LockKey>>> {
        self.keys.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for NotesKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesKeyring")
            .field("open_keys", &self.lock().len())
            .finish()
    }
}
