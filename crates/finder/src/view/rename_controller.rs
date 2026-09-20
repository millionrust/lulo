use super::*;

impl FinderView {
    pub(super) fn rename_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let path = entry.path.clone();
        let name = entry.name.to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => this.rename_commit(cx),
            InputEvent::Blur => this.renaming = None,
            _ => {}
        })
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.renaming = Some((path, input));
        cx.notify();
        // The TextField action handlers exist after the next render. Select
        // the whole generated/current name then so typing replaces it, just
        // like Finder, regardless of whether Rename came from a menu, a name
        // click, Return, or New Folder.
        window.on_next_frame(move |window, cx| {
            window.focus(&focus, cx);
            window.dispatch_action(Box::new(rmac_ui::SelectAll), cx);
        });
    }

    fn rename_commit(&mut self, cx: &mut Context<Self>) {
        let Some((path, input)) = self.renaming.take() else {
            return;
        };
        let new_name = input.read(cx).value().to_string();
        if let Some(entry) = entry_for(&path) {
            let new_name = new_name.trim();
            if !new_name.is_empty() && new_name != entry.name.as_ref() {
                let destination = entry
                    .path
                    .parent()
                    .unwrap_or(self.cwd.as_path())
                    .join(new_name);
                if !destination.exists() {
                    let failures =
                        file_ops::rename(&file_ops::RealFileSystem, &entry.path, &destination)
                            .err()
                            .into_iter()
                            .collect();
                    self.record_operation_failures(failures, cx);
                } else {
                    self.record_operation_failures(
                        vec![file_ops::Failure::message(
                            file_ops::Operation::Rename,
                            &entry.path,
                            Some(&destination),
                            "an item with that name already exists",
                        )],
                        cx,
                    );
                }
            }
        }
        self.reload(cx);
    }
}
