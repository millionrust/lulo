use super::*;

impl FinderView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let host = home
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string());
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");

        let p =
            |name: &str, path: PathBuf, icon: &'static str, tint: Hsla, kind: PlaceKind| Place {
                name: name.to_string().into(),
                path,
                icon,
                tint,
                kind,
            };

        // Real mounted volumes.
        let mut locations = vec![
            p(
                &host,
                home.clone(),
                "icons/house.svg",
                drive_gray(),
                PlaceKind::Item,
            ),
            p(
                root_volume_name(),
                "/".into(),
                "icons/hard-drive.svg",
                drive_gray(),
                PlaceKind::Item,
            ),
        ];
        let (mounts, mount_error) = match rmac_mounts::discover() {
            Ok(mounts) => (mounts, None),
            Err(error) => (
                Vec::new(),
                Some(format!("Could not load mounted volumes: {error}").into()),
            ),
        };
        locations.extend(mounts.iter().cloned().map(|mount| {
            p(
                &mount.name,
                mount.path,
                "icons/hard-drive.svg",
                drive_gray(),
                if mount.ejectable {
                    PlaceKind::Volume
                } else {
                    PlaceKind::Item
                },
            )
        }));

        #[cfg(target_os = "macos")]
        let tag = |name: &str, color: u32| p(name, PathBuf::new(), "", hsl(color), PlaceKind::Tag);
        let mut favorites = vec![p(
            "Recents",
            PathBuf::new(),
            "icons/clock.svg",
            accent(),
            PlaceKind::Recents,
        )];
        #[cfg(target_os = "macos")]
        favorites.push(p(
            "Applications",
            "/Applications".into(),
            "icons/layout-grid.svg",
            accent(),
            PlaceKind::Item,
        ));
        favorites.extend([
            p(
                "Desktop",
                home.join("Desktop"),
                "icons/folder-fill.svg",
                accent(),
                PlaceKind::Item,
            ),
            p(
                "Documents",
                home.join("Documents"),
                "icons/folder-fill.svg",
                accent(),
                PlaceKind::Item,
            ),
            p(
                "Downloads",
                home.join("Downloads"),
                "icons/download.svg",
                accent(),
                PlaceKind::Item,
            ),
        ]);
        #[cfg(target_os = "linux")]
        favorites.push(p(
            "Trash",
            PathBuf::new(),
            "icons/trash-2.svg",
            accent(),
            PlaceKind::Trash,
        ));
        let mut sections = vec![Section {
            title: "Favorites".into(),
            places: favorites,
        }];
        // Only show iCloud Drive when the real CloudDocs folder exists.
        if icloud.is_dir() {
            sections.push(Section {
                title: "iCloud".into(),
                places: vec![p(
                    "iCloud Drive",
                    icloud,
                    "icons/cloud.svg",
                    accent(),
                    PlaceKind::Item,
                )],
            });
        }
        sections.push(Section {
            title: "Locations".into(),
            places: locations,
        });
        #[cfg(target_os = "macos")]
        sections.push(Section {
            title: "Tags".into(),
            places: vec![
                tag("Red", 0xff3b30),
                tag("Orange", 0xff9500),
                tag("Yellow", 0xffcc00),
                tag("Green", 0x34c759),
                tag("Blue", 0x007aff),
                tag("Purple", 0xaf52de),
                tag("Gray", 0x8e8e93),
            ],
        });

        // Keyboard shortcuts → actions (handled on the focused list).
        cx.bind_keys([
            KeyBinding::new(
                rmac_ui::shortcuts::SELECT_ALL.keystroke,
                SelectAll,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::COPY.keystroke,
                CopyItems,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::CUT.keystroke, CutItems, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::PASTE.keystroke,
                PasteItems,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::UNDO.keystroke,
                UndoOperation,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DUPLICATE.keystroke,
                Duplicate,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DELETE.keystroke,
                MoveToTrash,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DELETE_PERMANENT.keystroke,
                DeletePermanently,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_FOLDER.keystroke,
                NewFolder,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::GO_UP.keystroke, GoUp, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::OPEN_SELECTION.keystroke,
                OpenItems,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ENTER.keystroke,
                RenameItem,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::TOGGLE_HIDDEN.keystroke,
                ToggleHidden,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::SPACE.keystroke,
                QuickLook,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::INFO.keystroke, GetInfo, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_TAB.keystroke,
                NewTab,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::CLOSE.keystroke,
                CloseTab,
                Some("Finder"),
            ),
        ]);

        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        // Pressing Return runs a recursive Spotlight search of the whole folder tree.
        cx.subscribe(&query, |this, _input, ev: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.recursive_search(cx);
            }
        })
        .detach();

        // The bounded channel bridges notify's callback thread to GPUI. A
        // capacity of one coalesces filesystem-event bursts into one reload.
        let (fs_events, fs_event_rx) = async_channel::bounded(1);
        let fs_hints = Arc::new(Mutex::new(FilesystemHints::default()));
        let watcher = filesystem_watcher(fs_events.clone(), fs_hints.clone()).ok();
        #[cfg(target_os = "linux")]
        let (mount_events, mount_event_rx) = async_channel::bounded(8);

        let focus = cx.focus_handle();
        window.focus(&focus);

        let mut view = Self {
            cwd: home.clone(),
            tabs: vec![Tab {
                cwd: home.clone(),
                identity: None,
                back: Vec::new(),
                fwd: Vec::new(),
            }],
            active: 0,
            home: home.clone(),
            mounts: mounts.clone(),
            mount_generation: 0,
            #[cfg(target_os = "linux")]
            mount_watch_health: MountWatchHealth::default(),
            cwd_identity: None,
            directory_generation: 0,
            thumbs: std::collections::HashMap::new(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            menu_at: None,
            anchor: None,
            clipboard: Vec::new(),
            clip_cut: false,
            renaming: None,
            show_hidden: false,
            view: ViewMode::List,
            col_stack: vec![home.clone()],
            sort_key: SortKey::Name,
            sort_asc: true,
            query,
            back: Vec::new(),
            fwd: Vec::new(),
            sections,
            info: None,
            open_with: None,
            open_generation: 0,
            quick_look: None,
            quick_look_generation: 0,
            result_title: None,
            search_summary: None,
            search_relevance_order: false,
            operation_notice: None,
            operation_error: mount_error,
            operation_journal: None,
            journal_loading: true,
            undo_available: None,
            undo_operation: None,
            pending_operations: 0,
            recovery_reviews: Vec::new(),
            recovery_open: false,
            recovery_busy: false,
            transfer: None,
            conflict_preflight: false,
            conflict_batch: None,
            conflict_busy: false,
            #[cfg(any(target_os = "linux", test))]
            trash_store: None,
            #[cfg(any(target_os = "linux", test))]
            trash_loading: true,
            #[cfg(any(target_os = "linux", test))]
            trash_pending: 0,
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_reviews: Vec::new(),
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_open: false,
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_busy: false,
            #[cfg(any(target_os = "linux", test))]
            trash_operation: None,
            trash_view: false,
            #[cfg(any(target_os = "linux", test))]
            trash_items: Vec::new(),
            #[cfg(any(target_os = "linux", test))]
            trash_generation: 0,
            #[cfg(any(target_os = "linux", test))]
            delete_confirmation: None,
            free_bytes: None,
            dragging: false,
            focus,
            watcher,
            filesystem_events: fs_events,
            filesystem_hints: fs_hints.clone(),
            watched: None,
            watched_parent: None,
            search_generation: 0,
            search_cancel: None,
        };
        view.reload(cx);

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

        #[cfg(target_os = "linux")]
        {
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
                                        this.operation_error =
                                            Some(MOUNT_WATCH_UNAVAILABLE_MESSAGE.into());
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
            view.refresh_mounts(cx);
        }

        view
    }
}
