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
    pub(super) fn pane_available(&self, name: &str) -> bool {
        !self.hardware_ready || self.hardware.allows_pane(name)
    }

    pub(super) fn apply_hardware(&mut self, hardware: Capabilities, cx: &mut Context<Self>) {
        // Only the first scan (startup, or the initial `--pane` launch
        // argument) can redirect away from an unavailable pane. A later
        // udev-triggered rescan only updates the rows/gating *within* the
        // pane the person is already looking at; it must never evict them
        // to General just because one rescan raced a device that was still
        // settling (macOS never does this to an open pane either).
        let was_ready = self.hardware_ready;
        let available_before = self.available_panes();
        self.hardware = hardware;
        self.hardware_ready = true;
        let mut redirected = false;
        if !was_ready && !self.pane_available(self.current().name.as_ref()) {
            if let Some(position) = category_position(&self.sections, "General") {
                self.selected = position;
                self.nav.clear();
                self.forward.clear();
                self.pane_history.clear();
                self.pane_forward.clear();
                redirected = true;
            }
        }
        self.cancel_storage_scan_if_hidden();
        if redirected || self.available_panes() != available_before {
            // The sidebar's rows (and the menu's Show items) changed.
            cx.notify();
        } else {
            self.notify_if_showing(
                &[
                    "Displays",
                    "Keyboard",
                    "Mouse",
                    "Trackpad",
                    "Touchscreen",
                    "Lock Screen",
                    "Menu Bar",
                ],
                cx,
            );
        }
    }

    /// Which categories the sidebar lists, in order.
    fn available_panes(&self) -> Vec<bool> {
        self.sections
            .iter()
            .flatten()
            .map(|category| self.pane_available(category.name.as_ref()))
            .collect()
    }

    pub(super) fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    /// Watchers keep their cached data current for hidden panes. Only a pane
    /// showing that data needs a new frame when its stream reports an update.
    pub(super) fn notify_if_current_pane(&self, panes: &[&str], cx: &mut Context<Self>) {
        if panes.contains(&self.current().name.as_ref()) {
            self.notify_pane(cx);
        }
    }

    /// The window's master/detail split for its current width.
    pub(super) fn layout(&self, window: &Window) -> crate::responsive_layout::SettingsLayout {
        crate::responsive_layout::responsive_layout(
            f32::from(window.bounds().size.width),
            self.compact_sidebar_open,
        )
    }

    /// Repaint the detail pane, and the root shell around it (toolbar,
    /// banner, sheets), but not the sidebar: for data no sidebar row reads.
    pub(super) fn notify_pane(&self, cx: &mut Context<Self>) {
        match &self.views {
            Some(views) => views.pane.update(cx, |_, cx| cx.notify()),
            None => cx.notify(),
        }
    }

    /// Repaint for a background load only when the window can be showing
    /// what it loaded: one of `panes` is open, or any subpage (they show
    /// details of many sources), or a search; or the window-wide error
    /// banner has to appear, change or go. Only the pane repaints (the
    /// sidebar reads no loaded data). A hidden pane reads the latest values
    /// when it is opened, since navigation repaints the whole window. At
    /// launch this keeps a dozen snapshot loads landing over ~350 ms from
    /// repainting the whole window for General (SPEED-02).
    pub(super) fn notify_if_showing(&self, panes: &[&str], cx: &mut Context<Self>) {
        if self.views.is_none()
            || pane_shows_load(
                self.current().name.as_ref(),
                panes,
                !self.nav.is_empty(),
                !self.search.read(cx).value().trim().is_empty(),
            )
            || self.global_settings_error() != self.rendered_banner.as_ref()
        {
            self.notify_pane(cx);
        }
    }

    pub(super) fn application_identity(&self, app_id: &str) -> Option<&rmac_apps::Application> {
        rmac_apps::find_desktop_entry(&self.app_catalog, app_id)
    }

    pub(super) fn catalog_pane_visible(&self) -> bool {
        matches!(
            self.current().name.as_ref(),
            "Notifications" | "Focus" | "Privacy & Security" | "Accessibility"
        )
    }

    pub(super) fn sync_catalog_for_pane(&mut self, was_visible: bool) {
        if self.catalog_pane_visible() {
            if !was_visible {
                let _ = self.catalog_reload.try_send(());
            }
        } else {
            self.app_catalog = Vec::new();
            self._app_catalog_watcher = None;
        }
    }

    /// Search results, ranked the way the Mac ranks them: a hit on a pane's
    /// own name (e.g. "Wallpaper" for "wallpaper") outranks one that only
    /// hit its description or hidden search vocabulary (e.g. "Desktop &
    /// Dock", whose search terms happen to mention "wallpaper" in
    /// passing), within the same section; sections stay in their declared
    /// order and categories keep their declared order within a rank tie.
    pub(super) fn search_matches(&self, cx: &Context<Self>) -> Vec<(usize, usize)> {
        let query = self.search.read(cx).value();
        let mut ranked: Vec<(u8, usize, usize)> = self
            .sections
            .iter()
            .enumerate()
            .flat_map(|(section_index, section)| {
                let query = &query;
                section
                    .iter()
                    .enumerate()
                    .filter_map(move |(category_index, category)| {
                        self.pane_available(category.name.as_ref())
                            .then(|| crate::settings_search::match_rank(category, query))
                            .flatten()
                            .map(|rank| (rank, section_index, category_index))
                    })
            })
            .collect();
        ranked.sort_by_key(|&(rank, section_index, category_index)| {
            (section_index, rank, category_index)
        });
        ranked
            .into_iter()
            .map(|(_, section_index, category_index)| (section_index, category_index))
            .collect()
    }

    pub(super) fn move_search_selection(&mut self, delta: isize, cx: &Context<Self>) -> bool {
        let count = self.search_matches(cx).len();
        if count == 0 {
            return false;
        }
        if !self.search_result_focused {
            // Nothing is highlighted yet (the Mac's own search results
            // start this way too): the first Down/Up lands on the current
            // value (0, the top-ranked result) instead of skipping past it.
            self.search_result_focused = true;
            self.search_selection = self.search_selection.min(count - 1);
            return true;
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
                    .filter(|(_, category)| {
                        category_parent(category.name.as_ref()).is_none()
                            && self.pane_available(category.name.as_ref())
                    })
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
        self.search_result_focused = false;
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

    /// Back pops a subpage, leaves a pane that macOS files under General
    /// (Date & Time, Sharing, …) for General itself, or (SET-02) returns to
    /// the top-level category left behind by the last committed navigation
    /// — a sidebar click, a search result, or an in-pane link — the Mac's
    /// own "Wi-Fi → Sound → Back returns to Wi-Fi".
    pub(super) fn can_go_back(&self) -> bool {
        !self.nav.is_empty()
            || category_parent(self.current().name.as_ref()).is_some()
            || !self.pane_history.is_empty()
    }

    pub(super) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty() || !self.pane_forward.is_empty()
    }

    pub(super) fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.nav.pop() {
            self.forward.push(page);
        } else if let Some(parent) = category_parent(self.current().name.as_ref()) {
            // Going up to General is itself an undo, not a new committed
            // navigation, so it does not disturb `pane_history`.
            if let Some(target) = self.position_for_category(parent) {
                self.navigate_to_position(target, window, cx);
            }
        } else if let Some(previous) = self.pane_history.pop() {
            let current = self.selected;
            self.navigate_to_position(previous, window, cx);
            self.pane_forward.push(current);
        }
        self.sync_wifi_pane_scan_on_navigation(cx);
        self.cancel_storage_scan_if_hidden();
        cx.notify();
    }

    pub(super) fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.forward.pop() {
            if self.nav.len() < rmac_system_settings::accessibility::MAX_NAVIGATION_DEPTH {
                self.nav.push(page);
            }
        } else if let Some(next) = self.pane_forward.pop() {
            let current = self.selected;
            self.navigate_to_position(next, window, cx);
            self.pane_history.push(current);
        }
        self.sync_wifi_pane_scan_on_navigation(cx);
        self.cancel_storage_scan_if_hidden();
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
            self.measure_storage_categories(false, cx);
        }
        if check_updates {
            self.refresh_update_status(cx);
        }
        self.sidebar_focused = false;
        self.sync_wifi_pane_scan_on_navigation(cx);
        self.cancel_storage_scan_if_hidden();
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
                let measure_storage = matches!(page, SubPage::Storage);
                self.nav = vec![page];
                self.forward.clear();
                // `push()` kicks off the Storage scan when it lands on the
                // pane; this deep-link/second-launch path (SET-57's
                // `NavigateToPane`) skipped the same call, so Storage never
                // measured and stuck on "0.0 GB everywhere in System Data"
                // when reached this way.
                if measure_storage {
                    self.measure_storage_categories(false, cx);
                }
            }
        }
        self.sync_wifi_pane_scan_on_navigation(cx);
        self.cancel_storage_scan_if_hidden();
        cx.notify();
    }

    pub(super) fn select_category(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.position_for_category(name) {
            self.select_position(target, window, cx);
        }
    }

    /// The `(section, item)` position of the category named `name`, if any.
    pub(super) fn position_for_category(&self, name: &str) -> Option<(usize, usize)> {
        self.sections
            .iter()
            .enumerate()
            .find_map(|(section, items)| {
                items
                    .iter()
                    .position(|category| {
                        category.name.as_ref() == name && self.pane_available(name)
                    })
                    .map(|item| (section, item))
            })
    }

    /// Choose a category, the way clicking or Return-activating a sidebar
    /// row, a search result, or an in-pane "jump to Keyboard…"-style link
    /// does: moves focus into the new pane's content
    /// (`content_focus`/`enter_content_focus`), same as macOS's own sidebar,
    /// and — unlike [`Self::navigate_to_position`] — records `target.0`'s
    /// predecessor in `pane_history` so Back (SET-02) can return to it. Up/
    /// Down browsing the sidebar's own highlight uses
    /// [`Self::select_position_keeping_focus`] instead, so it doesn't kick
    /// focus out of the list mid-arrow-press, and doesn't record history.
    pub(super) fn select_position(
        &mut self,
        target: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.selected;
        if !self.select_position_keeping_focus(target, cx) {
            return;
        }
        if target != previous {
            self.pane_history.push(previous);
            self.pane_forward.clear();
        }
        self.sidebar_focused = false;
        if self.current().name.as_ref() == "General" {
            window.focus(&self.general_focus, cx);
        } else {
            enter_content_focus(&self.content_focus, window, cx);
        }
    }

    /// The state change and focus move behind Back/Forward crossing panes
    /// (`go_back`/`go_forward`): like [`Self::select_position`], but an
    /// undo/redo of `pane_history`/`pane_forward` itself, so it doesn't
    /// record a fresh history entry (the caller does, if it should).
    fn navigate_to_position(
        &mut self,
        target: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.select_position_keeping_focus(target, cx) {
            return;
        }
        self.sidebar_focused = false;
        if self.current().name.as_ref() == "General" {
            window.focus(&self.general_focus, cx);
        } else {
            enter_content_focus(&self.content_focus, window, cx);
        }
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
        if !self.pane_available(category.name.as_ref()) {
            return false;
        }
        let Some(pane_id) = pane_id_for_category_name(category.name.as_ref()) else {
            return false;
        };
        let catalog_was_visible = self.catalog_pane_visible();
        let changed_pane = self.selected != target;
        self.selected = target;
        self.nav.clear();
        self.forward.clear();
        self.sync_catalog_for_pane(catalog_was_visible);
        self.refresh_wallpaper_preview(cx);
        self.compact_sidebar_open = false;
        self.navigation_persistence.schedule(pane_id);
        self.sync_wifi_pane_scan_on_navigation(cx);
        if changed_pane {
            // Stream updates for hidden hardware/network panes skip costly
            // snapshots. Read once on entry so their first visible frame
            // catches up with changes made elsewhere while hidden.
            let pane = self.current().name.to_string();
            match pane.as_str() {
                "Wi-Fi" => self.refresh_wifi_state(cx),
                "Network" => self.refresh_network(cx),
                "Internet Accounts" => self.refresh_internet_accounts(cx),
                "Users & Groups" | "Login Password" => self.refresh_users(cx),
                "Printers & Scanners" => self.refresh_printers(cx),
                "VPN" => self.refresh_vpn(cx),
                "Battery" => self.refresh_power(cx),
                _ => {}
            }
        }
        self.cancel_storage_scan_if_hidden();
        cx.notify();
        true
    }
}

/// Whether a background load for `panes` can change what the detail pane
/// shows: one of them is the open pane, or a subpage or a search is showing.
pub(super) fn pane_shows_load(
    current: &str,
    panes: &[&str],
    subpage: bool,
    searching: bool,
) -> bool {
    subpage || searching || panes.contains(&current)
}
