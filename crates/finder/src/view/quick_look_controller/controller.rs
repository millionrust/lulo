use super::*;

impl FinderView {
    pub(in crate::view) fn quick_look(&mut self, cx: &mut Context<Self>) {
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
        self.quick_look = Some(QuickLookPanel {
            paths,
            current: 0,
            content: None,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.load_quick_look(cx);
    }

    fn load_quick_look(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.quick_look.as_mut() else {
            return;
        };
        let Some(path) = panel.paths.get(panel.current).cloned() else {
            panel.cancel.store(true, Ordering::Release);
            self.quick_look = None;
            cx.notify();
            return;
        };
        panel.cancel.store(true, Ordering::Release);
        let cancel = Arc::new(AtomicBool::new(false));
        panel.cancel = cancel.clone();
        panel.content = None;
        panel.error = None;
        self.quick_look_generation = self.quick_look_generation.wrapping_add(1);
        let generation = self.quick_look_generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { quick_look::load_cancellable(&path, &cancel) }
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.quick_look_generation != generation {
                    return;
                }
                let Some(panel) = this.quick_look.as_mut() else {
                    return;
                };
                if panel.paths.get(panel.current) != Some(&path) {
                    return;
                }
                match result {
                    Ok(content) => panel.content = Some(content),
                    Err(error) => {
                        panel.error = Some(quick_look_error_message(&error).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::view) fn move_quick_look(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(panel) = self.quick_look.as_mut() else {
            return;
        };
        if panel.paths.len() < 2 {
            return;
        }
        let next = panel
            .current
            .saturating_add_signed(delta)
            .min(panel.paths.len() - 1);
        if next == panel.current {
            return;
        }
        panel.current = next;
        self.load_quick_look(cx);
    }

    pub(in crate::view) fn close_quick_look(&mut self, cx: &mut Context<Self>) {
        self.quick_look_generation = self.quick_look_generation.wrapping_add(1);
        if let Some(panel) = &self.quick_look {
            panel.cancel.store(true, Ordering::Release);
        }
        self.quick_look = None;
        cx.notify();
    }
}
