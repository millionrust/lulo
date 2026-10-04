use super::*;

/// View ▸ Clean Up / Clean Up By / Clean Up Selection.
///
/// Lulo's Icon view always auto-flows icons onto the grid (there is no
/// free-form icon placement to tidy, unlike the Mac), so "Clean Up" and
/// "Sort By" would be the exact same operation if Clean Up just set the
/// ongoing sort key. The real distinction the Mac draws — Clean Up is a
/// one-off tidy that does not change the window's standing sort order — is
/// kept here: these commands reorder `root_entries` (and so what's on
/// screen) without touching `self.sort_key`, the Sort By menu's checkmark,
/// or the persisted folder option. The next directory reload (navigating
/// away and back, or a filesystem change) re-fetches and re-sorts by the
/// standing `sort_key`, exactly as a Mac window reverts once its icons are
/// moved again.
impl FinderView {
    pub(super) fn clean_up_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.view != ViewMode::Icon {
            return;
        }
        sort_entries(&mut self.root_entries, key, self.sort_asc);
        self.rebuild_list_entries();
        cx.notify();
    }

    pub(super) fn clean_up(&mut self, cx: &mut Context<Self>) {
        self.clean_up_by(self.sort_key, cx);
    }

    /// Tidy only the selected icons into sorted order, leaving every other
    /// icon's position alone — the Mac's "Clean Up Selection".
    pub(super) fn clean_up_selection(&mut self, cx: &mut Context<Self>) {
        if self.view != ViewMode::Icon || self.selected.is_empty() {
            return;
        }
        let key = self.sort_key;
        let asc = self.sort_asc;
        let positions: Vec<usize> = self
            .selected
            .iter()
            .copied()
            .filter(|&index| index < self.root_entries.len())
            .collect();
        if positions.len() < 2 {
            return;
        }
        let mut subset: Vec<Entry> = positions
            .iter()
            .map(|&index| self.root_entries[index].clone())
            .collect();
        sort_entries(&mut subset, key, asc);
        for (slot, entry) in positions.into_iter().zip(subset) {
            self.root_entries[slot] = entry;
        }
        self.rebuild_list_entries();
        cx.notify();
    }
}
