use super::*;

pub(super) fn spawn_recovery_loaders(cx: &mut Context<FinderView>) {
    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async move {
                let journal = Arc::new(operation_journal::Journal::open_default()?);
                let recovery = journal.recover_unambiguous()?;
                let reviews = journal.review_pending()?;
                let undo = journal.undo_store().latest()?;
                Ok::<_, std::io::Error>((journal, recovery, reviews, undo))
            })
            .await;
        let _ = this.update(cx, |this: &mut FinderView, cx| {
            this.journal_loading = false;
            match result {
                Ok((journal, recovery, reviews, undo)) => {
                    this.operation_journal = Some(journal);
                    this.undo_available = undo;
                    this.pending_operations = reviews.len();
                    this.recovery_open = !reviews.is_empty();
                    this.recovery_reviews = reviews;
                    if recovery.finalized != 0 {
                        this.operation_notice = Some(
                            format!(
                                "Files safely completed {} interrupted file operation{}",
                                recovery.finalized,
                                if recovery.finalized == 1 { "" } else { "s" }
                            )
                            .into(),
                        );
                    } else if recovery.active != 0 {
                        this.operation_notice = Some(
                            format!(
                                "Another Files window is safely handling {} file operation{}",
                                recovery.active,
                                if recovery.active == 1 { "" } else { "s" }
                            )
                            .into(),
                        );
                    }
                    if this.pending_operations != 0 {
                        this.operation_error = Some(
                            format!(
                                "Review {} unfinished file operation{} before starting another transfer",
                                this.pending_operations,
                                if this.pending_operations == 1 { "" } else { "s" }
                            )
                            .into(),
                        );
                    }
                }
                Err(_) => {
                    this.operation_journal = None;
                    this.undo_available = None;
                    this.operation_error = Some(
                        "File-operation recovery data could not be verified; transfers are disabled"
                            .into(),
                    );
                }
            }
            cx.notify();
        });
    })
    .detach();

    #[cfg(any(target_os = "linux", test))]
    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
        let result = cx
            .background_executor()
            .spawn(async move {
                let store = Arc::new(trash_store::TrashStore::open_default()?);
                let recovery = store.recover_and_review();
                let undo_availability = store.undo_store().latest();
                Ok::<_, std::io::Error>((store, recovery, undo_availability))
            })
            .await;
        let _ = this.update(cx, |this: &mut FinderView, cx| {
            this.trash_loading = false;
            match result {
                Ok((store, recovery, undo_availability)) => match recovery {
                    Ok((recovery, reviews)) => {
                        this.trash_store = Some(store);
                        match undo_availability {
                            Ok(availability) => this.undo_available = availability,
                            Err(_) => {
                                this.undo_available = None;
                                this.operation_journal = None;
                            }
                        }
                        this.trash_pending = recovery.pending;
                        this.trash_recovery_reviews = reviews;
                        this.trash_recovery_open = recovery.pending != 0;
                        if recovery.finalized != 0 {
                            this.operation_notice = Some(
                                format!(
                                    "Files safely completed {} interrupted Trash operation{}",
                                    recovery.finalized,
                                    if recovery.finalized == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                        if recovery.pending != 0 {
                            this.operation_error = Some(
                                format!(
                                    "Review {} changed Trash operation{} before using Trash",
                                    recovery.pending,
                                    if recovery.pending == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        this.trash_store = Some(store);
                        this.operation_notice =
                            Some("Another Files window is safely handling Trash".into());
                    }
                    Err(_) => {
                        this.trash_store = None;
                        this.trash_recovery_reviews.clear();
                        this.trash_recovery_open = false;
                        this.operation_error = Some(
                            "Trash recovery data could not be verified; Trash actions are disabled"
                                .into(),
                        );
                    }
                },
                Err(_) => {
                    this.trash_store = None;
                    this.trash_recovery_reviews.clear();
                    this.trash_recovery_open = false;
                    this.operation_error = Some(
                        "Trash recovery data could not be verified; Trash actions are disabled"
                            .into(),
                    );
                }
            }
            if this.trash_view && this.trash_store.is_some() {
                this.reload_trash(cx);
            } else {
                cx.notify();
            }
        });
    })
    .detach();
}

pub(super) fn spawn_filesystem_event_loop(
    fs_event_rx: async_channel::Receiver<()>,
    fs_hints: Arc<Mutex<FilesystemHints>>,
    cx: &mut Context<FinderView>,
) {
    // Live directory watching → identity-bound reload/recovery on changes.
    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
        while fs_event_rx.recv().await.is_ok() {
            // FSEvents can deliver a rapid sequence for one logical
            // operation. Wait for 200 ms of quiet, but cap continuous
            // churn at two seconds so the view cannot remain stale.
            for _ in 0..10 {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if fs_event_rx.try_recv().is_err() {
                    break;
                }
            }
            while fs_event_rx.try_recv().is_ok() {}
            let hints = fs_hints
                .lock()
                .map(|mut hints| std::mem::take(&mut *hints))
                .unwrap_or_default();
            if this
                .update(cx, |this: &mut FinderView, cx| {
                    this.reload_after_event(hints, cx)
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

#[cfg(target_os = "linux")]
pub(super) fn spawn_mount_watchers(
    mount_events: async_channel::Sender<rmac_mounts::WatchEvent>,
    mount_event_rx: async_channel::Receiver<rmac_mounts::WatchEvent>,
    cx: &mut Context<FinderView>,
) {
    let watch_sender = mount_events.clone();
    cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
        let mut failures = 0;
        loop {
            let started = std::time::Instant::now();
            let _ = rmac_mounts::watch(watch_sender.clone()).await;
            if watch_sender.is_closed() {
                break;
            }
            let retry = next_mount_watch_retry(failures, started.elapsed());
            failures = retry.0;
            cx.background_executor().timer(retry.1).await;
        }
    })
    .detach();
    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
        while let Ok(mut event) = mount_event_rx.recv().await {
            while let Ok(next) = mount_event_rx.try_recv() {
                event = next;
            }
            if this
                .update(cx, |this: &mut FinderView, cx| {
                    match this.mount_watch_health.record(event) {
                        MountWatchNotice::Unavailable => {
                            if this.operation_error.is_none() {
                                this.operation_error = Some(MOUNT_WATCH_UNAVAILABLE_MESSAGE.into());
                            }
                        }
                        MountWatchNotice::Restored => {
                            if this.operation_error.as_ref().is_some_and(|message| {
                                message.as_ref() == MOUNT_WATCH_UNAVAILABLE_MESSAGE
                            }) {
                                this.operation_error = None;
                            }
                            this.operation_notice =
                                Some("Automatic mounted-volume updates resumed".into());
                        }
                        MountWatchNotice::None => {}
                    }
                    this.refresh_mounts(cx);
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}
