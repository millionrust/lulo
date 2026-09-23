//! Category selection and detail back-stack state.

use super::*;

impl Settings {
    pub(super) fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    pub(super) fn application_identity(&self, app_id: &str) -> Option<&rmac_apps::Application> {
        rmac_apps::find_desktop_entry(&self.app_catalog, app_id)
    }

    pub(super) fn search_matches(&self, cx: &Context<Self>) -> Vec<(usize, usize)> {
        let query = self.search.read(cx).value();
        self.sections
            .iter()
            .enumerate()
            .flat_map(|(section_index, section)| {
                section
                    .iter()
                    .enumerate()
                    .filter(|(_, category)| crate::settings_search::matches(category, &query))
                    .map(move |(category_index, _)| (section_index, category_index))
            })
            .collect()
    }

    pub(super) fn move_search_selection(&mut self, delta: isize, cx: &Context<Self>) -> bool {
        let count = self.search_matches(cx).len();
        if count == 0 {
            return false;
        }
        self.search_selection = self
            .search_selection
            .saturating_add_signed(delta)
            .min(count - 1);
        true
    }

    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.search_selection = 0;
    }

    pub(super) fn toggle_compact_sidebar(&mut self, cx: &mut Context<Self>) {
        self.compact_sidebar_open = !self.compact_sidebar_open;
        cx.notify();
    }

    pub(super) fn activate_search_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let matches = self.search_matches(cx);
        let Some(target) = matches
            .get(self.search_selection.min(matches.len().saturating_sub(1)))
            .copied()
        else {
            return false;
        };
        self.select_position(target, cx);
        self.clear_search(window, cx);
        window.focus(&self.focus, cx);
        true
    }

    /// Back pops a subpage, or leaves a pane that macOS files under General
    /// (Date & Time, Sharing, …) for General itself.
    pub(super) fn can_go_back(&self) -> bool {
        !self.nav.is_empty() || category_parent(self.current().name.as_ref()).is_some()
    }

    pub(super) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.nav.pop() {
            self.forward.push(page);
        } else if let Some(parent) = category_parent(self.current().name.as_ref()) {
            self.select_category(parent, cx);
        }
        cx.notify();
    }

    pub(super) fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.forward.pop() {
            if self.nav.len() < rmac_system_settings::accessibility::MAX_NAVIGATION_DEPTH {
                self.nav.push(page);
            }
        }
        cx.notify();
    }

    pub(super) fn push(&mut self, sub: SubPage, cx: &mut Context<Self>) {
        if self.nav.len() >= rmac_system_settings::accessibility::MAX_NAVIGATION_DEPTH {
            return;
        }
        self.nav.push(sub);
        self.forward.clear();
        self.sidebar_focused = false;
        cx.notify();
    }

    pub(super) fn select_category(&mut self, name: &str, cx: &mut Context<Self>) {
        let target = self
            .sections
            .iter()
            .enumerate()
            .find_map(|(section, items)| {
                items
                    .iter()
                    .position(|category| category.name.as_ref() == name)
                    .map(|item| (section, item))
            });
        if let Some(target) = target {
            self.select_position(target, cx);
        }
    }

    pub(super) fn select_position(&mut self, target: (usize, usize), cx: &mut Context<Self>) {
        let Some(category) = self
            .sections
            .get(target.0)
            .and_then(|section| section.get(target.1))
        else {
            return;
        };
        let Some(pane_id) = pane_id_for_category_name(category.name.as_ref()) else {
            return;
        };
        self.selected = target;
        self.nav.clear();
        self.forward.clear();
        self.compact_sidebar_open = false;
        self.navigation_persistence.schedule(pane_id);
        cx.notify();
    }
}
