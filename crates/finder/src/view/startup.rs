mod runtime;
mod shortcuts;

use super::*;
use runtime::*;
use shortcuts::*;

impl FinderView {
    /// `restore_tabs` is true only for a window opened at the default
    /// destination (a plain launch or Dock click): it alone restores the
    /// last-closed window's tabs. A window opened at an explicit
    /// destination — ⌘N's fresh window, Trash, a revealed file, a search —
    /// always starts from exactly one tab, never the last-closed window's,
    /// as the Mac's own New Finder Window does.
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>, restore_tabs: bool) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let file_words = rmac_locale::FileVocabulary::from_environment();

        let (mounts, mount_error) = match rmac_mounts::discover() {
            Ok(mounts) => (mounts, None),
            Err(error) => (
                Vec::new(),
                Some(format!("Could not load mounted volumes: {error}").into()),
            ),
        };

        // User-added Favourites (drag a folder onto the Favourites header),
        // shared by every window and pruned to folders that still exist.
        let favourite_extras = Vec::new();
        let favourite_order = Vec::new();
        // Built from `favourite_extras`/`mounts`/the Settings window's
        // Sidebar and Tags tabs below, right after `view` exists.
        let sections = Vec::new();

        #[cfg(any(target_os = "linux", test))]
        rmac_search::tag_index::start_background_scan(home.clone());

        bind_finder_keys(cx);

        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();
        // Pressing Return runs a recursive Spotlight search of the whole folder tree.
        cx.subscribe(&query, |this, _input, ev: &InputEvent, cx| {
            match ev {
                InputEvent::PressEnter { .. } => this.recursive_search(cx),
                InputEvent::Change if this.query.read(cx).value().is_empty() => {
                    this.search_open = false;
                    // Clearing a recursive-search query leaves the search
                    // result set, so reload the current folder as Finder does.
                    if this.showing_recursive_search() {
                        this.reload(cx);
                    } else {
                        cx.notify();
                    }
                }
                _ => {}
            }
        })
        .detach();

        let icon_size = 64.0;
        let icon_size_slider = cx.new(|_| {
            SliderState::new()
                .min(32.0)
                .max(128.0)
                .step(4.0)
                .default_value(icon_size)
        });
        cx.subscribe(&icon_size_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.icon_size = value.start().clamp(48.0, 88.0);
                let size = this.icon_size;
                this.change_icon_size(size, cx);
            }
        })
        .detach();

        let grid_spacing_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(2.0)
                .default_value(54.0)
        });
        cx.subscribe(&grid_spacing_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.change_options(|o| o.grid_spacing = value.start().clamp(0.0, 100.0), cx);
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
        window.focus(&focus, cx);
        // Top-bar menu commands reach this window's file view even after the
        // top bar took keyboard focus or nothing in the window is focused.
        rmac_ui::register_menu_target(window, &focus, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            // Another app may have copied files while this window was in
            // the background.
            if window.is_window_active() {
                this.publish_app_menu_state(cx);
                this.refresh_pasteboard_state(cx);
                this.refresh_sidebar_favourites(cx);
            }
            if !window.is_window_active()
                && rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx)
            {
                cx.notify();
            }
        })
        .detach();
        let restored = FinderPersistence::restore();
        let presentation = restored.presentation;
        let mut default_options = restored.defaults.clone();
        if restored.folders.is_empty() {
            // Preserve the pre-options window preference when migrating its state.
            default_options.view = presentation.view;
        }
        let (restored_paths, active) = if restore_tabs {
            restored.restorable_session(&home)
        } else {
            (vec![home.clone()], 0)
        };
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
        // Unique for this window's life in this process: the entity id is a
        // generational slot key, so even a reused id never collides with a
        // still-live window's file.
        let window_id = format!(
            "{}-{}",
            std::process::id(),
            cx.entity_id().as_non_zero_u64()
        );
        let finder_persistence = FinderPersistence::start(window_id, cx);

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
            directory_load_pending: false,
            thumbs: std::collections::HashMap::new(),
            entries: Vec::new(),
            root_entries: Vec::new(),
            expanded: BTreeSet::new(),
            child_entries: HashMap::new(),
            list_depths: Vec::new(),
            list_scroll: gpui::ScrollHandle::new(),
            watched_children: BTreeSet::new(),
            selected: BTreeSet::new(),
            menu_at: None,
            menu_purpose: MenuPurpose::Context,
            sidebar_context_path: None,
            sidebar_context_is_favourite: false,
            open_with_menu: None,
            missing_favourite: None,
            help_open: false,
            anchor: None,
            clipboard: Vec::new(),
            clip_cut: false,
            pasteboard_has_files: false,
            renaming: None,
            rename_click_generation: 0,
            show_hidden: false,
            view: presentation.view,
            sidebar_visible: presentation.sidebar_visible,
            sidebar_width: presentation.sidebar_width,
            resizing_sidebar: false,
            finder_persistence,
            folder_options: restored.folders,
            default_options,
            options_path: None,
            browse_view: None,
            view_options_open: false,
            view_options_window: None,
            col_stack: vec![cwd],
            column_selection: None,
            sort_key: SortKey::Name,
            sort_asc: true,
            query,
            icon_size,
            icon_size_slider,
            grid_spacing_slider,
            directory_sizes: Default::default(),
            size_scan_cancel: None,
            back: Vec::new(),
            fwd: Vec::new(),
            file_words,
            sections,
            favourite_extras,
            favourite_order,
            sidebar_drop_index: None,
            info_windows: Vec::new(),
            go_to: None,
            pending_select: None,
            pending_select_many: Vec::new(),
            open_with: None,
            open_generation: 0,
            quick_look: None,
            archive_job: None,
            archive_generation: 0,
            archive_alert: None,
            result_title: None,
            search_summary: None,
            search_relevance_order: false,
            operation_notice: None,
            operation_error: mount_error,
            rename_conflict: None,
            operation_journal: None,
            journal_loading: true,
            undo_available: None,
            undo_operation: None,
            pending_operations: 0,
            recovery_reviews: Vec::new(),
            recovery_open: false,
            recovery_busy: false,
            transfer: None,
            new_folder_busy: false,
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
            applications_view: false,
            #[cfg(any(target_os = "linux", test))]
            trash_items: Vec::new(),
            #[cfg(any(target_os = "linux", test))]
            trash_generation: 0,
            #[cfg(any(target_os = "linux", test))]
            delete_confirmation: None,
            free_bytes: None,
            dragging: None,
            focus,
            native_window_title: "Files".into(),
            watcher,
            filesystem_events: fs_events,
            filesystem_hints: fs_hints.clone(),
            watched: None,
            watched_parent: None,
            search_generation: 0,
            search_cancel: None,
            search_open: false,
            show_path_bar: false,
            show_status_bar: true,
            icon_scroll: gpui::ScrollHandle::new(),
            marquee: None,
            type_select: TypeSelect::default(),
            spring: SpringLoading::default(),
        };
        view.rebuild_sidebar_sections(cx);
        view.refresh_sidebar_favourites(cx);
        super::settings::register_window(cx.weak_entity(), window.window_handle(), cx);
        view.persist_finder_state();
        view.reload(cx);
        view.refresh_pasteboard_state(cx);

        // A close request from outside the window (the compositor, an app
        // quit, logging out) takes the same path as ⌘W and the traffic
        // light: this window's state is saved as the one a fresh launch
        // restores before the window actually goes away. The guard removes
        // the window itself, so the request is always declined here.
        let closing = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            closing
                .update(cx, |this, cx| this.close_finder_window(window, cx))
                .is_err()
        });

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
