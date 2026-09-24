//! Which of a process's documents are unsaved, reduced to the one fact the
//! session needs: does this process hold unsaved work at all.

use std::collections::HashSet;
use std::hash::Hash;

pub(crate) struct UnsavedSet<K> {
    holders: HashSet<K>,
}

impl<K> Default for UnsavedSet<K> {
    fn default() -> Self {
        Self {
            holders: HashSet::new(),
        }
    }
}

impl<K: Hash + Eq> UnsavedSet<K> {
    /// Record whether `key` is unsaved. Returns the process's new state when
    /// it changes, `None` when it stays the same.
    pub(crate) fn set(&mut self, key: K, unsaved: bool) -> Option<bool> {
        let before = !self.holders.is_empty();
        if unsaved {
            self.holders.insert(key);
        } else {
            self.holders.remove(&key);
        }
        let after = !self.holders.is_empty();
        (before != after).then_some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_unsaved_and_the_last_saved_document_change_the_process() {
        let mut set = UnsavedSet::default();
        assert_eq!(set.set(1, false), None);
        assert_eq!(set.set(1, true), Some(true));
        assert_eq!(set.set(1, true), None);
        assert_eq!(set.set(2, true), None);
        assert_eq!(set.set(1, false), None);
        assert_eq!(set.set(2, false), Some(false));
        assert_eq!(set.set(2, false), None);
    }
}
