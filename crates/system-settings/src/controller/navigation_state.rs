//! Category selection and detail back-stack state.

use super::*;

/// Bring focus onto `content_focus`'s pane if it currently isn't there.
///
/// Deliberately two frames, not one, the same way `rmac_ui`'s own dialog
/// focus trap is: `window.focus_next` reads the *last painted* frame's tab
/// stops, which on the frame a pane's content first changes don't yet
/// reflect it — only `window.focus(content_focus, ...)` is safe to call
/// before that content has ever been painted. So the first frame's focus
/// lands on `content_focus` itself (the detail column's own boundary, a
/// legitimate landing spot), and once that has been painted at least once,
/// the next frame steps from it onto the pane's real first control.
fn enter_content_focus(content_focus: &FocusHandle, window: &mut Window, cx: &mut App) {
    if content_focus.is_focused(window) {
        window.focus_next(cx);
        if !content_focus.contains_focused(window, cx) {
            window.focus(content_focus, cx);
        }
        return;
    }
    if !content_focus.contains_focused(window, cx) {
        window.focus(content_focus, cx);
    }
}

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

    /// The top-level categories the sidebar shows when not searching, in
    /// the same section/row order `render_sidebar` paints them
    /// (`chrome.rs`'s `matching` filter: `category_parent(..).is_none()`).
    pub(super) fn visible_sidebar_positions(&self) -> Vec<(usize, usize)> {
        self.sections
            .iter()
            .enumerate()
            .flat_map(|(section_index, section)| {
                section
                    .iter()
                    .enumerate()
                    .filter(|(_, category)| category_parent(category.name.as_ref()).is_none())
                    .map(move |(category_index, _)| (section_index, category_index))
            })
            .collect()
    }

    /// Up/Down while the sidebar list itself holds keyboard focus: move the
    /// highlighted row to the next or previous top-level category, stopping
    /// at both ends (macOS's own sidebar list does not wrap).
    pub(super) fn move_category_selection(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let positions = self.visible_sidebar_positions();
        if positions.is_empty() {
            return false;
        }
        let current = self.current().name.clone();
        let owner = category_parent(current.as_ref())
            .map(SharedString::from)
            .unwrap_or(current);
        let current_index = positions
            .iter()
            .position(|&(section, item)| {
                self.sections
                    .get(section)
                    .and_then(|items| items.get(item))
                    .is_some_and(|category| category.name == owner)
            })
            .unwrap_or(0);
        let next_index = current_index
            .saturating_add_signed(delta)
            .min(positions.len() - 1);
        self.sidebar_focused = true;
        // Not `select_position`: Up/Down here browses the highlight while
        // the sidebar list itself keeps keyboard focus (so the next
        // arrow-press keeps working), unlike actually choosing a category.
        self.select_position_keeping_focus(positions[next_index], cx);
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
        self.select_position(target, window, cx);
        self.clear_search(window, cx);
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

    pub(super) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.nav.pop() {
            self.forward.push(page);
        } else if let Some(parent) = category_parent(self.current().name.as_ref()) {
            self.select_category(parent, window, cx);
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
        let measure_storage = matches!(sub, SubPage::Storage);
        // The Mac checks for updates each time Software Update opens.
        let check_updates = matches!(sub, SubPage::SoftwareUpdate);
        self.nav.push(sub);
        self.forward.clear();
        if measure_storage {
            self.measure_storage_categories(cx);
        }
        if check_updates {
            self.refresh_update_status(cx);
        }
        self.sidebar_focused = false;
        cx.notify();
    }

    /// Navigate this already-open window to `pane_id` — the same route a
    /// fresh `--pane <id>` launch selects at startup
    /// (`initial_navigation`) — for a second launch's request handed off
    /// instead of starting another process (SET-57).
    pub(super) fn navigate_to_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested_subpage = subpage_route(pane_id);
        let Some(category) = category_name_for_pane_id(pane_id)
            .or_else(|| requested_subpage.as_ref().map(|(category, _)| *category))
        else {
            return;
        };
        self.select_category(category, window, cx);
        if let Some((subpage_category, page)) = requested_subpage {
            if self.current().name == subpage_category {
                self.nav = vec![page];
                self.forward.clear();
            }
        }
        cx.notify();
    }

    pub(super) fn select_category(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            self.select_position(target, window, cx);
        }
    }

    /// Choose a category, the way clicking or Return-activating a sidebar
    /// row, a search result, or an in-pane "jump to Keyboard…"-style link
    /// does: moves focus into the new pane's content
    /// (`content_focus`/`enter_content_focus`), same as macOS's own sidebar.
    /// Up/Down browsing the sidebar's own highlight uses
    /// [`Self::select_position_keeping_focus`] instead, so it doesn't kick
    /// focus out of the list mid-arrow-press.
    pub(super) fn select_position(
        &mut self,
        target: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.select_position_keeping_focus(target, cx) {
            return;
        }
        self.sidebar_focused = false;
        enter_content_focus(&self.content_focus, window, cx);
    }

    /// The state change behind [`Self::select_position`], without moving
    /// focus. Returns whether `target` was a real, selectable category.
    fn select_position_keeping_focus(
        &mut self,
        target: (usize, usize),
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(category) = self
            .sections
            .get(target.0)
            .and_then(|section| section.get(target.1))
        else {
            return false;
        };
        let Some(pane_id) = pane_id_for_category_name(category.name.as_ref()) else {
            return false;
        };
        self.selected = target;
        self.nav.clear();
        self.forward.clear();
        self.compact_sidebar_open = false;
        self.navigation_persistence.schedule(pane_id);
        cx.notify();
        true
    }
}
