use super::*;

impl FinderView {
    /// The strip Finder shows above the Trash's contents: its name and an
    /// Empty button (geometry S: the owner's Trash was not opened to measure).
    pub(super) fn render_trash_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(any(target_os = "linux", test))]
        let empty = self.trash_items.is_empty();
        #[cfg(not(any(target_os = "linux", test)))]
        let empty = true;
        div()
            .id("trash-bar")
            .role(Role::Toolbar)
            .aria_label(self.file_words.bin())
            .h(px(TRASH_BAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(TRASH_BAR_INSET))
            .border_b_1()
            .border_color(hairline())
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(primary_text())
                    .child(self.file_words.bin()),
            )
            .child(
                Button::new("empty-trash", "Empty")
                    .small()
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, _, cx| this.request_empty_trash(cx))),
            )
    }

    pub(super) fn move_to_trash(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.restore_selected(cx);
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        #[cfg(any(target_os = "linux", test))]
        {
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error = Some(
                    "A changed Trash operation needs manual recovery before another item can be moved"
                        .into(),
                );
                cx.notify();
                return;
            }
            let total = paths.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Moving to Trash".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for path in paths {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.trash(&path, &cancel) {
                            Ok(()) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Trash,
                                    &path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    if completed > 0 && !cfg!(test) {
                        let _ = rmac_sound::play(rmac_sound::Cue::Trash);
                    }
                    let recovery = store.recover_and_review();
                    let undo_availability = store.undo_store().latest();
                    let _ = events.send_blocking(TrashEvent::Finished(TrashCompletion {
                        kind: TrashTaskKind::Move,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                        undo_availability,
                    }));
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }

        #[cfg(not(any(target_os = "linux", test)))]
        {
            let failures = trash::delete_all(&paths)
                .err()
                .map(|error| {
                    file_ops::Failure::message(
                        file_ops::Operation::Trash,
                        &paths[0],
                        None,
                        error.to_string(),
                    )
                })
                .into_iter()
                .collect();
            self.record_operation_failures(failures, cx);
            self.reload(cx);
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn selected_trash_items(&self) -> Vec<trash_store::TrashedItem> {
        let selected_paths = self.selected_paths().into_iter().collect::<BTreeSet<_>>();
        self.trash_items
            .iter()
            .filter(|item| selected_paths.contains(item.data_path()))
            .cloned()
            .collect()
    }

    pub(super) fn restore_selected(&mut self, cx: &mut Context<Self>) {
        if !self.trash_view {
            self.operation_error = Some("Open Trash to restore items".into());
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            if self.transfer.is_some()
                || self.undo_operation.is_some()
                || self.trash_operation.is_some()
            {
                self.operation_error = Some("Wait for the current file operation to finish".into());
                cx.notify();
                return;
            }
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error =
                    Some("A changed Trash operation needs manual recovery before restore".into());
                cx.notify();
                return;
            }
            let items = self.selected_trash_items();
            if items.is_empty() {
                return;
            }
            let total = items.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Restoring".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for item in items {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.restore(&item, &cancel) {
                            Ok(_) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Restore,
                                    &item.original_path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    let recovery = store.recover_and_review();
                    let undo_availability = store.undo_store().latest();
                    let _ = events.send_blocking(TrashEvent::Finished(TrashCompletion {
                        kind: TrashTaskKind::Restore,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                        undo_availability,
                    }));
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.operation_error = Some("Trash restore is available on Linux".into());
            cx.notify();
        }
    }
}
