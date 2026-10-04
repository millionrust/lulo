//! Bounded, least-recently-used eviction for Finder's in-memory caches.
//!
//! `FinderView` keeps a few caches that are keyed by filesystem path and grow
//! for as long as a window stays open: decoded directory listings for
//! expanded rows in List view (`child_entries`) and resolved thumbnail paths
//! (`thumbs`). Neither cache was ever pruned except on a full directory
//! reload, so expanding and collapsing many folders over a long session (see
//! `docs/parity.md` MEM-03) grew them without bound. These helpers cap each
//! cache at a fixed number of entries appropriate for a 4 GiB machine,
//! evicting the least-recently-touched key first, while never evicting a key
//! that is still visible (`protect` returns `true` for it).
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// Cap for `child_entries`: decoded directory listings for folders expanded
/// in List view. Each entry carries several formatted-date `SharedString`s,
/// so a folder of a few hundred items can cost tens of kilobytes; 128
/// distinct folders is generous for how many a person keeps expanded at
/// once while staying well under a megabyte on a 4 GiB machine.
pub(super) const CHILD_ENTRIES_CACHE_CAP: usize = 128;

/// Cap for `thumbs`: resolved thumbnail file paths (not pixel data, which
/// lives in the on-disk `rmac_thumbnails` cache). Each entry is two
/// `PathBuf`s, so even a few thousand cost well under a megabyte; the cap
/// exists so a long session browsing many folders cannot grow this map
/// forever.
pub(super) const THUMBNAIL_CACHE_CAP: usize = 2000;

/// Record that `key` was just inserted or read, moving it to the
/// most-recently-used end of `order`.
pub(super) fn touch_cache_key<K>(order: &mut VecDeque<K>, key: &K)
where
    K: Eq + Clone,
{
    if let Some(position) = order.iter().position(|existing| existing == key) {
        order.remove(position);
    }
    order.push_back(key.clone());
}

/// Trim `map` down to at most `cap` entries, evicting the least-recently-used
/// keys in `order` first. A key for which `protect` returns `true` (still
/// visible on screen) is never evicted, even past the cap: the cap bounds
/// stale cache growth, not the working set actually on screen.
pub(super) fn bound_cache<K, V>(
    order: &mut VecDeque<K>,
    map: &mut HashMap<K, V>,
    cap: usize,
    protect: impl Fn(&K) -> bool,
) where
    K: Eq + Hash + Clone,
{
    let mut requeued = 0usize;
    while map.len() > cap {
        let Some(candidate) = order.pop_front() else {
            break;
        };
        if protect(&candidate) {
            order.push_back(candidate);
            requeued += 1;
            // Every key still in `order` has been re-examined once; the rest
            // of the overflow is protected, so stop instead of spinning.
            if requeued >= order.len() {
                break;
            }
            continue;
        }
        map.remove(&candidate);
        requeued = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf {
        PathBuf::from("/tmp").join(name)
    }

    #[test]
    fn eviction_drops_the_least_recently_used_key_first() {
        let mut order = VecDeque::new();
        let mut map = HashMap::new();
        for name in ["a", "b", "c"] {
            map.insert(path(name), ());
            touch_cache_key(&mut order, &path(name));
        }

        bound_cache(&mut order, &mut map, 2, |_| false);

        assert_eq!(map.len(), 2);
        assert!(!map.contains_key(&path("a")), "oldest key must be evicted");
        assert!(map.contains_key(&path("b")));
        assert!(map.contains_key(&path("c")));
    }

    #[test]
    fn touching_a_key_again_protects_it_from_the_next_eviction() {
        let mut order = VecDeque::new();
        let mut map = HashMap::new();
        for name in ["a", "b", "c"] {
            map.insert(path(name), ());
            touch_cache_key(&mut order, &path(name));
        }
        // Re-read "a": it is now the most recently used, so "b" is next in
        // line to go.
        touch_cache_key(&mut order, &path("a"));

        bound_cache(&mut order, &mut map, 2, |_| false);

        assert!(map.contains_key(&path("a")), "re-touched key must survive");
        assert!(
            !map.contains_key(&path("b")),
            "untouched key must be evicted"
        );
        assert!(map.contains_key(&path("c")));
    }

    #[test]
    fn a_still_visible_key_is_never_evicted_even_past_the_cap() {
        let mut order = VecDeque::new();
        let mut map = HashMap::new();
        for name in ["a", "b", "c"] {
            map.insert(path(name), ());
            touch_cache_key(&mut order, &path(name));
        }

        // "a" is the oldest, but it is still expanded/visible, so it must
        // stay even though the map is over its cap of 1.
        bound_cache(&mut order, &mut map, 1, |key| *key == path("a"));

        assert!(map.contains_key(&path("a")), "visible key must survive");
        assert_eq!(map.len(), 1, "every evictable key over the cap must go");
    }

    #[test]
    fn a_cache_within_its_cap_is_left_untouched() {
        let mut order = VecDeque::new();
        let mut map = HashMap::new();
        map.insert(path("a"), ());
        touch_cache_key(&mut order, &path("a"));

        bound_cache(&mut order, &mut map, 128, |_| false);

        assert_eq!(map.len(), 1);
        assert_eq!(order.len(), 1);
    }
}
