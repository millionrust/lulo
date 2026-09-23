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

    /// ← / → pressed while Files still has the keyboard.
    pub(in crate::view) fn move_quick_look(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(panel) = &self.quick_look {
            panel.handle.step(delta, cx);
        }
    }

    pub(in crate::view) fn close_quick_look(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.quick_look.take() {
            panel.handle.close(cx);
        }
        cx.notify();
    }
}
