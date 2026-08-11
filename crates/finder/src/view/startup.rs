mod runtime;
mod shortcuts;

use super::*;
use runtime::*;
use shortcuts::*;

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

        bind_finder_keys(cx);

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
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active()
                && rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window)
            {
                cx.notify();
            }
        })
        .detach();
        let restored = FinderPersistence::restore();
        let presentation = restored.presentation;
        let (restored_paths, active) = restored.restorable_session(&home);
        let cwd = restored_paths[active].clone();
        let tabs = restored_paths
            .into_iter()
            .map(|cwd| Tab {
                cwd,
                identity: None,
                back: Vec::new(),
                fwd: Vec::new(),
            })
            .collect();
        let finder_persistence = FinderPersistence::start(cx);

        let mut view = Self {
            cwd: cwd.clone(),
            tabs,
            active,
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
            view: presentation.view,
            sidebar_visible: presentation.sidebar_visible,
            sidebar_width: presentation.sidebar_width,
            resizing_sidebar: false,
            finder_persistence,
            col_stack: vec![cwd],
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
            native_window_title: "Finder".into(),
            watcher,
            filesystem_events: fs_events,
            filesystem_hints: fs_hints.clone(),
            watched: None,
            watched_parent: None,
            search_generation: 0,
            search_cancel: None,
        };
        view.persist_finder_state();
        view.reload(cx);

        spawn_recovery_loaders(cx);

        spawn_filesystem_event_loop(fs_event_rx, fs_hints, cx);

        #[cfg(target_os = "linux")]
        {
            spawn_mount_watchers(mount_events, mount_event_rx, cx);
            view.refresh_mounts(cx);
        }

        view
    }
}
