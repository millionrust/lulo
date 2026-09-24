use super::*;

impl FinderView {
    /// Space: open the floating Quick Look panel on the selection, or close
    /// it when it is already open (Space toggles, as on the Mac).
    pub(in crate::view) fn quick_look(&mut self, cx: &mut Context<Self>) {
        if self
            .quick_look
            .as_ref()
            .is_some_and(|panel| panel.handle.is_open())
        {
            self.close_quick_look(cx);
            return;
        }
        if self.applications_view {
            self.operation_error = Some("Quick Look is unavailable for applications".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Restore items before previewing them".into());
            cx.notify();
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.menu_at = None;
        self.info = None;
        self.open_with = None;
        let options = rmac_quick_look::Options { uncompress: true };
        let Some((handle, panel)) = rmac_quick_look::open(paths.clone(), 0, options, cx) else {
            self.operation_error = Some("Quick Look could not open its window".into());
            cx.notify();
            return;
        };
        let events = cx.subscribe(
            &panel,
            |this, _, event: &rmac_quick_look::Event, cx| match event {
                rmac_quick_look::Event::Current(_) => {}
                rmac_quick_look::Event::Uncompress(path) => {
                    this.expand_archives(vec![path.clone()], cx)
                }
            },
        );
        let released = cx.observe_release(&panel, |this, _, cx| {
            this.quick_look = None;
            cx.notify();
        });
        self.quick_look = Some(QuickLookPanel {
            handle,
            paths,
            _subscriptions: [events, released],
        });
        cx.notify();
    }

    /// An arrow key pressed while Files still has the keyboard and Quick
    /// Look is open. A genuine multi-selection steps through exactly the
    /// items Quick Look opened on, as it always did; a single selection
    /// instead keeps moving through the folder behind it, picking up
    /// whatever ↑/↓ or ←/→ means in the current view.
    pub(in crate::view) fn move_quick_look(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(panel) = &self.quick_look else {
            return;
        };
        if panel.paths.len() > 1 {
            panel.handle.step(delta, cx);
            return;
        }
        self.browse_quick_look(delta, cx);
    }

    /// The single item Quick Look opened on is not the whole story: the Mac
    /// keeps ↑/↓ (←/→ in icon and gallery view) moving the *window's own*
    /// selection through the folder, and Quick Look follows it, rather than
    /// being stuck on the one item selected when it opened.
    fn browse_quick_look(&mut self, delta: isize, cx: &mut Context<Self>) {
        let is_column =
            self.view == ViewMode::Column && !self.applications_view && !self.trash_view;
        let next = if is_column {
            let Some(selection) = self.column_selection.clone() else {
                return;
            };
            let Some(ci) = column_index_for_selection(&self.col_stack, &selection) else {
                return;
            };
            let Some(dir) = self.col_stack.get(ci).cloned() else {
                return;
            };
            let mut entries = read_entries(&dir, self.show_hidden);
            sort_entries(&mut entries, self.sort_key, self.sort_asc);
            let Some(target) =
                column_vertical_target(&entries, Some(selection.path.as_path()), delta as i32)
            else {
                return;
            };
            let target = target.clone();
            let path = target.path.clone();
            self.select_column_entry(ci, target);
            path
        } else {
            let Some(current) = self.selected_entry().map(|entry| entry.path.clone()) else {
                return;
            };
            let Some(index) = self.entries.iter().position(|entry| entry.path == current) else {
                return;
            };
            let next_index = if delta < 0 {
                let Some(previous) = index.checked_sub(1) else {
                    return;
                };
                previous
            } else {
                index + 1
            };
            let Some(entry) = self.entries.get(next_index).cloned() else {
                return;
            };
            self.select_single(next_index);
            entry.path
        };
        if let Some(panel) = self.quick_look.as_mut() {
            panel.paths = vec![next.clone()];
            panel.handle.show(vec![next], 0, cx);
        }
        cx.notify();
    }

    pub(in crate::view) fn close_quick_look(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.quick_look.take() {
            panel.handle.close(cx);
        }
        cx.notify();
    }
}
