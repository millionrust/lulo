#[cfg(all(target_os = "linux", feature = "wayland"))]
mod ipc;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod drag_endpoint;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use gpui::{
        canvas, div, img, layer_shell::*, point, prelude::*, px, rgba, AccessibleAction,
        AnyWindowHandle, App, Bounds, Context, DisplayId, Entity, ExternalPaths, FocusHandle,
        FontWeight, KeyDownEvent, MouseButton, PathBuilder, PlatformDisplay, QuitMode, Role,
        SharedString, Size, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds,
        WindowHandle, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_shell_ui::tokens;

    // Measured from the owner's Mac 2026-09-23 and drawn in
    // design-lab/dock.html, at the default tile size (macOS Desktop & Dock ▸
    // Size, DOCK-01). `TileMetrics` below turns this into a runtime struct
    // so `DockSettings::tile_size` (a Settings slider and a live separator
    // drag, DOCK-03) can change it; every field is a measured ratio of the
    // tile size, exactly as the consts it replaces were.
    // The macOS icon grid shows a 52-of-64 squircle; rmac art draws its
    // squircle at 824/1024 of the image, so the image is scaled to match.
    const ICON_SQUIRCLE: f32 = 0.8125;
    const ICON_ART_SCALE: f32 = ICON_SQUIRCLE * 1024.0 / 824.0;
    // Separator: a 1 × 62 line centred in the shelf with 13 either side
    // (plus the tile gap), so the pitch across it is 99 (at the default
    // tile size).
    const SEPARATOR_WIDTH: f32 = 1.0;

    /// Runtime Dock tile geometry: every value is a measured ratio of
    /// `icon_size` (see the module doc comment above).
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct TileMetrics {
        icon_size: f32,
        // Pitch 68: tiles sit 4 apart (their visible squircles 16 apart).
        icon_gap: f32,
        // Tile → shelf rim on every side (16 from the visible squircle).
        shelf_padding: f32,
        shelf_thickness: f32,
        // Shelf rim → screen edge.
        shelf_bottom_margin: f32,
        exclusive_zone: f32,
        separator_length: f32,
        separator_margin: f32,
        separator_slot: f32,
        // Running dot: 4 across, its centre 4 below the tile.
        indicator_size: f32,
        indicator_offset: f32,
        tooltip_bottom: f32,
    }

    impl TileMetrics {
        fn new(tile_size: f32) -> Self {
            let icon_size = if tile_size.is_finite() {
                tile_size.clamp(
                    rmac_shell_settings::MIN_DOCK_TILE_SIZE,
                    rmac_shell_settings::MAX_DOCK_TILE_SIZE,
                )
            } else {
                rmac_shell_settings::DEFAULT_DOCK_TILE_SIZE
            };
            let shelf_padding = icon_size * 0.15625;
            let shelf_thickness = icon_size + 2.0 * shelf_padding;
            let shelf_bottom_margin = icon_size * 0.078125;
            let exclusive_zone = shelf_thickness + shelf_bottom_margin;
            let separator_margin = icon_size * 0.203125;
            Self {
                icon_size,
                icon_gap: icon_size * 0.0625,
                shelf_padding,
                shelf_thickness,
                shelf_bottom_margin,
                exclusive_zone,
                separator_length: icon_size * 0.96875,
                separator_margin,
                separator_slot: SEPARATOR_WIDTH + 2.0 * separator_margin,
                indicator_size: icon_size * 0.0625,
                indicator_offset: icon_size * 0.03125,
                tooltip_bottom: exclusive_zone + 6.0,
            }
        }
    }

    impl Default for TileMetrics {
        fn default() -> Self {
            Self::new(rmac_shell_settings::DEFAULT_DOCK_TILE_SIZE)
        }
    }

    // The tile whose Dock menu is open is darkened (black ≈ 53 %).
    const MENU_OPEN_DIM: u32 = 0x00000087;
    // Dock menu (captures f046–f052): no title, 5 inside the edge (4 padding
    // + 1 rim), 11-tall separators whose line is inset 16, text 16 from the
    // edge or 24.5 with a check column. The pointer is 20 × 10, its tip on
    // the tile centre 26.5 from the menu's left edge; the body ends 25.5
    // above the shelf.
    const MENU_PADDING: f32 = 4.0;
    const MENU_BORDER: f32 = 1.0;
    const MENU_SEPARATOR: f32 = 11.0;
    const MENU_ROW_INSET: f32 = 11.0;
    const MENU_CHECK_INSET: f32 = 4.0;
    const MENU_CHECK_COLUMN: f32 = 15.5;
    const MENU_ANCHOR_INSET: f32 = 26.5;
    const MENU_SHELF_GAP: f32 = 25.5;
    const MENU_POINTER_WIDTH: f32 = 20.0;
    const MENU_POINTER_HEIGHT: f32 = 10.0;
    // Options ▸: room for the chevron, and how far a submenu tucks under
    // its parent panel (rmac values; the submenu was not measured).
    const MENU_CHEVRON_COLUMN: f32 = 20.0;
    const MENU_SUBMENU_OVERLAP: f32 = 3.0;
    const TOOLTIP_WIDTH: f32 = 240.0;
    // Stack popover (item 1, design-lab/dock-stack-popover.html, S: not
    // measured against the owner's Mac). GPUI has no rotate/scale
    // transform, so the Fan is a plain row rather than the Mac's fanned
    // stagger; both views cap how many items they draw and note the rest.
    const STACK_POPOVER_WIDTH: f32 = 260.0;
    const STACK_POPOVER_ITEM: f32 = 56.0;
    const STACK_POPOVER_MAX_ITEMS: usize = 8;
    // Badge bubble and progress bar published by apps (rmac values; not yet
    // measured against a Mac badge).
    const BADGE_SIZE: f32 = 20.0;
    const PROGRESS_HEIGHT: f32 = 8.0;
    const READY_FILE_ENV: &str = "RMAC_DOCK_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_DOCK_RENDER_COUNT_DIR";
    static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

    struct DockStatus {
        snapshot: Option<rmac_dock_runtime::Snapshot>,
        outputs: std::collections::BTreeSet<uuid::Uuid>,
        removed_outputs: std::collections::BTreeSet<uuid::Uuid>,
        actions: rmac_dock_system::interaction::State,
        /// Displays whose Dock is auto-hidden; their shelf material hides too.
        hidden_displays: std::collections::BTreeSet<u64>,
        /// Badges, progress and attention apps publish over LauncherEntry.
        launcher: rmac_dock::badges::LauncherEntries,
    }

    impl DockStatus {
        fn new(
            updates: async_channel::Receiver<rmac_dock_runtime::Update>,
            reconcile: async_channel::Sender<()>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| {
                while let Ok(update) = updates.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            let was_ready = this.snapshot.is_some();
                            if update.visible {
                                cx.notify();
                            }
                            let outputs = update
                                .snapshot
                                .outputs
                                .iter()
                                .map(rmac_shell_layer::stable_output_uuid)
                                .collect();
                            if was_ready
                                && rmac_shell_layer::output_reappeared(
                                    &this.outputs,
                                    &outputs,
                                    &mut this.removed_outputs,
                                )
                            {
                                std::process::exit(
                                    rmac_shell_layer::WAYLAND_OUTPUT_RESTART_EXIT_CODE,
                                );
                            }
                            this.outputs = outputs;
                            this.snapshot = Some(update.snapshot);
                        })
                        .is_err()
                    {
                        break;
                    }
                    let _ = reconcile.try_send(());
                }
            })
            .detach();
            Self {
                snapshot: None,
                outputs: std::collections::BTreeSet::new(),
                removed_outputs: std::collections::BTreeSet::new(),
                actions: rmac_dock_system::interaction::State::default(),
                hidden_displays: std::collections::BTreeSet::new(),
                launcher: rmac_dock::badges::LauncherEntries::default(),
            }
        }

        fn snapshot(&self) -> Option<&rmac_dock_runtime::Snapshot> {
            self.snapshot.as_ref()
        }

        fn model(&self) -> Option<&rmac_dock::Model> {
            self.snapshot().map(|snapshot| &snapshot.model)
        }

        fn surfaces(&self) -> Option<Vec<DockSurface>> {
            let snapshot = self.snapshot()?;
            let fullscreen = rmac_shell_layer::top_bar_output_policies(&snapshot.compositor);
            let shelf_extent = dock_shelf_extent(snapshot);
            snapshot.surface_plan.as_ref().ok().map(|surfaces| {
                surfaces
                    .iter()
                    .map(|surface| {
                        let output = rmac_shell_layer::stable_output_uuid(&surface.output);
                        DockSurface::from_description(
                            surface,
                            fullscreen.get(&output).copied().unwrap_or(false),
                            shelf_extent,
                            snapshot.settings.tile_size,
                        )
                    })
                    .collect()
            })
        }

        /// The compositor has confirmed it has no outputs to place the Dock
        /// on (no niri, for example) rather than merely starting up: the
        /// Dock falls back to a default shelf on every display GPUI itself
        /// knows about, the same way the wallpaper and top bar do.
        fn needs_display_fallback(&self) -> bool {
            self.snapshot().is_some_and(|snapshot| {
                matches!(
                    snapshot.health.compositor,
                    rmac_dock_runtime::SourceHealth::Unavailable { .. }
                ) && snapshot
                    .surface_plan
                    .as_ref()
                    .is_ok_and(|surfaces| surfaces.is_empty())
            })
        }
    }

    fn dock_shelf_extent(snapshot: &rmac_dock_runtime::Snapshot) -> f32 {
        let metrics = TileMetrics::new(snapshot.settings.tile_size);
        let entries = snapshot.content.applications.len();
        let minimized = snapshot
            .content
            .places
            .iter()
            .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Minimized(_)))
            .count();
        // Folder/file stacks (§ folder/file stacks left of the Trash) sit
        // between the minimized group and Trash, same as any other place.
        let stacks = snapshot
            .content
            .places
            .iter()
            .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Stack(_)))
            .count();
        let pinned = snapshot
            .model
            .items
            .iter()
            .take_while(|item| item.pinned)
            .count();
        let separates_running = pinned > 0 && pinned < entries;
        let separators = usize::from(separates_running) + usize::from(entries > 0);
        let items = entries + minimized + stacks + 1;
        let children = items + separators;
        metrics.icon_size * items as f32
            + metrics.separator_slot * separators as f32
            + metrics.icon_gap * children.saturating_sub(1) as f32
            + 2.0 * metrics.shelf_padding
    }

    struct TileDragUi {
        app_id: String,
        drag: rmac_dock::reorder::TileDrag,
        /// Kept-app order when the press began; a drop is applied only while
        /// the settings still hold exactly this order.
        pinned: Vec<String>,
        /// Pointer in surface coordinates and its offset inside the icon.
        pointer: (f32, f32),
        grab: (f32, f32),
        icon: Option<PathBuf>,
    }

    struct RemovedTile {
        icon: Option<PathBuf>,
        origin: (f32, f32),
        started_ms: u64,
    }

    /// A live drag of the Dock separator (DOCK-03): dragging it towards or
    /// away from the screen edge previews a new tile size, committed to
    /// `DockSettings::tile_size` on release.
    struct SeparatorDragUi {
        /// Pointer distance from the shelf's inner edge when the drag
        /// began, and the tile size at that moment.
        start_lift: f32,
        start_size: f32,
        preview_size: f32,
    }

    /// The Dock separator's own right-click menu (DOCK-02): Turn Hiding On/
    /// Off, Turn Magnification On/Off, Position on Screen ▸, Minimise Using
    /// ▸, Dock Settings…. Modelled locally (not in `rmac_dock::menu`) since
    /// every row here writes `DockSettings` directly rather than going
    /// through the Dock's item/stack model.
    struct SeparatorMenu {
        anchor: f32,
        dock: rmac_shell_settings::DockSettings,
        submenu_open: Option<SeparatorSubmenu>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SeparatorSubmenu {
        Position,
        MinimizeUsing,
    }

    struct DockMenu {
        anchor: f32,
        session: rmac_dock::menu::Session,
        /// Options ▸ is open (the pointer rested on its parent row).
        submenu_open: bool,
        /// Options ▸ Open at Login, once the XDG autostart state is read.
        login: Option<LoginOption>,
    }

    /// A folder/file stack's Fan or Grid popover (§ folder/file stacks left
    /// of the Trash): a click on the stack tile opens this rather than
    /// activating the stack (`StackActivation::OpenDirectory` is the
    /// context menu's separate "Open <name>" row). Dock-local UI state,
    /// never reaching the dispatch layer (commit 10e708d8).
    struct StackPopoverUi {
        kind: rmac_shell_settings::DockStackKind,
        name: String,
        display_as: rmac_shell_settings::DockStackDisplayAs,
        view_content_as: rmac_shell_settings::DockStackViewContentAs,
        /// Resting centre of the stack tile that opened this.
        anchor: f32,
        /// The latest directory listing, once the background scan and live
        /// watch (`rmac_desktop::watch`/`scan`) deliver one.
        snapshot: Option<rmac_desktop::Snapshot>,
        /// Keeps the watch (and the background task that owns it) alive
        /// only while the popover is open: dropping this cancels both, so
        /// closing the popover leaves nothing watching the filesystem.
        _watch: gpui::Task<()>,
    }

    /// The app's XDG autostart state behind Options ▸ Open at Login.
    #[derive(Clone)]
    struct LoginOption {
        desktop_entry: PathBuf,
        /// Autostart entry ID: the desktop file name.
        id: String,
        enabled: bool,
    }

    /// ⌃F3 keyboard mode on one Dock (design-lab/dock.html, scene 3).
    ///
    /// The Dock's own layer surface is created with no keyboard
    /// interactivity, and GPUI's `PlatformWindow` has no call to change a
    /// layer surface's interactivity after it is mapped. Re-creating the
    /// Dock with an exclusive keyboard would drop and re-reserve its work
    /// area (every window would reflow twice) and lose hover, bounce and
    /// menu state. So, like the ⌘Tab switcher, the keyboard is held by a
    /// separate, invisible 1 × 1 overlay surface with exclusive
    /// interactivity that exists only while the Dock is focused; it forwards
    /// keys here and carries the accessible focus Orca announces.
    struct KeyboardMode {
        navigator: rmac_dock::keyboard::Navigator,
        surface: WindowHandle<DockKeyboard>,
        /// The window that had the keyboard before ⌃F3; Esc refocuses it.
        previous_window: Option<rmac_compositor::WindowId>,
    }

    /// One tile the keyboard can reach, in shelf order.
    #[derive(Clone)]
    struct KeyTarget {
        id: rmac_dock::presentation::EntryId,
        /// Shown in the name bubble.
        name: String,
        accessible: String,
        /// Resting centre along the Dock axis, relative to the shelf start.
        center: f32,
    }

    /// What the focus surface exposes as the focused accessible node.
    #[derive(Clone, PartialEq)]
    struct Announcement {
        role: Role,
        label: SharedString,
        /// Changes with the focused tile or row, so each move is a new
        /// focus event rather than a rename of the same node.
        key: usize,
    }

    const MENU_ANNOUNCEMENT_KEY: usize = 1 << 20;

    fn dock_edge(placement: rmac_shell_settings::DockPlacement) -> rmac_dock::keyboard::Edge {
        match placement {
            rmac_shell_settings::DockPlacement::Bottom => rmac_dock::keyboard::Edge::Bottom,
            rmac_shell_settings::DockPlacement::Left => rmac_dock::keyboard::Edge::Left,
            rmac_shell_settings::DockPlacement::Right => rmac_dock::keyboard::Edge::Right,
        }
    }

    fn trash_accessible_label(model: &rmac_dock::Model) -> String {
        let trash = model
            .special_items
            .iter()
            .find(|item| item.kind == rmac_dock::SpecialItemKind::Trash);
        match trash.and_then(|trash| trash.item_count) {
            Some(0) => "Trash, empty".into(),
            Some(1) => "Trash, 1 item".into(),
            Some(count) => format!("Trash, {count} items"),
            None => "Trash unavailable".into(),
        }
    }

    /// Applications, minimized windows and Trash in shelf order, with the
    /// same resting centres the renderer lays out.
    fn keyboard_targets(snapshot: &rmac_dock_runtime::Snapshot) -> Vec<KeyTarget> {
        let metrics = TileMetrics::new(snapshot.settings.tile_size);
        let entries = &snapshot.content.applications;
        let pinned = snapshot
            .model
            .items
            .iter()
            .take_while(|item| item.pinned)
            .count();
        let separates_running = pinned > 0 && pinned < entries.len();
        let mut targets: Vec<KeyTarget> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| KeyTarget {
                id: entry.id.clone(),
                name: entry.label.clone(),
                accessible: entry.accessible_label.clone(),
                center: metrics.shelf_padding
                    + metrics.icon_size / 2.0
                    + index as f32 * (metrics.icon_size + metrics.icon_gap)
                    + if separates_running && index >= pinned {
                        metrics.separator_slot + metrics.icon_gap
                    } else {
                        0.0
                    },
            })
            .collect();
        let trash_center =
            dock_shelf_extent(snapshot) - metrics.shelf_padding - metrics.icon_size / 2.0;
        // Shelf order past the applications is minimized windows, then
        // folder/file stacks, then Trash (§ folder/file stacks left of the
        // Trash; `rmac_dock::presentation::ShelfContent::project`).
        let minimized: Vec<_> = snapshot
            .content
            .places
            .iter()
            .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Minimized(_)))
            .collect();
        let stacks: Vec<_> = snapshot
            .content
            .places
            .iter()
            .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Stack(_)))
            .collect();
        let pitch = metrics.icon_size + metrics.icon_gap;
        let stack_count = stacks.len();
        let trailing = minimized.len() + stack_count;
        targets.extend(
            minimized
                .into_iter()
                .enumerate()
                .map(|(index, entry)| KeyTarget {
                    id: entry.id.clone(),
                    name: entry.label.clone(),
                    accessible: entry.accessible_label.clone(),
                    center: trash_center - (trailing - index) as f32 * pitch,
                }),
        );
        targets.extend(
            stacks
                .into_iter()
                .enumerate()
                .map(|(index, entry)| KeyTarget {
                    id: entry.id.clone(),
                    name: entry.label.clone(),
                    accessible: entry.accessible_label.clone(),
                    center: trash_center - (stack_count - index) as f32 * pitch,
                }),
        );
        targets.push(KeyTarget {
            id: rmac_dock::presentation::EntryId::Special(rmac_dock::SpecialItemKind::Trash),
            name: "Trash".into(),
            accessible: trash_accessible_label(&snapshot.model),
            center: trash_center,
        });
        targets
    }

    /// Never keep an invisible exclusive surface that never got the keyboard.
    const KEYBOARD_ACTIVATION_TIMEOUT: Duration = Duration::from_millis(1_000);

    /// The invisible surface that holds the keyboard while the Dock is
    /// focused. It draws nothing and takes no pointer input.
    struct DockKeyboard {
        dock: WeakEntity<Dock>,
        focus: FocusHandle,
        announcement: Announcement,
        was_active: bool,
        closing: bool,
        input_region_set: bool,
    }

    impl DockKeyboard {
        fn new(
            dock: WeakEntity<Dock>,
            announcement: Announcement,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.was_active = true;
                } else if this.was_active {
                    // Another surface took the keyboard (⌘Tab, a menu bar
                    // click, the lock screen): leave without refocusing.
                    this.close(false, window, cx);
                }
            })
            .detach();
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(KEYBOARD_ACTIVATION_TIMEOUT)
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if !this.was_active {
                        this.close(false, window, cx);
                    }
                });
            })
            .detach();
            Self {
                dock,
                focus,
                announcement,
                was_active: false,
                closing: false,
                input_region_set: false,
            }
        }

        fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
            if self.closing {
                return;
            }
            self.closing = true;
            let _ = self
                .dock
                .update(cx, |dock, cx| dock.leave_keyboard(restore, cx));
            window.remove_window();
        }

        fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
            cx.stop_propagation();
            if self.closing {
                return;
            }
            let key = event.keystroke.key.clone();
            let modifiers = event.keystroke.modifiers;
            let next = self
                .dock
                .update(cx, |dock, cx| {
                    dock.keyboard_key(&key, modifiers.shift, modifiers.alt, cx)
                })
                .ok()
                .flatten();
            match next {
                Some(announcement) => {
                    if announcement != self.announcement {
                        self.announcement = announcement;
                        cx.notify();
                    }
                }
                None => {
                    // The Dock already left keyboard mode (or is gone).
                    self.closing = true;
                    window.remove_window();
                }
            }
        }
    }

    impl Render for DockKeyboard {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if !self.input_region_set {
                self.input_region_set = true;
                window.set_input_region(Some(&[]));
            }
            // The container holds real focus; the child stands for the
            // focused tile (or menu row) through aria-activedescendant, so
            // Orca reads "Safari, button" on every move.
            div()
                .id("dock-keyboard")
                .track_focus(&self.focus)
                .role(Role::Toolbar)
                .aria_label("Dock")
                .size_full()
                .on_key_down(cx.listener(Self::key_down))
                .child(
                    div()
                        .id(("dock-keyboard-focus", self.announcement.key))
                        .role(self.announcement.role)
                        .aria_label(self.announcement.label.clone())
                        .aria_active_descendant()
                        .size_full(),
                )
        }
    }

    struct Dock {
        display_id: u64,
        placement: rmac_shell_settings::DockPlacement,
        render_count: u64,
        status: Entity<DockStatus>,
        hovered_item: Option<(f32, String)>,
        context_menu: Option<DockMenu>,
        input_region: Option<(f32, f32, bool, bool)>,
        pointer_inside: bool,
        hidden: bool,
        /// Auto-hide slide in progress: (start ms, sliding out).
        hide_slide: Option<(u64, bool)>,
        fullscreen: bool,
        overview_visible: bool,
        visibility_policy: Option<(bool, bool)>,
        hide_generation: u64,
        content: rmac_dock::presentation::ShelfContent,
        /// A pressed kept app, becoming a drag once the pointer moves.
        tile_drag: Option<TileDragUi>,
        /// A live insertion-point hint from a drag out of Apps
        /// (`drag_endpoint::Command::Hover`, item 7): the fraction along
        /// the kept-apps list the pointer last reported.
        apps_drag_hover: Option<f32>,
        /// Neighbours sliding into place: (offset along the axis, start ms).
        slides: std::collections::BTreeMap<String, (f32, u64)>,
        /// Kept-app order shown until the settings watcher confirms a drop.
        pending_pins: Option<(Vec<String>, u64)>,
        /// An icon dragged off the Dock, fading out where it was dropped.
        removing: Option<RemovedTile>,
        /// Resting geometry of the last frame, for drags that start on it.
        shelf_start: f32,
        surface_size: (f32, f32),
        /// Clock for tile animations (bounce, slide, fade).
        epoch: std::time::Instant,
        bounces: rmac_dock::bounce::BounceTracker,
        /// The tile under a held primary button: macOS darkens it.
        pressed: Option<String>,
        /// Option held (read from pointer events; the Dock surface never takes the
        /// keyboard): Dock menus show Force Quit instead of Quit.
        option_held: bool,
        /// Reviewed Trash contents waiting for the Empty Trash alert.
        trash_review: Option<rmac_dock_system::dispatch::ReviewedTrash>,
        /// ⌃F3: the tile with keyboard focus and the surface holding it.
        keyboard: Option<KeyboardMode>,
        /// DockSettings::tile_size (DOCK-01), refreshed live every render.
        tile_size: f32,
        /// A live drag of the separator (DOCK-03), previewing a new size
        /// before it commits to settings on release.
        separator_drag: Option<SeparatorDragUi>,
        /// The separator's own right-click menu (DOCK-02), independent of
        /// `context_menu` since it has no `EntryId` of its own.
        separator_menu: Option<SeparatorMenu>,
        /// The open folder/file stack popover (item 1), if any.
        stack_popover: Option<StackPopoverUi>,
    }

    impl Dock {
        fn new(
            display_id: DisplayId,
            surface: DockSurface,
            status: Entity<DockStatus>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            let hidden = surface.fullscreen && !surface.overview_visible;
            if hidden {
                let id = u64::from(display_id);
                status.update(cx, |status, _| {
                    status.hidden_displays.insert(id);
                });
            }
            Self {
                display_id: u64::from(display_id),
                placement: surface.placement,
                render_count: 0,
                status,
                hovered_item: None,
                context_menu: None,
                input_region: None,
                pointer_inside: false,
                hidden,
                hide_slide: None,
                fullscreen: surface.fullscreen,
                overview_visible: surface.overview_visible,
                visibility_policy: None,
                hide_generation: 0,
                content: rmac_dock::presentation::ShelfContent::default(),
                tile_drag: None,
                apps_drag_hover: None,
                slides: std::collections::BTreeMap::new(),
                pending_pins: None,
                removing: None,
                shelf_start: 0.0,
                surface_size: (0.0, 0.0),
                epoch: std::time::Instant::now(),
                bounces: rmac_dock::bounce::BounceTracker::default(),
                pressed: None,
                option_held: false,
                trash_review: None,
                keyboard: None,
                tile_size: surface.tile_size,
                separator_drag: None,
                separator_menu: None,
                stack_popover: None,
            }
        }

        /// The runtime tile geometry (DOCK-01): the settings-driven resting
        /// size, or a live separator-drag preview while one is in progress
        /// (DOCK-03). The exclusive zone the compositor actually reserves
        /// only updates once a drag commits (`open_dock`/`DockWindows`), so
        /// this only ever affects what is drawn, never the work area, until
        /// then.
        fn metrics(&self) -> TileMetrics {
            TileMetrics::new(
                self.separator_drag
                    .as_ref()
                    .map_or(self.tile_size, |drag| drag.preview_size),
            )
        }

        fn now_ms(&self) -> u64 {
            u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        /// Distance of a surface point beyond the shelf's inner edge (towards
        /// the screen centre); negative inside the shelf.
        fn lift_of(&self, x: f32, y: f32) -> f32 {
            let metrics = self.metrics();
            let (width, height) = self.surface_size;
            let depth = metrics.shelf_bottom_margin + metrics.shelf_thickness;
            match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => height - depth - y,
                rmac_shell_settings::DockPlacement::Left => x - depth,
                rmac_shell_settings::DockPlacement::Right => width - depth - x,
            }
        }

        fn axis_of(&self, x: f32, y: f32) -> f32 {
            match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => x,
                _ => y,
            }
        }

        /// Resting centre of kept-app tile `index` along the Dock axis.
        fn pinned_center(&self, index: usize) -> f32 {
            let metrics = self.metrics();
            self.shelf_start
                + metrics.shelf_padding
                + metrics.icon_size / 2.0
                + index as f32 * (metrics.icon_size + metrics.icon_gap)
        }

        fn begin_tile_drag(
            &mut self,
            app_id: &str,
            icon: Option<PathBuf>,
            position: (f32, f32),
            cx: &mut Context<Self>,
        ) {
            let metrics = self.metrics();
            let Some(model) = self.model_snapshot(cx) else {
                return;
            };
            let pinned: Vec<String> = model
                .items
                .iter()
                .take_while(|item| item.pinned)
                .map(|item| item.id.clone())
                .collect();
            let order = self
                .pending_pins
                .as_ref()
                .map(|(order, _)| order.clone())
                .unwrap_or_else(|| pinned.clone());
            if order != pinned {
                // A previous drop is still being saved.
                return;
            }
            let Some(source) = pinned.iter().position(|id| id == app_id) else {
                return;
            };
            let centers: Vec<f32> = (0..pinned.len())
                .map(|index| self.pinned_center(index))
                .collect();
            let axis = self.axis_of(position.0, position.1);
            let lift = self.lift_of(position.0, position.1);
            let Some(drag) = rmac_dock::reorder::TileDrag::begin(
                source,
                centers,
                axis,
                lift,
                metrics.icon_size,
                true,
            ) else {
                return;
            };
            // Where the pointer sits inside the icon, so the lifted icon
            // does not jump to the pointer.
            let center = self.pinned_center(source);
            let (width, height) = self.surface_size;
            let tile_origin = match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => (
                    center - metrics.icon_size / 2.0,
                    height
                        - metrics.shelf_bottom_margin
                        - metrics.shelf_padding
                        - metrics.icon_size,
                ),
                rmac_shell_settings::DockPlacement::Left => (
                    metrics.shelf_bottom_margin + metrics.shelf_padding,
                    center - metrics.icon_size / 2.0,
                ),
                rmac_shell_settings::DockPlacement::Right => (
                    width - metrics.shelf_bottom_margin - metrics.shelf_padding - metrics.icon_size,
                    center - metrics.icon_size / 2.0,
                ),
            };
            self.tile_drag = Some(TileDragUi {
                app_id: app_id.to_owned(),
                drag,
                pinned,
                pointer: position,
                grab: (position.0 - tile_origin.0, position.1 - tile_origin.1),
                icon,
            });
        }

        fn update_tile_drag(&mut self, position: (f32, f32), cx: &mut Context<Self>) {
            let metrics = self.metrics();
            let axis = self.axis_of(position.0, position.1);
            let lift = self.lift_of(position.0, position.1);
            let now = self.now_ms();
            let Some(ui) = self.tile_drag.as_mut() else {
                return;
            };
            let before = ui.drag.preview_order();
            let was_active = ui.drag.is_active();
            let state = ui.drag.update(axis, lift);
            ui.pointer = position;
            if !state.active {
                return;
            }
            if !was_active {
                // The label goes away and the drag owns the whole output.
                self.hovered_item = None;
                self.input_region = None;
            }
            let after = ui.drag.preview_order();
            if before != after {
                // Neighbours slide from where they were to their new slot.
                let pitch = metrics.icon_size + metrics.icon_gap;
                for (new_slot, original) in after.iter().enumerate() {
                    if *original == ui.drag.source() {
                        continue;
                    }
                    let Some(old_slot) = before.iter().position(|index| index == original) else {
                        continue;
                    };
                    if old_slot != new_slot {
                        let id = ui.pinned[*original].clone();
                        let current = self
                            .slides
                            .get(&id)
                            .map(|(offset, started)| {
                                offset
                                    * (1.0
                                        - rmac_dock::reorder::ease_out(
                                            now - started,
                                            rmac_dock::reorder::SLIDE_MS,
                                        ))
                            })
                            .unwrap_or(0.0);
                        let offset = (old_slot as f32 - new_slot as f32) * pitch + current;
                        self.slides.insert(id, (offset, now));
                    }
                }
            }
            cx.notify();
        }

        fn finish_tile_drag(&mut self, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
            let Some(ui) = self.tile_drag.take() else {
                return;
            };
            self.input_region = None;
            self.slides.clear();
            cx.notify();
            let current: Option<Vec<String>> = self.model_snapshot(cx).map(|model| {
                model
                    .items
                    .iter()
                    .take_while(|item| item.pinned)
                    .map(|item| item.id.clone())
                    .collect()
            });
            let command = match ui.drag.finish() {
                rmac_dock::reorder::TileDrop::Click => {
                    self.activate_entry(&ui.app_id, modifiers, cx);
                    return;
                }
                rmac_dock::reorder::TileDrop::NoChange => return,
                rmac_dock::reorder::TileDrop::Move { from, to } => {
                    let mut order = ui.pinned.clone();
                    let moved = order.remove(from);
                    order.insert(to, moved);
                    (
                        rmac_dock::PinCommand::MoveTo {
                            app_id: ui.app_id.clone(),
                            index: to,
                        },
                        order,
                    )
                }
                rmac_dock::reorder::TileDrop::Remove => {
                    self.removing = Some(RemovedTile {
                        icon: ui.icon.clone(),
                        origin: (ui.pointer.0 - ui.grab.0, ui.pointer.1 - ui.grab.1),
                        started_ms: self.now_ms(),
                    });
                    let order = ui
                        .pinned
                        .iter()
                        .filter(|id| **id != ui.app_id)
                        .cloned()
                        .collect();
                    (
                        rmac_dock::PinCommand::Unpin {
                            app_id: ui.app_id.clone(),
                        },
                        order,
                    )
                }
            };
            // Apply only against the exact order the user manipulated.
            if current.as_ref() != Some(&ui.pinned) {
                eprintln!("the kept apps changed during the drag; the drop was not applied");
                return;
            }
            let (command, order) = command;
            self.pending_pins = Some((order, self.now_ms()));
            self.dispatch_action(
                rmac_dock::menu::Action::Context(rmac_dock::ContextAction::UpdatePins(command)),
                cx,
            );
        }

        /// Files dropped on an app's tile open with that app, launching it
        /// (with its bounce) when it is not running.
        fn open_dropped_files(
            &mut self,
            app_id: &str,
            desktop_entry: PathBuf,
            paths: Vec<PathBuf>,
            cx: &mut Context<Self>,
        ) {
            if paths.is_empty() {
                return;
            }
            self.note_launch(app_id);
            cx.notify();
            cx.background_executor()
                .spawn(async move {
                    // gio interprets the trusted entry's field codes without a
                    // shell, exactly as Files' Open With does.
                    let status = std::process::Command::new("gio")
                        .arg("launch")
                        .arg(&desktop_entry)
                        .args(&paths)
                        .status();
                    if !status.is_ok_and(|status| status.success()) {
                        eprintln!("could not open the dropped files");
                    }
                })
                .detach();
        }

        /// Files dropped on the Trash move to the Trash.
        fn trash_dropped_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
            if paths.is_empty() {
                return;
            }
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = trash::delete_all(&paths) {
                        eprintln!("could not move the dropped items to the Trash: {error}");
                    }
                })
                .detach();
        }

        /// An application (.desktop entry) dropped on the Dock is kept in it.
        fn keep_dropped_applications(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
            let entries: Vec<PathBuf> = paths
                .into_iter()
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "desktop")
                })
                .collect();
            if entries.is_empty() {
                return;
            }
            cx.background_executor()
                .spawn(async move {
                    use rmac_dock_system::Backend as _;
                    let Ok(catalog) = rmac_apps::discover() else {
                        eprintln!("could not read the installed applications");
                        return;
                    };
                    let backend = rmac_dock_system::SystemBackend;
                    for entry in entries {
                        let file_name = entry
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_owned();
                        let Some(application) = catalog.iter().find(|application| {
                            application.source == entry || application.id == file_name
                        }) else {
                            continue;
                        };
                        let command = rmac_dock::PinCommand::Pin {
                            app_id: application.id.clone(),
                        };
                        if let Err(error) = backend.update_pins(&command).await {
                            eprintln!(
                                "could not keep the application in the Dock: {}",
                                error.detail
                            );
                        }
                    }
                })
                .detach();
        }

        /// A folder or file dropped on the Dock (anywhere but the Trash and
        /// pinned application tiles, which have their own drop handling)
        /// becomes a stack, kept left of the Trash.
        fn keep_dropped_stacks(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
            if paths.is_empty() {
                return;
            }
            cx.background_executor()
                .spawn(async move {
                    use rmac_dock_system::Backend as _;
                    let backend = rmac_dock_system::SystemBackend;
                    for path in paths {
                        let Some(path) = path
                            .canonicalize()
                            .ok()
                            .and_then(|path| path.into_os_string().into_string().ok())
                        else {
                            continue;
                        };
                        let command = rmac_dock::StackCommand::Add(
                            rmac_shell_settings::DockStackKind::Path { path },
                        );
                        if let Err(error) = backend.update_stacks(&command).await {
                            eprintln!("could not keep the item in the Dock: {}", error.detail);
                        }
                    }
                })
                .detach();
        }

        /// A click on a stack tile: open its Fan/Grid popover, or close it
        /// if it is already open for this stack.
        fn toggle_stack_popover(
            &mut self,
            kind: rmac_shell_settings::DockStackKind,
            anchor: f32,
            cx: &mut Context<Self>,
        ) {
            if self
                .stack_popover
                .as_ref()
                .is_some_and(|popover| popover.kind == kind)
            {
                self.close_stack_popover(cx);
                return;
            }
            let Some(model) = self.status.read(cx).model().cloned() else {
                return;
            };
            let (name, display_as, view_content_as, sort_by, path) = {
                let Some(menu) = model.stack_context_menu(&kind) else {
                    return;
                };
                let rmac_dock::StackActivation::OpenDirectory { path, .. } = menu.open else {
                    // Unavailable (the stack's directory is gone): nothing
                    // to show a popover for.
                    return;
                };
                (
                    menu.name,
                    menu.display_as,
                    menu.view_content_as,
                    menu.sort_by,
                    path,
                )
            };
            self.open_stack_popover(
                kind,
                name,
                path,
                display_as,
                view_content_as,
                sort_by,
                anchor,
                cx,
            );
        }

        #[allow(clippy::too_many_arguments)]
        fn open_stack_popover(
            &mut self,
            kind: rmac_shell_settings::DockStackKind,
            name: String,
            path: PathBuf,
            display_as: rmac_shell_settings::DockStackDisplayAs,
            view_content_as: rmac_shell_settings::DockStackViewContentAs,
            sort_by: rmac_shell_settings::DockStackSortBy,
            anchor: f32,
            cx: &mut Context<Self>,
        ) {
            let sort = stack_sort_order(sort_by);
            let (events_tx, events_rx) = async_channel::bounded::<()>(1);
            let watch_path = path.clone();
            let watch = cx.spawn(async move |this, cx| {
                let watcher = blocking::unblock(move || {
                    rmac_desktop::watch(&watch_path, move || {
                        let _ = events_tx.try_send(());
                    })
                })
                .await;
                let Ok(_watcher) = watcher else {
                    return;
                };
                loop {
                    let scan_path = path.clone();
                    let result =
                        blocking::unblock(move || rmac_desktop::scan(&scan_path, sort)).await;
                    let updated = this.update(cx, |dock, cx| {
                        let Some(popover) = dock.stack_popover.as_mut() else {
                            return false;
                        };
                        popover.snapshot = result.ok();
                        cx.notify();
                        true
                    });
                    if !matches!(updated, Ok(true)) {
                        return;
                    }
                    if events_rx.recv().await.is_err() {
                        return;
                    }
                    // Coalesce a burst of filesystem events (a copy, an
                    // extraction) into one rescan, as the Desktop watcher
                    // does (`shell/bins/rmac-wallpaper`).
                    async_io::Timer::after(Duration::from_millis(75)).await;
                    while events_rx.try_recv().is_ok() {}
                }
            });
            self.stack_popover = Some(StackPopoverUi {
                kind,
                name,
                display_as,
                view_content_as,
                anchor,
                snapshot: None,
                _watch: watch,
            });
            self.context_menu = None;
            self.separator_menu = None;
            cx.notify();
        }

        fn close_stack_popover(&mut self, cx: &mut Context<Self>) {
            if self.stack_popover.take().is_some() {
                cx.notify();
            }
        }

        /// A click on an item inside the popover: open it with its default
        /// handler (the Mac opens a Stack item exactly like a Finder
        /// double-click) and close the popover, as the Mac does.
        fn open_stack_popover_item(&mut self, path: PathBuf, cx: &mut Context<Self>) {
            self.close_stack_popover(cx);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_app_launch::open_item(path.clone()).await {
                        eprintln!("could not open {path:?}: {error}");
                    }
                })
                .detach();
        }

        /// The separator's right-click menu (DOCK-02): Turn Hiding On/Off,
        /// Turn Magnification On/Off, Position on Screen ▸, Minimise Using
        /// ▸, Dock Settings….
        fn open_separator_menu(&mut self, anchor: f32, cx: &mut Context<Self>) {
            let Some(dock) = self
                .status
                .read(cx)
                .snapshot()
                .map(|snapshot| snapshot.settings.clone())
            else {
                return;
            };
            self.context_menu = None;
            self.close_stack_popover(cx);
            self.separator_menu = Some(SeparatorMenu {
                anchor,
                dock,
                submenu_open: None,
            });
            self.input_region = None;
            cx.notify();
        }

        fn close_separator_menu(&mut self, cx: &mut Context<Self>) {
            if self.separator_menu.take().is_some() {
                self.input_region = None;
                cx.notify();
            }
        }

        /// A separator-menu row that flips one `DockSettings` field
        /// directly (load, mutate, save): the same shape as
        /// `toggle_dock_hiding` and `rmac_dock_system::backend`'s
        /// `update_pins_in_store`, since these are plain settings writes
        /// the live Dock already reacts to, not model-validated commands.
        fn write_dock_settings(
            mutate: impl FnOnce(&mut rmac_shell_settings::DockSettings) + Send + 'static,
            cx: &mut Context<Self>,
        ) {
            cx.background_executor()
                .spawn(async move {
                    let result = blocking::unblock(move || {
                        let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
                        let mut settings = store.load()?.settings;
                        mutate(&mut settings.dock);
                        store.save(&settings)
                    })
                    .await;
                    if let Err(error) = result {
                        eprintln!("could not change the Dock's settings: {error}");
                    }
                })
                .detach();
        }

        fn toggle_hiding_from_menu(&mut self, cx: &mut Context<Self>) {
            self.close_separator_menu(cx);
            toggle_dock_hiding(cx);
        }

        fn toggle_magnification_from_menu(&mut self, cx: &mut Context<Self>) {
            self.close_separator_menu(cx);
            Self::write_dock_settings(|dock| dock.magnification = !dock.magnification, cx);
        }

        fn set_placement_from_menu(
            &mut self,
            placement: rmac_shell_settings::DockPlacement,
            cx: &mut Context<Self>,
        ) {
            self.close_separator_menu(cx);
            Self::write_dock_settings(move |dock| dock.placement = placement, cx);
        }

        fn set_minimize_effect_from_menu(
            &mut self,
            effect: rmac_shell_settings::DockMinimizeEffect,
            cx: &mut Context<Self>,
        ) {
            self.close_separator_menu(cx);
            Self::write_dock_settings(move |dock| dock.minimize_effect = effect, cx);
        }

        /// Dock Settings…: open System Settings at Desktop & Dock, the way
        /// `launcher-app`'s Settings surface bridge opens any other pane.
        fn open_dock_settings(&mut self, cx: &mut Context<Self>) {
            self.close_separator_menu(cx);
            cx.background_executor()
                .spawn(async move {
                    let executable = std::env::current_exe()
                        .ok()
                        .map(|path| path.with_file_name("rmac-system-settings"))
                        .unwrap_or_else(|| PathBuf::from("/usr/bin/rmac-system-settings"));
                    if let Err(error) = std::process::Command::new(executable)
                        .arg("--pane")
                        .arg("desktop-dock")
                        .spawn()
                    {
                        eprintln!("could not open Desktop & Dock settings: {error}");
                    }
                })
                .detach();
        }

        /// A press on the separator's drag/resize hit target (DOCK-03).
        fn begin_separator_drag(&mut self, position: (f32, f32), cx: &mut Context<Self>) {
            self.close_separator_menu(cx);
            self.close_stack_popover(cx);
            self.context_menu = None;
            let lift = self.lift_of(position.0, position.1);
            let size = self.tile_size;
            self.separator_drag = Some(SeparatorDragUi {
                start_lift: lift,
                start_size: size,
                preview_size: size,
            });
            self.input_region = None;
            cx.notify();
        }

        /// A pointer move while the separator is held: live-preview a new
        /// tile size (rendering only -- the compositor's reserved work
        /// area updates once the drag commits, same as any other Dock
        /// settings change).
        fn update_separator_drag(&mut self, position: (f32, f32), cx: &mut Context<Self>) {
            let lift = self.lift_of(position.0, position.1);
            let Some(drag) = self.separator_drag.as_mut() else {
                return;
            };
            let delta = lift - drag.start_lift;
            // Shelf thickness is icon_size * 1.3125 (the tile plus 2x its
            // own padding ratio, TileMetrics::new), so resizing the shelf
            // by `delta` moves the tile size by roughly that much.
            let next = (drag.start_size + delta / 1.3125).clamp(
                rmac_shell_settings::MIN_DOCK_TILE_SIZE,
                rmac_shell_settings::MAX_DOCK_TILE_SIZE,
            );
            if (next - drag.preview_size).abs() > f32::EPSILON {
                drag.preview_size = next;
                cx.notify();
            }
        }

        /// Release: commit the previewed size to settings, unless it never
        /// moved enough to count as a resize rather than a click.
        fn finish_separator_drag(&mut self, cx: &mut Context<Self>) {
            let Some(drag) = self.separator_drag.take() else {
                return;
            };
            cx.notify();
            if (drag.preview_size - drag.start_size).abs() < 0.5 {
                return;
            }
            let size = drag.preview_size;
            Self::write_dock_settings(move |dock| dock.tile_size = size, cx);
        }

        /// Keep an application dragged out of Apps (crates/app-drawer) in
        /// the Dock, at roughly the position it was dropped. This is the
        /// Dock command endpoint `drag_endpoint` documents: GPUI's Linux
        /// backend cannot start a real cross-process Wayland drag, so Apps
        /// reports its own window-local pointer position over a private
        /// socket instead, and the Dock resolves it against its catalog
        /// exactly like a `.desktop` file drop.
        fn keep_dragged_application(
            &mut self,
            app_id: String,
            fraction: f32,
            cx: &mut Context<Self>,
        ) {
            cx.background_executor()
                .spawn(async move {
                    use rmac_dock_system::Backend as _;
                    let Ok(catalog) = rmac_apps::discover() else {
                        eprintln!("could not read the installed applications");
                        return;
                    };
                    let Some(application) =
                        catalog.iter().find(|application| application.id == app_id)
                    else {
                        return;
                    };
                    let backend = rmac_dock_system::SystemBackend;
                    let pin = rmac_dock::PinCommand::Pin {
                        app_id: application.id.clone(),
                    };
                    let pinned = match backend.update_pins(&pin).await {
                        Ok(pinned) => pinned,
                        Err(error) => {
                            eprintln!(
                                "could not keep the dragged application in the Dock: {}",
                                error.detail
                            );
                            return;
                        }
                    };
                    // Insert at roughly the dropped position; clamped, since
                    // the sender's fraction is an untrusted hint.
                    let destination = ((fraction.clamp(0.0, 1.0) * pinned.len() as f32).round()
                        as usize)
                        .min(pinned.len().saturating_sub(1));
                    let move_to = rmac_dock::PinCommand::MoveTo {
                        app_id: application.id.clone(),
                        index: destination,
                    };
                    if let Err(error) = backend.update_pins(&move_to).await {
                        eprintln!(
                            "kept the dragged application, but could not place it: {}",
                            error.detail
                        );
                    }
                })
                .detach();
        }

        /// Read whether `app_id` opens at login (XDG autostart) off the UI
        /// thread; the Options row appears only once that is known.
        fn load_login_state(&mut self, app_id: &str, cx: &mut Context<Self>) {
            let source = self
                .status
                .read(cx)
                .model()
                .and_then(|model| model.context_menu(app_id))
                .and_then(|menu| match menu.show_in_finder {
                    Some(rmac_dock::ContextAction::RevealApplication { source, .. }) => {
                        Some(source)
                    }
                    _ => None,
                });
            let Some(desktop_entry) = source else {
                return;
            };
            let Some(id) = desktop_entry
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                return;
            };
            let invoker = rmac_dock::presentation::EntryId::Application(app_id.to_owned());
            let lookup = id.clone();
            let task = cx.background_executor().spawn(async move {
                rmac_login_items_linux::snapshot().ok().map(|snapshot| {
                    snapshot
                        .items
                        .iter()
                        .any(|item| item.id == lookup && item.enabled)
                })
            });
            cx.spawn(async move |this, cx| {
                let Some(enabled) = task.await else {
                    return;
                };
                let _ = this.update(cx, |this, cx| {
                    if let Some(menu) = this.context_menu.as_mut() {
                        if menu.session.invoker() == &invoker {
                            menu.login = Some(LoginOption {
                                desktop_entry,
                                id,
                                enabled,
                            });
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
        }

        /// Options ▸ Open at Login: add or enable the app's XDG autostart
        /// entry, or disable it.
        fn toggle_open_at_login(&mut self, login: LoginOption, cx: &mut Context<Self>) {
            self.context_menu = None;
            self.input_region = None;
            cx.notify();
            cx.background_executor()
                .spawn(async move {
                    let result = if login.enabled {
                        rmac_login_items_linux::set_enabled(&login.id, false).map(|_| ())
                    } else {
                        match rmac_login_items_linux::snapshot() {
                            Ok(snapshot)
                                if snapshot.items.iter().any(|item| item.id == login.id) =>
                            {
                                rmac_login_items_linux::set_enabled(&login.id, true).map(|_| ())
                            }
                            Ok(_) => {
                                rmac_login_items_linux::prepare_add_source(&login.desktop_entry)
                                    .and_then(|preview| {
                                        rmac_login_items_linux::add_source(&preview)
                                    })
                                    .map(|_| ())
                            }
                            Err(error) => Err(error),
                        }
                    };
                    if result.is_err() {
                        eprintln!("could not change Open at Login");
                    }
                })
                .detach();
        }

        /// A launch the Dock starts bounces its tile until a window appears.
        fn note_launch(&mut self, app_id: &str) {
            let now = self.now_ms();
            self.bounces.launch(app_id, now);
        }

        fn schedule_hide(&mut self, cx: &mut Context<Self>) {
            self.hide_generation = self.hide_generation.saturating_add(1);
            let generation = self.hide_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(rmac_dock::motion::HIDE_DELAY_MS))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.hide_generation == generation
                        && !this.pointer_inside
                        && this.keyboard.is_none()
                    {
                        this.hovered_item = None;
                        this.set_hidden(true, cx);
                    }
                });
            })
            .detach();
        }

        /// The pointer reached the edge of a hidden Dock: it slides back in
        /// once the pointer has stayed there for the measured 200 ms.
        fn schedule_reveal(&mut self, cx: &mut Context<Self>) {
            self.hide_generation = self.hide_generation.saturating_add(1);
            let generation = self.hide_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(rmac_dock::motion::REVEAL_PRESSURE_MS))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.hide_generation == generation && this.pointer_inside {
                        this.set_hidden(false, cx);
                    }
                });
            })
            .detach();
        }

        /// Start the auto-hide slide and hide or show the shelf material.
        fn set_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
            if self.hidden == hidden {
                return;
            }
            self.hidden = hidden;
            self.hide_slide = Some((self.now_ms(), hidden));
            self.input_region = None;
            let display_id = self.display_id;
            self.status.update(cx, |status, cx| {
                let changed = if hidden {
                    status.hidden_displays.insert(display_id)
                } else {
                    status.hidden_displays.remove(&display_id)
                };
                if changed {
                    cx.notify();
                }
            });
            cx.notify();
        }

        /// 0 when the shelf rests in place, 1 when it is fully out of view.
        fn hide_progress(&mut self, now: u64) -> f32 {
            let resting = if self.hidden { 1.0 } else { 0.0 };
            let Some((started, hiding)) = self.hide_slide else {
                return resting;
            };
            let elapsed = now.saturating_sub(started);
            if elapsed >= rmac_dock::motion::AUTOHIDE_SLIDE_MS {
                self.hide_slide = None;
                return resting;
            }
            let t = rmac_dock::reorder::ease_out(elapsed, rmac_dock::motion::AUTOHIDE_SLIDE_MS);
            if hiding {
                t
            } else {
                1.0 - t
            }
        }

        fn model_snapshot(&self, cx: &Context<Self>) -> Option<rmac_dock::Model> {
            self.status
                .read(cx)
                .snapshot()
                .map(|snapshot| snapshot.model.clone())
        }

        /// Primary activation for one Dock tile: ⌘-click reveals in Files,
        /// otherwise launch/focus through the system authority. As on the
        /// Mac, ⌥-click also hides the application being left, and ⌥⌘-click
        /// hides every other application.
        fn activate_entry(
            &mut self,
            app_id: &str,
            modifiers: gpui::Modifiers,
            cx: &mut Context<Self>,
        ) {
            let hide = modifiers.alt.then(|| {
                let status = self.status.read(cx);
                let model = status.model()?;
                let action = if modifiers.platform {
                    model.context_menu(app_id)?.hide_others?
                } else {
                    let front = model
                        .items
                        .iter()
                        .find(|item| item.active && item.id != app_id)?;
                    model.context_menu(&front.id)?.hide?
                };
                model.authorizes_context_action(&action).then_some(action)
            });
            if modifiers.platform && !modifiers.alt {
                let reveal = {
                    let status = self.status.read(cx);
                    status
                        .model()
                        .and_then(|model| model.context_menu(app_id))
                        .and_then(|menu| menu.show_in_finder)
                        .filter(|action| {
                            status
                                .model()
                                .is_some_and(|model| model.authorizes_context_action(action))
                        })
                };
                if let Some(action) = reveal {
                    self.dispatch_action(rmac_dock::menu::Action::Context(action), cx);
                }
            } else {
                let launches = self.status.read(cx).model().is_some_and(|model| {
                    matches!(model.activate(app_id), rmac_dock::Activation::Launch { .. })
                });
                if launches {
                    self.note_launch(app_id);
                }
                self.dispatch_action(
                    rmac_dock::menu::Action::ActivateEntry(
                        rmac_dock::presentation::EntryId::Application(app_id.to_owned()),
                    ),
                    cx,
                );
            }
            if let Some(action) = hide.flatten() {
                self.dispatch_action(rmac_dock::menu::Action::Context(action), cx);
            }
        }

        /// Empty Trash from the Dock menu: bind the exact Trash contents on a
        /// worker, then ask for confirmation before anything is deleted.
        fn review_trash(&mut self, action: rmac_dock::menu::Action, cx: &mut Context<Self>) {
            let pending = self.status.update(cx, |status, _| {
                let model = status.model()?;
                match rmac_dock_system::dispatch::prepare(model, action).ok()? {
                    rmac_dock_system::dispatch::Preparation::TrashReview(review) => {
                        review.begin(&mut status.actions).ok()
                    }
                    _ => None,
                }
            });
            let Some(pending) = pending else {
                eprintln!("the Trash changed before it could be emptied");
                return;
            };
            let work = cx
                .background_executor()
                .spawn(async move { pending.run_blocking(&rmac_places_system::SystemBackend) });
            cx.spawn(async move |this, cx| {
                let completion = work.await;
                let _ = this.update(cx, |this, cx| {
                    let (result, _) = this
                        .status
                        .update(cx, |status, _| completion.apply(&mut status.actions));
                    match result {
                        Ok(reviewed) if reviewed.item_count() > 0 => {
                            this.trash_review = Some(reviewed);
                            this.input_region = None;
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("{error}"),
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        /// The confirmation alert's answer. Only an affirmative answer creates
        /// the capability that deletes the reviewed items.
        fn finish_trash_review(&mut self, confirmed: bool, cx: &mut Context<Self>) {
            let Some(reviewed) = self.trash_review.take() else {
                return;
            };
            self.input_region = None;
            cx.notify();
            let Some(confirmed) = reviewed.confirm(confirmed) else {
                return;
            };
            let pending = match self
                .status
                .update(cx, |status, _| confirmed.begin(&mut status.actions))
            {
                Ok(pending) => pending,
                Err(error) => {
                    eprintln!("could not empty the Trash: {error}");
                    return;
                }
            };
            let work = cx
                .background_executor()
                .spawn(async move { pending.run_blocking(&rmac_places_system::SystemBackend) });
            cx.spawn(async move |this, cx| {
                let completion = work.await;
                let _ = this.update(cx, |this, cx| {
                    let (result, _) = this
                        .status
                        .update(cx, |status, _| completion.apply(&mut status.actions));
                    if let Err(error) = result {
                        eprintln!("{error}");
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        /// ⌃F3: focus the first tile and take the keyboard through the
        /// invisible focus surface (see `KeyboardMode`).
        fn begin_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.keyboard.is_some() || self.trash_review.is_some() {
                return;
            }
            let (targets, previous_window) = {
                let status = self.status.read(cx);
                let Some(snapshot) = status.snapshot() else {
                    return;
                };
                (keyboard_targets(snapshot), snapshot.compositor.focus.window)
            };
            let Some(navigator) = rmac_dock::keyboard::Navigator::new(targets.len()) else {
                return;
            };
            self.context_menu = None;
            self.tile_drag = None;
            self.pressed = None;
            let announcement = Announcement {
                role: Role::Button,
                label: targets[0].accessible.clone().into(),
                key: 0,
            };
            let dock = cx.entity().downgrade();
            let options = WindowOptions {
                titlebar: None,
                focus: true,
                show: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: Size::new(px(1.0), px(1.0)),
                })),
                display_id: window.display(cx).map(|display| display.id()),
                app_id: Some("dev.rmac.DockKeyboard".to_owned()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: "rmac-dock-keyboard".to_owned(),
                    layer: Layer::Overlay,
                    keyboard_interactivity: KeyboardInteractivity::Exclusive,
                    ..Default::default()
                }),
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            };
            let surface = match cx.open_window(options, move |window, cx| {
                cx.new(|cx| DockKeyboard::new(dock, announcement, window, cx))
            }) {
                Ok(surface) => surface,
                Err(error) => {
                    eprintln!("could not move keyboard focus to the Dock: {error}");
                    return;
                }
            };
            self.keyboard = Some(KeyboardMode {
                navigator,
                surface,
                previous_window,
            });
            // A hidden Dock slides in while it has the keyboard.
            self.hide_generation = self.hide_generation.saturating_add(1);
            self.set_hidden(false, cx);
            self.input_region = None;
            cx.notify();
        }

        /// Leave keyboard mode. `restore` (Esc) hands the keyboard back to
        /// the window that had it before ⌃F3; activating a tile does not,
        /// because the activation focuses its own window.
        fn leave_keyboard(&mut self, restore: bool, cx: &mut Context<Self>) {
            let Some(mode) = self.keyboard.take() else {
                return;
            };
            // Fails harmlessly when the focus surface is the caller; it then
            // removes itself.
            let _ = mode.surface.update(cx, |surface, window, _| {
                surface.closing = true;
                window.remove_window();
            });
            if let (true, Some(window)) = (restore, mode.previous_window) {
                cx.spawn(async move |_, _| {
                    let action = rmac_compositor::Action::FocusWindow { window };
                    if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                        eprintln!("could not return focus from the Dock: {error:?}");
                    }
                })
                .detach();
            }
            self.input_region = None;
            let autohide = self
                .status
                .read(cx)
                .snapshot()
                .is_some_and(|snapshot| snapshot.settings.autohide)
                || self.fullscreen;
            if autohide && !self.pointer_inside && !self.overview_visible {
                self.schedule_hide(cx);
            }
            cx.notify();
        }

        /// One key from the focus surface. Returns what the surface should
        /// expose next, or `None` once keyboard mode has ended.
        fn keyboard_key(
            &mut self,
            key: &str,
            shift: bool,
            alt: bool,
            cx: &mut Context<Self>,
        ) -> Option<Announcement> {
            let targets = {
                let status = self.status.read(cx);
                status.snapshot().map(keyboard_targets).unwrap_or_default()
            };
            let mode = self.keyboard.as_mut()?;
            if !mode.navigator.set_len(targets.len()) {
                self.leave_keyboard(true, cx);
                return None;
            }
            if self.context_menu.is_some() {
                return self.keyboard_menu_key(key, alt, &targets, cx);
            }
            let index = mode.navigator.index();
            let edge = dock_edge(self.placement);
            let outcome = mode
                .navigator
                .handle(rmac_dock::keyboard::Key::from_name(key, shift, edge));
            match outcome {
                rmac_dock::keyboard::Outcome::Moved => cx.notify(),
                rmac_dock::keyboard::Outcome::Unchanged => {}
                rmac_dock::keyboard::Outcome::Activate => {
                    let target = targets[index].clone();
                    self.leave_keyboard(false, cx);
                    self.activate_target(&target, cx);
                    return None;
                }
                rmac_dock::keyboard::Outcome::OpenMenu => {
                    self.open_target_menu(&targets[index], cx);
                }
                rmac_dock::keyboard::Outcome::Dismiss => {
                    self.leave_keyboard(true, cx);
                    return None;
                }
            }
            self.keyboard_announcement(&targets)
        }

        /// ↑ ↓ Return Esc inside a Dock menu opened from the keyboard.
        fn keyboard_menu_key(
            &mut self,
            key: &str,
            alt: bool,
            targets: &[KeyTarget],
            cx: &mut Context<Self>,
        ) -> Option<Announcement> {
            use rmac_dock::menu::{Effect, KeyCommand};
            let command = match key {
                "up" => Some(KeyCommand::ArrowUp),
                "down" => Some(KeyCommand::ArrowDown),
                "home" => Some(KeyCommand::Home),
                "end" => Some(KeyCommand::End),
                "enter" | "space" if alt => Some(KeyCommand::AlternateReturn),
                "enter" | "space" => Some(KeyCommand::Return),
                "escape" => Some(KeyCommand::Escape),
                _ => None,
            };
            if let Some(command) = command {
                let menu = self.context_menu.as_mut()?;
                match menu.session.handle_key(command) {
                    Effect::None => {}
                    Effect::SelectionChanged => {
                        // Options ▸ rows live in the submenu: show it while
                        // one of them is selected.
                        let rows = menu.session.rows();
                        menu.submenu_open = menu
                            .session
                            .selected()
                            .and_then(|id| rows.iter().find(|row| &row.id == id))
                            .is_some_and(|row| row.submenu.is_some());
                        cx.notify();
                    }
                    Effect::Activate { action, .. } => {
                        self.context_menu = None;
                        self.input_region = None;
                        self.leave_keyboard(false, cx);
                        self.run_menu_action(action, cx);
                        return None;
                    }
                    Effect::Dismissed { .. } => {
                        // Back to the tile, still in keyboard mode.
                        self.context_menu = None;
                        self.input_region = None;
                        cx.notify();
                    }
                }
            }
            self.keyboard_announcement(targets)
        }

        fn keyboard_announcement(&self, targets: &[KeyTarget]) -> Option<Announcement> {
            let mode = self.keyboard.as_ref()?;
            if let Some(menu) = &self.context_menu {
                let rows = menu.session.rows();
                let selected = menu
                    .session
                    .selected()
                    .and_then(|id| rows.iter().position(|row| &row.id == id));
                return Some(match selected {
                    Some(index) => Announcement {
                        role: Role::MenuItem,
                        label: rows[index].accessible_label.clone().into(),
                        key: MENU_ANNOUNCEMENT_KEY + 1 + index,
                    },
                    None => Announcement {
                        role: Role::Menu,
                        label: menu.session.accessible_title().to_owned().into(),
                        key: MENU_ANNOUNCEMENT_KEY,
                    },
                });
            }
            let index = mode.navigator.index();
            targets.get(index).map(|target| Announcement {
                role: Role::Button,
                label: target.accessible.clone().into(),
                key: index,
            })
        }

        /// Return / Space on a focused tile: the same paths a click runs.
        fn activate_target(&mut self, target: &KeyTarget, cx: &mut Context<Self>) {
            match &target.id {
                rmac_dock::presentation::EntryId::Application(app_id) => {
                    let app_id = app_id.clone();
                    self.activate_entry(&app_id, gpui::Modifiers::default(), cx);
                }
                id => self.dispatch_action(rmac_dock::menu::Action::ActivateEntry(id.clone()), cx),
            }
        }

        /// ↑ on a focused tile: its Dock menu, as a right-click opens it.
        fn open_target_menu(&mut self, target: &KeyTarget, cx: &mut Context<Self>) {
            let session = {
                let status = self.status.read(cx);
                status.model().and_then(|model| match &target.id {
                    rmac_dock::presentation::EntryId::Application(app_id) => model
                        .context_menu(app_id)
                        .and_then(|menu| rmac_dock::menu::Session::context(&menu).ok()),
                    rmac_dock::presentation::EntryId::Special(kind) => model
                        .special_context_menu(*kind)
                        .and_then(|menu| rmac_dock::menu::Session::special(&menu).ok()),
                    _ => None,
                })
            };
            let Some(session) = session else {
                return;
            };
            self.context_menu = Some(DockMenu {
                anchor: target.center,
                session,
                submenu_open: false,
                login: None,
            });
            self.input_region = None;
            if let rmac_dock::presentation::EntryId::Application(app_id) = &target.id {
                self.load_login_state(app_id, cx);
            }
            cx.notify();
        }

        /// Run a chosen Dock menu command, re-checking that it still
        /// matches the model.
        fn run_menu_action(&mut self, action: rmac_dock::menu::Action, cx: &mut Context<Self>) {
            let authorized = match &action {
                rmac_dock::menu::Action::Context(action) => self
                    .status
                    .read(cx)
                    .model()
                    .is_some_and(|model| model.authorizes_context_action(action)),
                // Stack commands are re-validated by the dispatch layer
                // (`prepare_stack_context_action`), like the Trash's.
                rmac_dock::menu::Action::ActivateEntry(_)
                | rmac_dock::menu::Action::SpecialContext(_)
                | rmac_dock::menu::Action::StackContext(_) => true,
            };
            if let rmac_dock::menu::Action::Context(rmac_dock::ContextAction::LaunchNew {
                app_id,
                ..
            }) = &action
            {
                if authorized {
                    self.note_launch(app_id);
                }
            }
            if authorized {
                self.dispatch_action(action, cx);
            } else {
                eprintln!("the selected Dock command is no longer current");
            }
            cx.notify();
        }

        fn dispatch_action(&mut self, action: rmac_dock::menu::Action, cx: &mut Context<Self>) {
            if matches!(action, rmac_dock::menu::Action::SpecialContext(_)) {
                self.review_trash(action, cx);
                return;
            }
            let pending = match self.status.update(cx, |status, _| {
                let model = status
                    .model()
                    .ok_or_else(|| "runtime is not ready".to_owned())?;
                let preparation = rmac_dock_system::dispatch::prepare(model, action)
                    .map_err(|error| error.to_string())?;
                let prepared = match preparation {
                    rmac_dock_system::dispatch::Preparation::Ready(prepared) => prepared,
                    rmac_dock_system::dispatch::Preparation::NoAction => return Ok(None),
                    rmac_dock_system::dispatch::Preparation::TrashReview(_) => {
                        return Err("Empty Trash requires the confirmation sheet".to_owned());
                    }
                };
                match prepared.begin(&mut status.actions) {
                    Ok(pending) => Ok(Some(pending)),
                    Err(rmac_dock_system::interaction::BeginError::Busy { .. }) => Ok(None),
                    Err(error) => Err(error.to_string()),
                }
            }) {
                Ok(Some(pending)) => pending,
                Ok(None) => return,
                Err(error) => {
                    eprintln!("could not begin Dock action: {error}");
                    return;
                }
            };
            let Some(request_id) = next_activation_id() else {
                self.status.update(cx, |status, cx| {
                    if pending.cancel(&mut status.actions).visible {
                        cx.notify();
                    }
                });
                eprintln!("could not run Dock action: activation IDs exhausted");
                return;
            };
            cx.notify();
            let status = self.status.clone();
            let execution = cx.background_executor().spawn(async move {
                let backend = rmac_dock_system::SystemBackend;
                pending.run(request_id, &backend).await
            });
            cx.spawn(async move |this, cx| {
                let completion = execution.await;
                let failed_launch = status.update(cx, |status, cx| {
                    let (result, transition) = completion.apply(&mut status.actions);
                    if let Err(error) = &result {
                        eprintln!("{error}");
                        if let Some(feedback) = transition.snapshot.feedback.last() {
                            eprintln!("{}", feedback.accessible_message());
                        }
                    }
                    if transition.visible {
                        cx.notify();
                    }
                    match result {
                        Err(error) if error.operation == rmac_dock_system::Operation::Launch => {
                            Some((error.app_id, error.kind))
                        }
                        _ => None,
                    }
                });
                if let Some((app_id, kind)) = failed_launch {
                    let _ = this.update(cx, |dock, cx| dock.launch_failed(&app_id, kind, cx));
                }
            })
            .detach();
        }

        /// A launch the Dock started failed: the tile stops bouncing and a
        /// notice names the application, as on the Mac, instead of the click
        /// silently doing nothing.
        fn launch_failed(
            &mut self,
            app_id: &str,
            kind: rmac_dock_system::FailureKind,
            cx: &mut Context<Self>,
        ) {
            let now = self.now_ms();
            self.bounces.launch_failed(app_id, now);
            cx.notify();
            let name = self
                .status
                .read(cx)
                .model()
                .and_then(|model| model.items.iter().find(|item| item.id == app_id))
                .map_or_else(|| app_id.to_owned(), |item| item.name.clone());
            let (summary, body) = rmac_dock_system::launch_failure_notice(&name, kind);
            post_notice(summary, body, cx);
        }
    }

    /// A transient notice through the session's notification server.
    fn post_notice(summary: String, body: String, cx: &mut Context<Dock>) {
        cx.background_executor()
            .spawn(async move {
                let shown = summary.clone();
                let result = blocking::unblock(move || -> zbus::Result<()> {
                    let connection = zbus::blocking::Connection::session()?;
                    let hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> =
                        std::collections::HashMap::new();
                    connection.call_method(
                        Some("org.freedesktop.Notifications"),
                        "/org/freedesktop/Notifications",
                        Some("org.freedesktop.Notifications"),
                        "Notify",
                        &(
                            "Dock",
                            0_u32,
                            "dialog-warning",
                            summary.as_str(),
                            body.as_str(),
                            Vec::<&str>::new(),
                            hints,
                            -1_i32,
                        ),
                    )?;
                    Ok(())
                })
                .await;
                if let Err(error) = result {
                    eprintln!("could not show \"{shown}\": {error}");
                }
            })
            .detach();
    }

    impl Render for Dock {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            // `on_a11y_action` listeners run outside `Context<Self>` (they
            // get `&mut App`, not `&mut Context<Self>`, since AT-SPI can
            // dispatch to any node at any time); an owned entity handle
            // lets them reach the same activation path a mouse click uses.
            let a11y_entity = cx.entity();
            let launcher = self.status.read(cx).launcher.clone();
            let (dock_settings, model, mut entries, content, keyboard_focus) = {
                let status = self.status.read(cx);
                let snapshot = status
                    .snapshot()
                    .expect("a Dock surface is opened only after runtime readiness");
                // ⌃F3: the focused tile darkens like a menu-open tile and
                // shows its name bubble (design-lab/dock.html, scene 3).
                let keyboard_focus = self.keyboard.as_ref().and_then(|mode| {
                    keyboard_targets(snapshot)
                        .into_iter()
                        .nth(mode.navigator.index())
                });
                (
                    snapshot.settings.clone(),
                    snapshot.model.clone(),
                    snapshot.content.applications.clone(),
                    snapshot.content.clone(),
                    keyboard_focus,
                )
            };
            let keyboard_focus_id = keyboard_focus.as_ref().map(|target| target.id.clone());
            self.content = content;
            // DOCK-01: the settings-driven resting tile size, live-reactive
            // like every other Dock setting; a separator drag (DOCK-03)
            // previews a different size on top of it without touching this.
            self.tile_size = dock_settings.tile_size;
            let metrics = self.metrics();
            // Launch and attention bounces follow the authoritative model:
            // a window appearing ends a launch, urgency asks for attention.
            let now = self.now_ms();
            let running_apps: std::collections::BTreeSet<String> = model
                .items
                .iter()
                .filter(|item| item.running)
                .map(|item| item.id.clone())
                .collect();
            // Attention comes from niri urgency or the app's LauncherEntry.
            let attention_apps: std::collections::BTreeSet<String> = model
                .items
                .iter()
                .filter(|item| {
                    item.running
                        && !item.active
                        && (item.urgent || launcher.get(&item.id).is_some_and(|entry| entry.urgent))
                })
                .map(|item| item.id.clone())
                .collect();
            self.bounces.reconcile(&running_apps, &attention_apps, now);
            if self.bounces.is_animating(now) {
                window.request_animation_frame();
            }
            let effective_autohide = dock_settings.autohide || self.fullscreen;
            let visibility_policy = (effective_autohide, self.overview_visible);
            if self.visibility_policy != Some(visibility_policy) {
                self.visibility_policy = Some(visibility_policy);
                if self.overview_visible || !effective_autohide {
                    self.hide_generation = self.hide_generation.saturating_add(1);
                    self.set_hidden(false, cx);
                } else if !self.hidden {
                    self.schedule_hide(cx);
                }
            }
            let minimized_entries: Vec<rmac_dock::presentation::Entry> = self
                .content
                .places
                .iter()
                .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Minimized(_)))
                .cloned()
                .collect();
            // Folder/file stacks (§ folder/file stacks left of the Trash):
            // between the minimized group and Trash, newest-kept first, the
            // order `rmac_dock::presentation::ShelfContent::project` gives.
            let stack_entries: Vec<rmac_dock::presentation::Entry> = self
                .content
                .places
                .iter()
                .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Stack(_)))
                .cloned()
                .collect();
            let trash = model
                .special_items
                .iter()
                .find(|item| item.kind == rmac_dock::SpecialItemKind::Trash);
            let trash_full = trash
                .and_then(|trash| trash.item_count)
                .is_some_and(|count| count > 0);
            let trash_label = trash_accessible_label(&model);
            let trash_available = model.activate_special(rmac_dock::SpecialItemKind::Trash)
                == rmac_dock::SpecialActivation::OpenTrash;
            let pinned_count = model.items.iter().take_while(|item| item.pinned).count();
            // Kept apps are drawn in the drag preview order, or in the order a
            // drop asked for until the settings watcher confirms it.
            let model_pinned: Vec<String> = model
                .items
                .iter()
                .take_while(|item| item.pinned)
                .map(|item| item.id.clone())
                .collect();
            if self.pending_pins.as_ref().is_some_and(|(order, started)| {
                *order == model_pinned || now.saturating_sub(*started) > 2_000
            }) {
                self.pending_pins = None;
            }
            let shown_pinned: Option<Vec<String>> = match (&self.tile_drag, &self.pending_pins) {
                (Some(ui), _) if ui.drag.is_active() && ui.pinned == model_pinned => Some(
                    ui.drag
                        .preview_order()
                        .into_iter()
                        .map(|index| ui.pinned[index].clone())
                        .collect(),
                ),
                (_, Some((order, _))) => Some(order.clone()),
                _ => None,
            };
            if self
                .tile_drag
                .as_ref()
                .is_some_and(|ui| ui.pinned != model_pinned)
            {
                // The kept apps changed under the drag: abandon it.
                self.tile_drag = None;
                self.slides.clear();
            }
            let mut pinned_count = pinned_count;
            if let Some(order) = shown_pinned {
                let unpinned = entries.split_off(pinned_count.min(entries.len()));
                let mut kept: Vec<_> = order
                    .iter()
                    .filter_map(|id| {
                        entries.iter().find(|entry| {
                            matches!(&entry.id, rmac_dock::presentation::EntryId::Application(app_id) if app_id == id)
                        })
                    })
                    .cloned()
                    .collect();
                pinned_count = kept.len();
                kept.extend(unpinned);
                entries = kept;
            }
            self.slides.retain(|_, (_, started)| {
                now.saturating_sub(*started) < rmac_dock::reorder::SLIDE_MS
            });
            if self.removing.as_ref().is_some_and(|removed| {
                now.saturating_sub(removed.started_ms) >= rmac_dock::reorder::REMOVE_FADE_MS
            }) {
                self.removing = None;
            }
            if !self.slides.is_empty() || self.removing.is_some() {
                window.request_animation_frame();
            }
            let separates_running = pinned_count > 0 && pinned_count < entries.len();
            let separator_count = usize::from(separates_running) + usize::from(!entries.is_empty());
            // Minimized tiles, then stacks, sit between the application
            // group and Trash.
            let item_count = entries.len() + minimized_entries.len() + stack_entries.len() + 1;
            let child_count = item_count + separator_count;
            let window_size = window.bounds().size;
            let surface_width = f32::from(window_size.width);
            let surface_height = f32::from(window_size.height);
            let horizontal = self.placement == rmac_shell_settings::DockPlacement::Bottom;
            let axis = if horizontal {
                surface_width
            } else {
                surface_height
            };
            let shelf_extent = metrics.icon_size * item_count as f32
                + metrics.separator_slot * separator_count as f32
                + metrics.icon_gap * child_count.saturating_sub(1) as f32
                + 2.0 * metrics.shelf_padding;
            let shelf_start = (axis - shelf_extent) / 2.0;
            self.shelf_start = shelf_start;
            self.surface_size = (surface_width, surface_height);
            let trash_center = shelf_extent - metrics.shelf_padding - metrics.icon_size / 2.0;
            let menu_anchor = self.context_menu.as_ref().map(|menu| menu.anchor);
            // (menu start along the Dock axis, pointer tip from that start,
            // menu width)
            let menu_geometry = self.context_menu.as_ref().map(|menu| {
                let width = dock_menu_width(menu, window);
                // macOS hangs the menu to the right of the tile: the pointer
                // sits 26.5 from its left edge unless the output edge clamps it.
                let tip = shelf_start + menu.anchor;
                let start = (tip - MENU_ANCHOR_INSET).clamp(8.0, (axis - width - 8.0).max(8.0));
                (start, tip - start, width)
            });
            let dragging = self
                .tile_drag
                .as_ref()
                .is_some_and(|ui| ui.drag.is_active())
                || self.separator_drag.is_some();
            // Keyboard mode also captures the next click anywhere, which
            // ends it, so the invisible focus surface can never keep the
            // keyboard after the user has moved on.
            let modal = menu_geometry.is_some()
                || self.trash_review.is_some()
                || dragging
                || self.keyboard.is_some()
                || self.separator_menu.is_some()
                || self.stack_popover.is_some();
            let input_region = (shelf_start, shelf_extent, self.hidden, modal);
            if self.input_region != Some(input_region) {
                let shelf_bounds = match (self.placement, self.hidden) {
                    (rmac_shell_settings::DockPlacement::Bottom, true) => Bounds {
                        origin: point(px(shelf_start), px(surface_height - 2.0)),
                        size: Size::new(px(shelf_extent), px(2.0)),
                    },
                    (rmac_shell_settings::DockPlacement::Left, true) => Bounds {
                        origin: point(px(0.0), px(shelf_start)),
                        size: Size::new(px(2.0), px(shelf_extent)),
                    },
                    (rmac_shell_settings::DockPlacement::Right, true) => Bounds {
                        origin: point(px(f32::from(window_size.width) - 2.0), px(shelf_start)),
                        size: Size::new(px(2.0), px(shelf_extent)),
                    },
                    (rmac_shell_settings::DockPlacement::Bottom, false) => Bounds {
                        origin: point(px(shelf_start), px(surface_height - metrics.exclusive_zone)),
                        size: Size::new(px(shelf_extent), px(metrics.exclusive_zone)),
                    },
                    (rmac_shell_settings::DockPlacement::Left, false) => Bounds {
                        origin: point(px(0.0), px(shelf_start)),
                        size: Size::new(px(metrics.exclusive_zone), px(shelf_extent)),
                    },
                    (rmac_shell_settings::DockPlacement::Right, false) => Bounds {
                        origin: point(
                            px(f32::from(window_size.width) - metrics.exclusive_zone),
                            px(shelf_start),
                        ),
                        size: Size::new(px(metrics.exclusive_zone), px(shelf_extent)),
                    },
                };
                // A native menu owns pointer interaction until it is
                // dismissed. Capture one click across the output while the
                // Dock menu is open; otherwise only the visible shelf is
                // interactive and this transparent layer is inert.
                let regions = if modal {
                    vec![Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: Size::new(window_size.width, window_size.height),
                    }]
                } else {
                    vec![shelf_bounds]
                };
                window.set_input_region(Some(&regions));
                self.input_region = Some(input_region);
            }
            let tooltip_item = keyboard_focus
                .as_ref()
                .map(|target| (target.center, target.name.clone()))
                .or_else(|| self.hovered_item.clone());
            let tooltip = (!self.hidden
                && self.context_menu.is_none()
                && self.separator_menu.is_none()
                && self.stack_popover.is_none()
                && !dragging)
                .then_some(tooltip_item.as_ref())
                .flatten()
                .map(|(relative_center, label)| {
                    let icon_center = shelf_start + *relative_center;
                    let tooltip_bottom = metrics.tooltip_bottom.max(
                        magnified_icon_size(
                            *relative_center,
                            Some(*relative_center),
                            &dock_settings,
                        ) + 36.0,
                    );
                    let tooltip = div()
                        .absolute()
                        .w(px(TOOLTIP_WIDTH))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded(px(tokens::tooltip_radius()))
                                .bg(rgba(tokens::tooltip_tint()))
                                .border_1()
                                .border_color(rgba(tokens::light_border()))
                                .shadow_lg()
                                .text_sm()
                                .text_color(rgba(tokens::primary_text()))
                                .child(label.clone()),
                        );
                    match self.placement {
                        rmac_shell_settings::DockPlacement::Bottom => tooltip
                            .left(px(icon_center - TOOLTIP_WIDTH / 2.0))
                            .bottom(px(tooltip_bottom)),
                        rmac_shell_settings::DockPlacement::Left => tooltip
                            .left(px(metrics.exclusive_zone + 8.0))
                            .top(px(icon_center - 18.0)),
                        rmac_shell_settings::DockPlacement::Right => tooltip
                            .right(px(metrics.exclusive_zone + 8.0))
                            .top(px(icon_center - 18.0)),
                    }
                });
            let autohide = effective_autohide;
            let hide_progress = self.hide_progress(now);
            if self.hide_slide.is_some() {
                window.request_animation_frame();
            }
            let root = div()
                .id(format!("dock-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("Dock")
                .size_full()
                .relative()
                .flex()
                .font_features(rmac_shell_ui::tabular_font_features())
                // Any click leaves keyboard mode first; a click on a tile
                // then does what it always does. niri gives the keyboard
                // back to its focused window when the focus surface goes.
                .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                    if this.keyboard.is_some() {
                        this.leave_keyboard(false, cx);
                    }
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    let mut changed = this.context_menu.take().is_some();
                    changed |= this.separator_menu.take().is_some();
                    changed |= this.stack_popover.take().is_some();
                    if changed {
                        this.input_region = None;
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                        if this.pressed.take().is_some() {
                            cx.notify();
                        }
                        this.finish_tile_drag(event.modifiers, cx);
                        this.finish_separator_drag(cx);
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.tile_drag.is_some() {
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.update_tile_drag(
                                (f32::from(event.position.x), f32::from(event.position.y)),
                                cx,
                            );
                        } else {
                            // The release happened somewhere we never saw.
                            this.tile_drag = None;
                            this.slides.clear();
                            this.input_region = None;
                            cx.notify();
                        }
                    }
                    if this.separator_drag.is_some() {
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.update_separator_drag(
                                (f32::from(event.position.x), f32::from(event.position.y)),
                                cx,
                            );
                        } else {
                            this.finish_separator_drag(cx);
                        }
                    }
                    if this.option_held != event.modifiers.alt {
                        this.option_held = event.modifiers.alt;
                        if this.context_menu.is_some() {
                            cx.notify();
                        }
                    }
                }))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.pointer_inside = *hovered;
                    this.hide_generation = this.hide_generation.saturating_add(1);
                    if *hovered {
                        if this.hidden {
                            this.schedule_reveal(cx);
                        }
                    } else if autohide {
                        this.schedule_hide(cx);
                    }
                }))
                .children(tooltip);
            let root = match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => root
                    .items_end()
                    .justify_center()
                    .pb(px(metrics.shelf_bottom_margin)),
                rmac_shell_settings::DockPlacement::Left => root
                    .items_start()
                    .justify_center()
                    .pl(px(metrics.shelf_bottom_margin)),
                rmac_shell_settings::DockPlacement::Right => root
                    .items_end()
                    .justify_center()
                    .pr(px(metrics.shelf_bottom_margin)),
            };
            let shelf = div()
                .flex()
                .gap(px(metrics.icon_gap))
                .p(px(metrics.shelf_padding))
                .rounded(px(tokens::dock_shelf_radius(metrics.icon_size)))
                .bg(rgba(tokens::transparent()))
                .relative()
                .opacity(if hide_progress >= 1.0 { 0.0 } else { 1.0 });
            // Auto-hide slides the shelf off its screen edge.
            let slide = hide_progress * metrics.exclusive_zone;
            let shelf = match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => shelf.top(px(slide)),
                rmac_shell_settings::DockPlacement::Left => shelf.left(px(-slide)),
                rmac_shell_settings::DockPlacement::Right => shelf.left(px(slide)),
            };
            let mut shelf = if horizontal {
                shelf.items_end()
            } else {
                shelf.flex_col().items_center()
            };
            // An application (.desktop entry) dropped anywhere else on the
            // shelf is kept; a folder or other file becomes a stack (§
            // folder/file stacks left of the Trash).
            shelf = shelf.on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let (desktop_entries, stack_paths): (Vec<_>, Vec<_>) =
                    paths.paths().iter().cloned().partition(|path| {
                        path.extension()
                            .is_some_and(|extension| extension == "desktop")
                    });
                this.keep_dropped_applications(desktop_entries, cx);
                this.keep_dropped_stacks(stack_paths, cx);
            }));
            let minimized_children: Vec<gpui::AnyElement> = minimized_entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let window = match &entry.id {
                        rmac_dock::presentation::EntryId::Minimized(window) => *window,
                        _ => return None,
                    };
                    // Trash is the rightmost item, so minimized tiles count
                    // back from it without touching application geometry.
                    let center = trash_center
                        - (minimized_entries.len() - index) as f32
                            * (metrics.icon_size + metrics.icon_gap);
                    let visual_size = magnified_icon_size(
                        center,
                        self.hovered_item.as_ref().map(|(center, _)| *center),
                        &dock_settings,
                    );
                    let visual_offset = (metrics.icon_size - visual_size) / 2.0;
                    let thumbnail = minimized_icon_path(entry);
                    let badge = minimized_badge_path(entry);
                    // Prefer the captured thumbnail; without one, show the
                    // application icon rather than a bare letter.
                    let main_icon = thumbnail.clone().or_else(|| badge.clone());
                    let badge_overlay = thumbnail.is_some().then(|| badge.clone()).flatten();
                    let keyboard_focused = keyboard_focus_id.as_ref() == Some(&entry.id);
                    let label = entry.label.clone();
                    let tooltip_label = entry.label.clone();
                    let mut tile = div()
                        .id(format!("dock-minimized-{}-{index}", self.display_id))
                        .role(Role::Button)
                        .aria_label(entry.accessible_label.clone())
                        .relative()
                        .w(px(metrics.icon_size))
                        .h(px(metrics.icon_size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgba(tokens::primary_text()))
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .rounded(px(tokens::dock_tile_radius(metrics.icon_size)))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.dispatch_action(
                                    rmac_dock::menu::Action::ActivateEntry(
                                        rmac_dock::presentation::EntryId::Minimized(window),
                                    ),
                                    cx,
                                );
                            }),
                        )
                        // Assistive technology (AT-SPI/Orca) invokes the
                        // node's default action rather than synthesizing a
                        // mouse click; wire it to the same restore path.
                        .on_a11y_action(AccessibleAction::Click, {
                            let entity = a11y_entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.dispatch_action(
                                        rmac_dock::menu::Action::ActivateEntry(
                                            rmac_dock::presentation::EntryId::Minimized(window),
                                        ),
                                        cx,
                                    );
                                });
                            }
                        })
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.hovered_item = Some((center, tooltip_label.clone()));
                                cx.notify();
                            } else if this
                                .hovered_item
                                .as_ref()
                                .is_some_and(|(candidate, _)| *candidate == center)
                            {
                                this.hovered_item = None;
                                cx.notify();
                            }
                        }));
                    let mut visual = div()
                        .absolute()
                        .w(px(visual_size))
                        .h(px(visual_size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(tokens::dock_tile_radius(visual_size)))
                        .bg(rgba(tokens::transparent()));
                    visual = match self.placement {
                        rmac_shell_settings::DockPlacement::Bottom => {
                            visual.left(px(visual_offset)).bottom_0()
                        }
                        rmac_shell_settings::DockPlacement::Left => {
                            visual.left_0().top(px(visual_offset))
                        }
                        rmac_shell_settings::DockPlacement::Right => {
                            visual.right_0().top(px(visual_offset))
                        }
                    };
                    visual = match main_icon {
                        Some(path) => visual.child(
                            img(path)
                                .w(px(visual_size * ICON_ART_SCALE))
                                .h(px(visual_size * ICON_ART_SCALE))
                                .rounded(px(tokens::dock_tile_radius(visual_size))),
                        ),
                        None => visual.child(item_mark(&label)),
                    };
                    if keyboard_focused {
                        // rmac value: the Mac's focused minimized tile was
                        // not captured; it darkens like an app tile.
                        visual = visual.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .size_full()
                                .rounded(px(tokens::dock_tile_radius(visual_size)))
                                .bg(rgba(MENU_OPEN_DIM)),
                        );
                    }
                    tile = tile.child(visual);
                    if let Some(path) = badge_overlay {
                        tile = tile.child(
                            img(path)
                                .absolute()
                                .bottom(px(-2.0))
                                .right(px(-2.0))
                                .w(px(20.0))
                                .h(px(20.0))
                                .rounded(px(tokens::menu_item_radius())),
                        );
                    }
                    Some(tile.into_any_element())
                })
                .collect();
            let stack_count = stack_entries.len();
            let stack_children: Vec<gpui::AnyElement> = stack_entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let kind = match &entry.id {
                        rmac_dock::presentation::EntryId::Stack(kind) => kind.clone(),
                        _ => return None,
                    };
                    // Stacks sit between the minimized group and Trash,
                    // counting back from Trash exactly like a minimized
                    // tile does (§ folder/file stacks left of the Trash).
                    let center = trash_center
                        - (stack_count - index) as f32 * (metrics.icon_size + metrics.icon_gap);
                    let visual_size = magnified_icon_size(
                        center,
                        self.hovered_item.as_ref().map(|(center, _)| *center),
                        &dock_settings,
                    );
                    let visual_offset = (metrics.icon_size - visual_size) / 2.0;
                    let icon_path = stack_icon_path(&kind);
                    let keyboard_focused = keyboard_focus_id.as_ref() == Some(&entry.id);
                    let popover_open = self
                        .stack_popover
                        .as_ref()
                        .is_some_and(|popover| popover.kind == kind);
                    let menu_open = menu_anchor == Some(center);
                    let tooltip_label = entry.label.clone();
                    let mut tile = div()
                        .id(format!("dock-stack-{}-{index}", self.display_id))
                        .role(Role::Button)
                        .aria_label(entry.accessible_label.clone())
                        .relative()
                        .w(px(metrics.icon_size))
                        .h(px(metrics.icon_size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(tokens::dock_tile_radius(metrics.icon_size)))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, {
                            let kind = kind.clone();
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_stack_popover(kind.clone(), center, cx);
                            })
                        })
                        .on_mouse_down(MouseButton::Right, {
                            let kind = kind.clone();
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let session = this.status.read(cx).model().and_then(|model| {
                                    model.stack_context_menu(&kind).and_then(|menu| {
                                        rmac_dock::menu::Session::stack(&menu).ok()
                                    })
                                });
                                this.close_stack_popover(cx);
                                this.context_menu = session.map(|session| DockMenu {
                                    anchor: center,
                                    session,
                                    submenu_open: false,
                                    login: None,
                                });
                                this.input_region = None;
                                cx.notify();
                            })
                        })
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.hovered_item = Some((center, tooltip_label.clone()));
                                cx.notify();
                            } else if this
                                .hovered_item
                                .as_ref()
                                .is_some_and(|(candidate, _)| *candidate == center)
                            {
                                this.hovered_item = None;
                                cx.notify();
                            }
                        }));
                    let mut visual = div()
                        .absolute()
                        .w(px(visual_size))
                        .h(px(visual_size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(tokens::dock_tile_radius(visual_size)));
                    visual = match self.placement {
                        rmac_shell_settings::DockPlacement::Bottom => {
                            visual.left(px(visual_offset)).bottom_0()
                        }
                        rmac_shell_settings::DockPlacement::Left => {
                            visual.left_0().top(px(visual_offset))
                        }
                        rmac_shell_settings::DockPlacement::Right => {
                            visual.right_0().top(px(visual_offset))
                        }
                    };
                    if let Some(path) = icon_path {
                        visual = visual.child(
                            img(path)
                                .w(px(visual_size * ICON_ART_SCALE))
                                .h(px(visual_size * ICON_ART_SCALE))
                                .rounded(px(tokens::dock_tile_radius(visual_size))),
                        );
                    }
                    if popover_open || menu_open || keyboard_focused {
                        visual = visual.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .size_full()
                                .rounded(px(tokens::dock_tile_radius(visual_size)))
                                .bg(rgba(MENU_OPEN_DIM)),
                        );
                    }
                    tile = tile.child(visual);
                    Some(tile.into_any_element())
                })
                .collect();
            let separator_anchor = entries.len() as f32 * (metrics.icon_size + metrics.icon_gap)
                + metrics.shelf_padding
                + if separates_running {
                    metrics.separator_slot + metrics.icon_gap
                } else {
                    0.0
                };
            // Item 7: a drag out of Apps previews where it would land as a
            // thin insertion-point bar between the two kept apps nearest
            // the pointer, live-updated from drag_endpoint's Hover hint.
            let apps_drag_gap = self.apps_drag_hover.map(|fraction| {
                const BAR_WIDTH: f32 = 3.0;
                let index = ((fraction.clamp(0.0, 1.0) * pinned_count as f32).round() as usize)
                    .min(pinned_count);
                let boundary = (metrics.shelf_padding
                    + index as f32 * (metrics.icon_size + metrics.icon_gap)
                    - metrics.icon_gap / 2.0
                    - BAR_WIDTH / 2.0)
                    .max(0.0);
                let mut bar = div()
                    .absolute()
                    .rounded(px(BAR_WIDTH / 2.0))
                    .bg(rgba(tokens::accent()));
                bar = match self.placement {
                    rmac_shell_settings::DockPlacement::Bottom => bar
                        .w(px(BAR_WIDTH))
                        .h(px(metrics.icon_size))
                        .left(px(boundary))
                        .bottom_0(),
                    rmac_shell_settings::DockPlacement::Left
                    | rmac_shell_settings::DockPlacement::Right => bar
                        .h(px(BAR_WIDTH))
                        .w(px(metrics.icon_size))
                        .top(px(boundary))
                        .left_0(),
                };
                bar.into_any_element()
            });
            root.child(
                shelf
                    .children(entries.into_iter().enumerate().flat_map(|(index, entry)| {
                        let relative_center = metrics.shelf_padding
                            + metrics.icon_size / 2.0
                            + index as f32 * (metrics.icon_size + metrics.icon_gap)
                            + if separates_running && index >= pinned_count {
                                metrics.separator_slot + metrics.icon_gap
                            } else {
                                0.0
                            };
                        let app_id = match &entry.id {
                            rmac_dock::presentation::EntryId::Application(app_id) => app_id.clone(),
                            _ => unreachable!("the application group contains only apps"),
                        };
                        let activation = model.activate(&app_id);
                        let available = entry.enabled;
                        // The frontmost app's tile still presses, drags and
                        // opens its menu; its click simply does nothing.
                        let actionable =
                            !matches!(activation, rmac_dock::Activation::Unavailable { .. });
                        let menu_open = menu_anchor == Some(relative_center);
                        let keyboard_focused = keyboard_focus_id.as_ref() == Some(&entry.id);
                        let running =
                            entry.activity != rmac_dock::presentation::ActivityIndicator::None;
                        let icon_path = item_icon_path(&entry.icon, &app_id);
                        let tooltip_label = entry.label.clone();
                        let visual_size = magnified_icon_size(
                            relative_center,
                            self.hovered_item.as_ref().map(|(center, _)| *center),
                            &dock_settings,
                        );
                        let visual_offset = (metrics.icon_size - visual_size) / 2.0;
                        let lift = self.bounces.lift(&app_id, now, metrics.icon_size);
                        let slide = self.slides.get(&app_id).map_or(0.0, |(offset, started)| {
                            offset
                                * (1.0
                                    - rmac_dock::reorder::ease_out(
                                        now.saturating_sub(*started),
                                        rmac_dock::reorder::SLIDE_MS,
                                    ))
                        });
                        let drop_handler = model.file_drop_handler(&app_id);
                        let dragged_here = self
                            .tile_drag
                            .as_ref()
                            .is_some_and(|ui| ui.drag.is_active() && ui.app_id == app_id);
                        let activate_app_id = app_id.clone();
                        let mut item = div()
                            .id(format!("dock-item-{}-{index}", self.display_id))
                            .role(Role::Button)
                            .aria_label(entry.accessible_label)
                            .relative()
                            .w(px(metrics.icon_size))
                            .h(px(metrics.icon_size))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgba(tokens::primary_text()))
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .opacity(if dragged_here {
                                0.0
                            } else if available {
                                1.0
                            } else {
                                0.58
                            });
                        item = if horizontal {
                            item.left(px(slide))
                        } else {
                            item.top(px(slide))
                        };
                        if actionable {
                            // Assistive technology (AT-SPI/Orca) dispatches
                            // the node's default action rather than a mouse
                            // press-and-release pair; run the same launch
                            // path a plain left click runs.
                            let entity = a11y_entity.clone();
                            let click_app_id = activate_app_id.clone();
                            item = item.on_a11y_action(AccessibleAction::Click, move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.activate_entry(
                                        &click_app_id,
                                        gpui::Modifiers::default(),
                                        cx,
                                    );
                                });
                            });
                        }
                        let mut visual = div()
                            .id(format!("dock-visual-{}-{index}", self.display_id))
                            .absolute()
                            .w(px(visual_size))
                            .h(px(visual_size))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(tokens::dock_tile_radius(visual_size)))
                            .when(actionable, |visual| {
                                let press_app_id = app_id.clone();
                                let press_icon = icon_path.clone();
                                visual
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &gpui::MouseDownEvent, _, cx| {
                                                cx.stop_propagation();
                                                this.pressed = Some(press_app_id.clone());
                                                cx.notify();
                                                if index < pinned_count {
                                                    this.begin_tile_drag(
                                                        &press_app_id,
                                                        press_icon.clone(),
                                                        (
                                                            f32::from(event.position.x),
                                                            f32::from(event.position.y),
                                                        ),
                                                        cx,
                                                    );
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &gpui::MouseUpEvent, _, cx| {
                                                // Kept apps finish on the root, which
                                                // sees drags that left the tile.
                                                if this.tile_drag.is_some() {
                                                    return;
                                                }
                                                if this.pressed.as_deref()
                                                    == Some(activate_app_id.as_str())
                                                {
                                                    this.activate_entry(
                                                        &activate_app_id,
                                                        event.modifiers,
                                                        cx,
                                                    );
                                                }
                                            },
                                        ),
                                    )
                            });
                        // A file dragged over an app that can open it darkens
                        // the icon; dropping opens the file with that app.
                        if let Some(handler) = drop_handler {
                            let drop_app_id = app_id.clone();
                            visual = visual
                                .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.55))
                                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                                    this.open_dropped_files(
                                        &drop_app_id,
                                        handler.clone(),
                                        paths.paths().to_vec(),
                                        cx,
                                    );
                                }));
                        }
                        visual = match self.placement {
                            rmac_shell_settings::DockPlacement::Bottom => {
                                visual.left(px(visual_offset)).bottom(px(lift))
                            }
                            rmac_shell_settings::DockPlacement::Left => {
                                visual.left(px(lift)).top(px(visual_offset))
                            }
                            rmac_shell_settings::DockPlacement::Right => {
                                visual.right(px(lift)).top(px(visual_offset))
                            }
                        };
                        let squircle = visual_size * ICON_SQUIRCLE;
                        if let Some(path) = icon_path {
                            visual = visual.child(
                                img(path)
                                    .w(px(visual_size * ICON_ART_SCALE))
                                    .h(px(visual_size * ICON_ART_SCALE))
                                    .rounded(px(tokens::dock_tile_radius(visual_size))),
                            );
                        } else {
                            // Only if even the generic application artwork
                            // is missing: a lettered squircle the size of a
                            // real icon's visible shape.
                            visual = visual.child(
                                div()
                                    .w(px(squircle))
                                    .h(px(squircle))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(tokens::dock_tile_radius(visual_size)))
                                    .bg(rgba(item_color(&app_id, available)))
                                    .child(item_mark(&entry.label)),
                            );
                        }
                        if menu_open
                            || keyboard_focused
                            || self.pressed.as_deref() == Some(app_id.as_str())
                        {
                            let inset = (visual_size - squircle) / 2.0;
                            visual = visual.child(
                                div()
                                    .absolute()
                                    .left(px(inset))
                                    .top(px(inset))
                                    .w(px(squircle))
                                    .h(px(squircle))
                                    .rounded(px(tokens::dock_tile_radius(visual_size)))
                                    .bg(rgba(MENU_OPEN_DIM)),
                            );
                        }
                        item = item.child(visual);
                        let context_app_id = app_id.clone();
                        // The Files tile's menu matches the Mac Finder
                        // tile's (DOCK-05): no Options, no Quit.
                        let is_finder = context_app_id.trim_end_matches(".desktop")
                            == rmac_apps::identity::FILES;
                        item = item.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let session = {
                                    let status = this.status.read(cx);
                                    status.model().and_then(|model| {
                                        if is_finder {
                                            model.finder_context_menu().and_then(|menu| {
                                                rmac_dock::menu::Session::finder(&menu).ok()
                                            })
                                        } else {
                                            model.context_menu(&context_app_id).and_then(|menu| {
                                                rmac_dock::menu::Session::context(&menu).ok()
                                            })
                                        }
                                    })
                                };
                                this.context_menu = session.map(|session| DockMenu {
                                    anchor: relative_center,
                                    session,
                                    submenu_open: false,
                                    login: None,
                                });
                                this.input_region = None;
                                if !is_finder {
                                    this.load_login_state(&context_app_id, cx);
                                }
                                cx.notify();
                            }),
                        );
                        item = item.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.hovered_item = Some((relative_center, tooltip_label.clone()));
                                cx.notify();
                            } else if this
                                .hovered_item
                                .as_ref()
                                .is_some_and(|(center, _)| *center == relative_center)
                            {
                                this.hovered_item = None;
                                cx.notify();
                            }
                        }));
                        if running {
                            // macOS draws one dot for every running app;
                            // the frontmost app gets no special mark.
                            let indicator = div()
                                .absolute()
                                .w(px(metrics.indicator_size))
                                .h(px(metrics.indicator_size))
                                .rounded_full()
                                .bg(rgba(tokens::dock_indicator()));
                            let outside = -(metrics.indicator_offset + metrics.indicator_size);
                            let along = (metrics.icon_size - metrics.indicator_size) / 2.0;
                            let indicator = match self.placement {
                                rmac_shell_settings::DockPlacement::Bottom => {
                                    indicator.bottom(px(outside)).left(px(along))
                                }
                                // The dot sits between the tile and the
                                // screen edge, as it does below a bottom Dock.
                                rmac_shell_settings::DockPlacement::Left => {
                                    indicator.left(px(outside)).top(px(along))
                                }
                                rmac_shell_settings::DockPlacement::Right => {
                                    indicator.right(px(outside)).top(px(along))
                                }
                            };
                            item = item.child(indicator);
                        }
                        // Attention is a bounce on macOS (see the bounce
                        // tracker), never a dot. Badges and progress come
                        // only from what a running app published.
                        let published = running.then(|| launcher.get(&app_id).copied()).flatten();
                        if let Some(progress) = published.and_then(|entry| entry.progress()) {
                            let inset =
                                (metrics.icon_size - metrics.icon_size * ICON_SQUIRCLE) / 2.0;
                            let width = metrics.icon_size * ICON_SQUIRCLE * 0.8;
                            item = item.child(
                                div()
                                    .absolute()
                                    .left(px((metrics.icon_size - width) / 2.0))
                                    .bottom(px(inset + 4.0 + lift))
                                    .w(px(width))
                                    .h(px(PROGRESS_HEIGHT))
                                    .rounded_full()
                                    .bg(rgba(0x000000a6))
                                    .border_1()
                                    .border_color(rgba(tokens::light_border()))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(px((width - 2.0) * progress))
                                            .rounded_full()
                                            .bg(rgba(0xffffffe6)),
                                    ),
                            );
                        }
                        if let Some(badge) = published.and_then(|entry| entry.badge()) {
                            // Centred on the squircle's top-right corner.
                            let corner =
                                (metrics.icon_size - metrics.icon_size * ICON_SQUIRCLE) / 2.0;
                            item = item.child(
                                div()
                                    .absolute()
                                    .top(px(corner - BADGE_SIZE / 2.0 - lift))
                                    .right(px(corner - BADGE_SIZE / 2.0))
                                    .min_w(px(BADGE_SIZE))
                                    .h(px(BADGE_SIZE))
                                    .px(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(rgba(tokens::system_red()))
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgba(0xffffffff))
                                    .child(badge),
                            );
                        }
                        let mut children: Vec<gpui::AnyElement> = Vec::with_capacity(2);
                        if separates_running && index == pinned_count {
                            children.push(dock_separator(self.placement, metrics, None));
                        }
                        children.push(item.into_any_element());
                        children
                    }))
                    .when(!model.items.is_empty(), |shelf| {
                        shelf.child(dock_separator(
                            self.placement,
                            metrics,
                            Some((separator_anchor, cx)),
                        ))
                    })
                    .children(minimized_children)
                    .children(stack_children)
                    .children(apps_drag_gap)
                    .child({
                        let visual_size = magnified_icon_size(
                            trash_center,
                            self.hovered_item.as_ref().map(|(center, _)| *center),
                            &dock_settings,
                        );
                        let visual_offset = (metrics.icon_size - visual_size) / 2.0;
                        let mut trash = div()
                            .id(format!("dock-trash-{}", self.display_id))
                            .role(Role::Button)
                            .aria_label(trash_label)
                            .relative()
                            .w(px(metrics.icon_size))
                            .h(px(metrics.icon_size))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(tokens::dock_tile_radius(metrics.icon_size)))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered {
                                    this.hovered_item = Some((trash_center, "Trash".into()));
                                    cx.notify();
                                } else if this
                                    .hovered_item
                                    .as_ref()
                                    .is_some_and(|(center, _)| *center == trash_center)
                                {
                                    this.hovered_item = None;
                                    cx.notify();
                                }
                            }));
                        if trash_available {
                            trash = trash
                                .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(0.55))
                                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                                    this.trash_dropped_files(paths.paths().to_vec(), cx);
                                }));
                            trash = trash.cursor_pointer().on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    eprintln!("Dock activation requested for Trash");
                                    this.dispatch_action(
                                        rmac_dock::menu::Action::ActivateEntry(
                                            rmac_dock::presentation::EntryId::Special(
                                                rmac_dock::SpecialItemKind::Trash,
                                            ),
                                        ),
                                        cx,
                                    );
                                }),
                            );
                            // Assistive technology (AT-SPI/Orca) dispatches
                            // the node's default action rather than a mouse
                            // press-and-release pair; run the same open path.
                            let entity = a11y_entity.clone();
                            trash =
                                trash.on_a11y_action(AccessibleAction::Click, move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.dispatch_action(
                                            rmac_dock::menu::Action::ActivateEntry(
                                                rmac_dock::presentation::EntryId::Special(
                                                    rmac_dock::SpecialItemKind::Trash,
                                                ),
                                            ),
                                            cx,
                                        );
                                    });
                                });
                        }
                        trash = trash.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let session = {
                                    let status = this.status.read(cx);
                                    status
                                        .model()
                                        .and_then(|model| {
                                            model.special_context_menu(
                                                rmac_dock::SpecialItemKind::Trash,
                                            )
                                        })
                                        .and_then(|menu| {
                                            // Empty Trash goes through the review and
                                            // confirmation alert below.
                                            rmac_dock::menu::Session::special(&menu).ok()
                                        })
                                };
                                this.context_menu = session.map(|session| DockMenu {
                                    anchor: trash_center,
                                    session,
                                    submenu_open: false,
                                    login: None,
                                });
                                this.input_region = None;
                                cx.notify();
                            }),
                        );
                        if let Some(path) = trash_icon_path(trash_full) {
                            let art = visual_size * ICON_ART_SCALE;
                            let inset = (visual_size - art) / 2.0;
                            // Darkened while its menu is open, like a tile.
                            let image = img(path).absolute().w(px(art)).h(px(art)).when(
                                menu_anchor == Some(trash_center)
                                    || keyboard_focus_id.as_ref()
                                        == Some(&rmac_dock::presentation::EntryId::Special(
                                            rmac_dock::SpecialItemKind::Trash,
                                        )),
                                |image| image.opacity(0.47),
                            );
                            let image = match self.placement {
                                rmac_shell_settings::DockPlacement::Bottom => {
                                    image.left(px(visual_offset + inset)).bottom(px(inset))
                                }
                                rmac_shell_settings::DockPlacement::Left => {
                                    image.left(px(inset)).top(px(visual_offset + inset))
                                }
                                rmac_shell_settings::DockPlacement::Right => {
                                    image.right(px(inset)).top(px(visual_offset + inset))
                                }
                            };
                            trash = trash.child(image);
                        }
                        trash
                    }),
            )
            .children(
                self.tile_drag
                    .as_ref()
                    .filter(|ui| ui.drag.is_active())
                    .map(|ui| {
                        let state = ui.drag.state();
                        let left = ui.pointer.0 - ui.grab.0;
                        let top = ui.pointer.1 - ui.grab.1;
                        let art = metrics.icon_size * ICON_ART_SCALE;
                        let inset = (metrics.icon_size - art) / 2.0;
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(px(metrics.icon_size))
                            .h(px(metrics.icon_size))
                            .children(ui.icon.clone().map(|path| {
                                img(path)
                                    .absolute()
                                    .left(px(inset))
                                    .top(px(inset))
                                    .w(px(art))
                                    .h(px(art))
                            }))
                            .when(state.remove_armed, |icon| {
                                icon.child(
                                    div()
                                        .absolute()
                                        .left(px(metrics.icon_size / 2.0 - TOOLTIP_WIDTH / 2.0))
                                        .bottom(px(metrics.icon_size + 12.0))
                                        .w(px(TOOLTIP_WIDTH))
                                        .flex()
                                        .justify_center()
                                        .child(
                                            div()
                                                .px_3()
                                                .py_1()
                                                .rounded(px(tokens::tooltip_radius()))
                                                .bg(rgba(tokens::tooltip_tint()))
                                                .border_1()
                                                .border_color(rgba(tokens::light_border()))
                                                .text_sm()
                                                .text_color(rgba(tokens::primary_text()))
                                                .child("Remove"),
                                        ),
                                )
                            })
                    }),
            )
            .children(self.removing.as_ref().map(|removed| {
                let fade = 1.0
                    - (now.saturating_sub(removed.started_ms) as f32
                        / rmac_dock::reorder::REMOVE_FADE_MS as f32)
                        .min(1.0);
                let art = metrics.icon_size * ICON_ART_SCALE;
                let inset = (metrics.icon_size - art) / 2.0;
                div()
                    .absolute()
                    .left(px(removed.origin.0 + inset))
                    .top(px(removed.origin.1 + inset))
                    .w(px(art))
                    .h(px(art))
                    .opacity(fade)
                    .children(
                        removed
                            .icon
                            .clone()
                            .map(|path| img(path).w(px(art)).h(px(art))),
                    )
            }))
            .children(self.trash_review.as_ref().map(|review| {
                render_empty_trash_alert(
                    review.item_count(),
                    surface_width,
                    surface_height,
                    self.display_id,
                    cx,
                )
            }))
            .children(render_context_menu(
                self.context_menu.as_ref(),
                self.placement,
                menu_geometry,
                metrics,
                axis,
                self.display_id,
                self.option_held,
                window,
                cx,
            ))
            .children(render_stack_popover(
                self.stack_popover.as_ref(),
                self.placement,
                metrics,
                shelf_start,
                self.display_id,
                cx,
            ))
            .children(render_separator_menu(
                self.separator_menu.as_ref(),
                self.placement,
                metrics,
                shelf_start,
                self.display_id,
                cx,
            ))
        }
    }

    /// One entry of the main Dock menu panel: a row, or the parent row that
    /// stands for a submenu (Options ▸).
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum MenuEntry {
        Row(usize),
        Submenu(rmac_dock::menu::Submenu),
    }

    fn menu_entries(rows: &[rmac_dock::menu::Row]) -> Vec<MenuEntry> {
        let mut entries = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            match row.submenu {
                None => entries.push(MenuEntry::Row(index)),
                Some(submenu) if !entries.contains(&MenuEntry::Submenu(submenu)) => {
                    entries.push(MenuEntry::Submenu(submenu));
                }
                Some(_) => {}
            }
        }
        entries
    }

    fn entry_section(rows: &[rmac_dock::menu::Row], entry: MenuEntry) -> rmac_dock::menu::Section {
        match entry {
            MenuEntry::Row(index) => rows[index].section,
            MenuEntry::Submenu(_) => rmac_dock::menu::Section::Organization,
        }
    }

    fn submenu_rows(
        rows: &[rmac_dock::menu::Row],
        submenu: rmac_dock::menu::Submenu,
    ) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter_map(|(index, row)| (row.submenu == Some(submenu)).then_some(index))
            .collect()
    }

    /// A row is as wide as its longer label, so holding Option (Quit →
    /// Force Quit) never resizes the menu.
    fn menu_row_text_width(window: &Window, row: &rmac_dock::menu::Row) -> f32 {
        let label = rmac_shell_ui::text_width(window, &row.label, FontWeight::NORMAL);
        row.alternate_label
            .as_deref()
            .map(|alternate| rmac_shell_ui::text_width(window, alternate, FontWeight::NORMAL))
            .map_or(label, |alternate| label.max(alternate))
    }

    fn menu_panel_width(text: f32, checks: bool) -> f32 {
        let leading = if checks {
            MENU_CHECK_INSET + MENU_CHECK_COLUMN
        } else {
            MENU_ROW_INSET
        };
        (text + leading + MENU_ROW_INSET + 2.0 * (MENU_PADDING + MENU_BORDER)).ceil()
    }

    /// Whether any main-panel row carries a check mark; macOS then gives
    /// every row of that panel a leading check column.
    fn dock_menu_has_checks(menu: &DockMenu) -> bool {
        let rows = menu.session.rows();
        menu_entries(rows)
            .into_iter()
            .any(|entry| matches!(entry, MenuEntry::Row(index) if rows[index].checked))
    }

    /// The Dock menu is exactly as wide as its widest row plus padding; unlike
    /// menu bar menus it has no minimum (the Trash menu is 92 wide on macOS).
    fn dock_menu_width(menu: &DockMenu, window: &Window) -> f32 {
        let rows = menu.session.rows();
        let text = menu_entries(rows)
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Row(index) => menu_row_text_width(window, &rows[index]),
                MenuEntry::Submenu(submenu) => {
                    rmac_shell_ui::text_width(window, submenu.label(), FontWeight::NORMAL)
                        + MENU_CHEVRON_COLUMN
                }
            })
            .fold(0.0, f32::max);
        menu_panel_width(text, dock_menu_has_checks(menu))
    }

    fn submenu_width(rows: &[rmac_dock::menu::Row], indices: &[usize], window: &Window) -> f32 {
        let text = indices
            .iter()
            .map(|index| menu_row_text_width(window, &rows[*index]))
            .fold(0.0, f32::max);
        menu_panel_width(text, indices.iter().any(|index| rows[*index].checked))
    }

    /// Height of the rows and separators of one panel, without its padding.
    fn menu_rows_height(sections: &[rmac_dock::menu::Section]) -> f32 {
        let separators = sections
            .windows(2)
            .filter(|pair| pair[0] != pair[1])
            .count();
        sections.len() as f32 * tokens::menu_row_height() + separators as f32 * MENU_SEPARATOR
    }

    fn menu_panel(id: String, label: String, width: f32) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .role(Role::Menu)
            .aria_label(label)
            .absolute()
            .w(px(width))
            .p(px(MENU_PADDING))
            .rounded(px(tokens::menu_radius()))
            .bg(rgba(tokens::regular_dark_tint()))
            .border_1()
            .border_color(rgba(tokens::light_border()))
            .shadow_lg()
            .text_size(px(13.0))
            .text_color(rgba(tokens::primary_text()))
            .occlude()
    }

    fn menu_separator() -> gpui::AnyElement {
        // An 11-tall separator whose line is inset 16 from the edge.
        div()
            .h(px(MENU_BORDER))
            .mx(px(MENU_ROW_INSET))
            .my(px((MENU_SEPARATOR - MENU_BORDER) / 2.0))
            .bg(rgba(tokens::separator()))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn menu_row_element(
        row: &rmac_dock::menu::Row,
        index: usize,
        display_id: u64,
        has_checks: bool,
        selected: bool,
        option_held: bool,
        in_submenu: bool,
        cx: &Context<Dock>,
    ) -> gpui::AnyElement {
        let row_id = row.id.clone();
        let primary = row.primary.clone();
        let secondary = row.secondary.clone();
        let has_alternate = row.alternate_label.is_some();
        let enabled = row.enabled && primary.is_some();
        let label = match (&row.alternate_label, option_held) {
            (Some(alternate), true) => alternate.clone(),
            _ => row.label.clone(),
        };
        let mut element = div()
            .id(format!("dock-menu-{display_id}-{index}"))
            .role(Role::MenuItem)
            .aria_label(row.accessible_label.clone())
            .h(px(tokens::menu_row_height()))
            .pl(px(if has_checks {
                MENU_CHECK_INSET
            } else {
                MENU_ROW_INSET
            }))
            .pr(px(MENU_ROW_INSET))
            .flex()
            .items_center()
            .rounded(px(tokens::menu_item_radius()))
            .when(selected, |style| style.bg(rgba(tokens::accent())))
            .when(!enabled, |style| {
                style.text_color(rgba(tokens::disabled_text()))
            });
        // macOS puts the check mark in a leading column.
        if has_checks {
            element = element.child(
                div()
                    .w(px(MENU_CHECK_COLUMN))
                    .flex_none()
                    .child(if row.checked { "✓" } else { "" }),
            );
        }
        element = element.child(label);
        if enabled {
            let primary = primary.expect("enabled Dock menu row has an action");
            element = element
                .cursor_pointer()
                .hover(|style| style.bg(rgba(tokens::accent())))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if !*hovered {
                        return;
                    }
                    let Some(menu) = this.context_menu.as_mut() else {
                        return;
                    };
                    // Moving onto a main-panel row closes the submenu.
                    let closed_submenu = !in_submenu && std::mem::take(&mut menu.submenu_open);
                    if menu.session.select(&row_id) || closed_submenu {
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                    cx.stop_propagation();
                    // Option turns Quit into Force Quit.
                    let action = if has_alternate && event.modifiers().alt {
                        secondary.clone().unwrap_or_else(|| primary.clone())
                    } else {
                        primary.clone()
                    };
                    this.context_menu = None;
                    this.input_region = None;
                    this.run_menu_action(action, cx);
                }));
        }
        element.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_context_menu(
        menu: Option<&DockMenu>,
        placement: rmac_shell_settings::DockPlacement,
        geometry: Option<(f32, f32, f32)>,
        metrics: TileMetrics,
        axis_length: f32,
        display_id: u64,
        option_held: bool,
        window: &Window,
        cx: &Context<Dock>,
    ) -> Vec<gpui::AnyElement> {
        let (Some(menu), Some((start, tip, width))) = (menu, geometry) else {
            return Vec::new();
        };
        let selected = menu.session.selected().cloned();
        let rows = menu.session.rows().to_vec();
        let entries = menu_entries(&rows);
        let has_checks = dock_menu_has_checks(menu);
        // macOS Dock menus have no title row; the app name stays the
        // accessible title.
        let mut panel = menu_panel(
            format!("dock-menu-{display_id}"),
            menu.session.accessible_title().to_owned(),
            width,
        );
        let offset = metrics.exclusive_zone + MENU_SHELF_GAP;
        panel = match placement {
            rmac_shell_settings::DockPlacement::Bottom => {
                panel.left(px(start)).bottom(px(offset)).child(
                    canvas(
                        |_, _, _| (),
                        |bounds, _, window, _| paint_menu_pointer(bounds, window),
                    )
                    .absolute()
                    // Start at the panel's inner edge so the pointer covers
                    // the rim across its base.
                    .left(px(tip - MENU_BORDER - MENU_POINTER_WIDTH / 2.0))
                    .bottom(px(-(MENU_POINTER_HEIGHT + MENU_BORDER)))
                    .w(px(MENU_POINTER_WIDTH))
                    .h(px(MENU_POINTER_HEIGHT + MENU_BORDER)),
                )
            }
            rmac_shell_settings::DockPlacement::Left => panel.left(px(offset)).top(px(start)),
            rmac_shell_settings::DockPlacement::Right => panel.right(px(offset)).top(px(start)),
        };
        let sections: Vec<_> = entries
            .iter()
            .map(|entry| entry_section(&rows, *entry))
            .collect();
        let panel_height = menu_rows_height(&sections) + 2.0 * (MENU_PADDING + MENU_BORDER);
        let mut submenu_panel = None;
        for (position, entry) in entries.iter().enumerate() {
            if position > 0 && sections[position - 1] != sections[position] {
                panel = panel.child(menu_separator());
            }
            match *entry {
                MenuEntry::Row(index) => {
                    panel = panel.child(menu_row_element(
                        &rows[index],
                        index,
                        display_id,
                        has_checks,
                        selected.as_ref() == Some(&rows[index].id),
                        option_held,
                        false,
                        cx,
                    ));
                }
                MenuEntry::Submenu(submenu) => {
                    let open = menu.submenu_open;
                    let parent = div()
                        .id(format!("dock-menu-{display_id}-submenu"))
                        .role(Role::MenuItem)
                        .aria_label(submenu.label())
                        .h(px(tokens::menu_row_height()))
                        .pl(px(if has_checks {
                            MENU_CHECK_INSET + MENU_CHECK_COLUMN
                        } else {
                            MENU_ROW_INSET
                        }))
                        .pr(px(MENU_ROW_INSET))
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(px(tokens::menu_item_radius()))
                        .when(open, |style| style.bg(rgba(tokens::accent())))
                        .child(submenu.label())
                        .child("›")
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if *hovered {
                                if let Some(menu) = this.context_menu.as_mut() {
                                    if !menu.submenu_open {
                                        menu.submenu_open = true;
                                        cx.notify();
                                    }
                                }
                            }
                        }));
                    panel = panel.child(parent);
                    if open {
                        // Top of the parent row, measured from the panel top.
                        let above = menu_rows_height(&sections[..position])
                            + if position > 0 && sections[position - 1] != sections[position] {
                                MENU_SEPARATOR
                            } else {
                                0.0
                            };
                        let parent_top = MENU_BORDER + MENU_PADDING + above;
                        let indices = submenu_rows(&rows, submenu);
                        let login = menu.login.clone();
                        let login_label = "Open at Login";
                        let sub_width = submenu_width(&rows, &indices, window).max(
                            login.as_ref().map_or(0.0, |_| {
                                menu_panel_width(
                                    rmac_shell_ui::text_width(
                                        window,
                                        login_label,
                                        FontWeight::NORMAL,
                                    ),
                                    true,
                                )
                            }),
                        );
                        let sub_rows = indices.len() + usize::from(login.is_some());
                        let sub_height = sub_rows as f32 * tokens::menu_row_height()
                            + 2.0 * (MENU_PADDING + MENU_BORDER);
                        let sub_checks = indices.iter().any(|index| rows[*index].checked)
                            || login.as_ref().is_some_and(|login| login.enabled);
                        let mut sub = menu_panel(
                            format!("dock-submenu-{display_id}"),
                            submenu.label().to_owned(),
                            sub_width,
                        );
                        // The submenu's first row lines up with its parent
                        // and it opens to the right unless the output ends.
                        sub = match placement {
                            rmac_shell_settings::DockPlacement::Bottom => {
                                let right_start = start + width - MENU_SUBMENU_OVERLAP;
                                let left = if right_start + sub_width > axis_length - 8.0 {
                                    start - sub_width + MENU_SUBMENU_OVERLAP
                                } else {
                                    right_start
                                };
                                let parent_top_from_bottom = offset + panel_height - parent_top;
                                sub.left(px(left))
                                    .bottom(px(parent_top_from_bottom + MENU_BORDER + MENU_PADDING
                                        - sub_height))
                            }
                            rmac_shell_settings::DockPlacement::Left => sub
                                .left(px(offset + width - MENU_SUBMENU_OVERLAP))
                                .top(px(start + parent_top - MENU_BORDER - MENU_PADDING)),
                            rmac_shell_settings::DockPlacement::Right => sub
                                .right(px(offset + width - MENU_SUBMENU_OVERLAP))
                                .top(px(start + parent_top - MENU_BORDER - MENU_PADDING)),
                        };
                        for index in indices {
                            sub = sub.child(menu_row_element(
                                &rows[index],
                                index,
                                display_id,
                                sub_checks,
                                selected.as_ref() == Some(&rows[index].id),
                                option_held,
                                true,
                                cx,
                            ));
                            // macOS order: Keep in Dock, Open at Login,
                            // Show in Finder.
                            if rows[index].id == rmac_dock::menu::RowId::Pin {
                                if let Some(login) = login.clone() {
                                    let checked = login.enabled;
                                    sub = sub.child(
                                        div()
                                            .id(format!("dock-menu-{display_id}-login"))
                                            .role(Role::MenuItem)
                                            .aria_label(login_label)
                                            .h(px(tokens::menu_row_height()))
                                            .pl(px(MENU_CHECK_INSET))
                                            .pr(px(MENU_ROW_INSET))
                                            .flex()
                                            .items_center()
                                            .rounded(px(tokens::menu_item_radius()))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(tokens::accent())))
                                            .child(
                                                div()
                                                    .w(px(MENU_CHECK_COLUMN))
                                                    .flex_none()
                                                    .child(if checked { "✓" } else { "" }),
                                            )
                                            .child(login_label)
                                            .on_click(cx.listener(
                                                move |this, _: &gpui::ClickEvent, _, cx| {
                                                    cx.stop_propagation();
                                                    this.toggle_open_at_login(login.clone(), cx);
                                                },
                                            )),
                                    );
                                }
                            }
                        }
                        submenu_panel = Some(sub.into_any_element());
                    }
                }
            }
        }
        let mut panels = vec![panel.into_any_element()];
        panels.extend(submenu_panel);
        panels
    }

    /// A folder/file stack's Fan or Grid popover (item 1). `Automatic`
    /// resolves to Fan, macOS's default for a small stack; `List` has no
    /// distinct rmac layout yet and also falls back to Grid.
    fn render_stack_popover(
        popover: Option<&StackPopoverUi>,
        placement: rmac_shell_settings::DockPlacement,
        metrics: TileMetrics,
        shelf_start: f32,
        display_id: u64,
        cx: &Context<Dock>,
    ) -> Vec<gpui::AnyElement> {
        let Some(popover) = popover else {
            return Vec::new();
        };
        let grid = matches!(
            popover.view_content_as,
            rmac_shell_settings::DockStackViewContentAs::Grid
                | rmac_shell_settings::DockStackViewContentAs::List
        );
        let items: Vec<&rmac_desktop::Item> = popover
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.items.iter().collect())
            .unwrap_or_default();
        let total = items.len();
        let shown = &items[..total.min(STACK_POPOVER_MAX_ITEMS)];

        let mut panel = div()
            .id(format!("dock-stack-popover-{display_id}"))
            .role(Role::Dialog)
            .aria_label(format!("{}, stack contents", popover.name))
            .absolute()
            .w(px(STACK_POPOVER_WIDTH))
            .p(px(MENU_PADDING + 6.0))
            .rounded(px(tokens::menu_radius()))
            .bg(rgba(tokens::regular_dark_tint()))
            .border_1()
            .border_color(rgba(tokens::light_border()))
            .shadow_lg()
            .text_color(rgba(tokens::primary_text()))
            .occlude();
        let offset = metrics.exclusive_zone + MENU_SHELF_GAP;
        let along = shelf_start + popover.anchor;
        panel = match placement {
            rmac_shell_settings::DockPlacement::Bottom => panel
                .left(px((along - STACK_POPOVER_WIDTH / 2.0).max(8.0)))
                .bottom(px(offset)),
            rmac_shell_settings::DockPlacement::Left => {
                panel.left(px(offset)).top(px((along - 80.0).max(8.0)))
            }
            rmac_shell_settings::DockPlacement::Right => {
                panel.right(px(offset)).top(px((along - 80.0).max(8.0)))
            }
        };
        panel = panel.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(tokens::secondary_text()))
                .pb(px(6.0))
                .truncate()
                .child(popover.name.clone()),
        );
        panel = if popover.snapshot.is_none() {
            panel.child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgba(tokens::secondary_text()))
                    .child("Loading…"),
            )
        } else if shown.is_empty() {
            panel.child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgba(tokens::secondary_text()))
                    .child("Empty"),
            )
        } else if grid {
            panel.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .children(shown.iter().map(|item| stack_popover_item(item, false, cx))),
            )
        } else {
            panel.child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(6.0))
                    .overflow_hidden()
                    .children(
                        shown
                            .iter()
                            .enumerate()
                            .map(|(index, item)| stack_popover_item(item, index == 0, cx)),
                    ),
            )
        };
        if total > shown.len() {
            panel = panel.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(tokens::disabled_text()))
                    .pt(px(4.0))
                    .child(format!("{} more…", total - shown.len())),
            );
        }
        panel = panel.child(menu_separator());
        let kind = popover.kind.clone();
        panel = panel.child(
            div()
                .id("dock-stack-popover-open-in-files")
                .role(Role::MenuItem)
                .aria_label("Open in Files")
                .h(px(tokens::menu_row_height()))
                .px(px(MENU_ROW_INSET))
                .flex()
                .items_center()
                .cursor_pointer()
                .rounded(px(tokens::menu_item_radius()))
                .child("Open in Files")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_stack_popover(cx);
                        this.dispatch_action(
                            rmac_dock::menu::Action::ActivateEntry(
                                rmac_dock::presentation::EntryId::Stack(kind.clone()),
                            ),
                            cx,
                        );
                    }),
                ),
        );
        vec![panel.into_any_element()]
    }

    /// One popover item: icon, truncated name, click to open (and close the
    /// popover, as the Mac does).
    fn stack_popover_item(
        item: &rmac_desktop::Item,
        emphasize: bool,
        cx: &Context<Dock>,
    ) -> gpui::AnyElement {
        let size = if emphasize {
            STACK_POPOVER_ITEM
        } else {
            STACK_POPOVER_ITEM * 0.85
        };
        let path = item.path.clone();
        let name = item.name.clone();
        div()
            .id(SharedString::from(format!("dock-stack-item-{name}")))
            .role(Role::Button)
            .aria_label(name.clone())
            .w(px(size + 8.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .cursor_pointer()
            .rounded(px(tokens::menu_item_radius()))
            .children(
                stack_popover_item_icon_path(item)
                    .map(|icon_path| img(icon_path).w(px(size)).h(px(size))),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(tokens::secondary_text()))
                    .w(px(size + 8.0))
                    .truncate()
                    .child(name),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_stack_popover_item(path.clone(), cx);
                }),
            )
            .into_any_element()
    }

    /// One row of the separator menu: a plain clickable row, optionally
    /// checked (a leading ✓, like the model-driven Dock menus) or a
    /// submenu parent (a trailing ›).
    fn separator_menu_row(
        id: SharedString,
        label: &'static str,
        checked: bool,
        submenu: bool,
        on_click: impl Fn(&mut Dock, &mut Context<Dock>) + 'static,
        cx: &Context<Dock>,
    ) -> gpui::AnyElement {
        div()
            .id(id)
            .role(Role::MenuItem)
            .aria_label(label)
            .h(px(tokens::menu_row_height()))
            .pl(px(if checked {
                MENU_CHECK_INSET + MENU_CHECK_COLUMN
            } else {
                MENU_ROW_INSET
            }))
            .pr(px(MENU_ROW_INSET))
            .flex()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .rounded(px(tokens::menu_item_radius()))
            .when(checked, |row| {
                row.child(div().absolute().left(px(MENU_CHECK_INSET)).child("✓"))
            })
            .child(label)
            .when(submenu, |row| row.child("›"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    on_click(this, cx);
                }),
            )
            .into_any_element()
    }

    /// The Dock separator's own menu (DOCK-02): Turn Hiding On/Off, Turn
    /// Magnification On/Off, Position on Screen ▸, Minimise Using ▸, Dock
    /// Settings…. Position on Screen/Minimise Using expand in place rather
    /// than as a flyout submenu (not measured against the owner's Mac: S).
    fn render_separator_menu(
        menu: Option<&SeparatorMenu>,
        placement: rmac_shell_settings::DockPlacement,
        metrics: TileMetrics,
        shelf_start: f32,
        display_id: u64,
        cx: &Context<Dock>,
    ) -> Vec<gpui::AnyElement> {
        let Some(menu) = menu else {
            return Vec::new();
        };
        const WIDTH: f32 = 240.0;
        let mut panel = div()
            .id(format!("dock-separator-menu-{display_id}"))
            .role(Role::Menu)
            .aria_label("Dock separator menu")
            .absolute()
            .w(px(WIDTH))
            .p(px(MENU_PADDING + 2.0))
            .rounded(px(tokens::menu_radius()))
            .bg(rgba(tokens::regular_dark_tint()))
            .border_1()
            .border_color(rgba(tokens::light_border()))
            .shadow_lg()
            .text_size(px(13.0))
            .text_color(rgba(tokens::primary_text()))
            .occlude();
        let offset = metrics.exclusive_zone + MENU_SHELF_GAP;
        let along = shelf_start + menu.anchor;
        panel = match placement {
            rmac_shell_settings::DockPlacement::Bottom => panel
                .left(px((along - WIDTH / 2.0).max(8.0)))
                .bottom(px(offset)),
            rmac_shell_settings::DockPlacement::Left => {
                panel.left(px(offset)).top(px((along - 80.0).max(8.0)))
            }
            rmac_shell_settings::DockPlacement::Right => {
                panel.right(px(offset)).top(px((along - 80.0).max(8.0)))
            }
        };
        let autohide = menu.dock.autohide;
        panel = panel.child(separator_menu_row(
            "dock-separator-menu-hiding".into(),
            if autohide {
                "Turn Hiding Off"
            } else {
                "Turn Hiding On"
            },
            false,
            false,
            |this, cx| this.toggle_hiding_from_menu(cx),
            cx,
        ));
        let magnification = menu.dock.magnification;
        panel = panel.child(separator_menu_row(
            "dock-separator-menu-magnification".into(),
            if magnification {
                "Turn Magnification Off"
            } else {
                "Turn Magnification On"
            },
            false,
            false,
            |this, cx| this.toggle_magnification_from_menu(cx),
            cx,
        ));
        panel = panel.child(menu_separator());
        let position_open = menu.submenu_open == Some(SeparatorSubmenu::Position);
        panel = panel.child(separator_menu_row(
            "dock-separator-menu-position".into(),
            "Position on Screen",
            false,
            true,
            move |this, cx| {
                if let Some(menu) = this.separator_menu.as_mut() {
                    menu.submenu_open = if position_open {
                        None
                    } else {
                        Some(SeparatorSubmenu::Position)
                    };
                    cx.notify();
                }
            },
            cx,
        ));
        if position_open {
            let placements = [
                (rmac_shell_settings::DockPlacement::Left, "Left"),
                (rmac_shell_settings::DockPlacement::Bottom, "Bottom"),
                (rmac_shell_settings::DockPlacement::Right, "Right"),
            ];
            for (value, label) in placements {
                panel = panel.child(separator_menu_row(
                    format!("dock-separator-menu-position-{label}").into(),
                    label,
                    menu.dock.placement == value,
                    false,
                    move |this, cx| this.set_placement_from_menu(value, cx),
                    cx,
                ));
            }
        }
        let minimize_open = menu.submenu_open == Some(SeparatorSubmenu::MinimizeUsing);
        panel = panel.child(separator_menu_row(
            "dock-separator-menu-minimize".into(),
            "Minimise Using",
            false,
            true,
            move |this, cx| {
                if let Some(menu) = this.separator_menu.as_mut() {
                    menu.submenu_open = if minimize_open {
                        None
                    } else {
                        Some(SeparatorSubmenu::MinimizeUsing)
                    };
                    cx.notify();
                }
            },
            cx,
        ));
        if minimize_open {
            let effects = [
                (
                    rmac_shell_settings::DockMinimizeEffect::Genie,
                    "Genie Effect",
                ),
                (
                    rmac_shell_settings::DockMinimizeEffect::Scale,
                    "Scale Effect",
                ),
            ];
            for (value, label) in effects {
                panel = panel.child(separator_menu_row(
                    format!("dock-separator-menu-minimize-{label}").into(),
                    label,
                    menu.dock.minimize_effect == value,
                    false,
                    move |this, cx| this.set_minimize_effect_from_menu(value, cx),
                    cx,
                ));
            }
        }
        panel = panel.child(menu_separator());
        panel = panel.child(separator_menu_row(
            "dock-separator-menu-settings".into(),
            "Dock Settings…",
            false,
            false,
            |this, cx| this.open_dock_settings(cx),
            cx,
        ));
        vec![panel.into_any_element()]
    }

    /// The 20 × 10 pointer under a bottom Dock's menu, tip on the tile
    /// centre, drawn in the panel's fill with its rim on the two slanted
    /// sides. `bounds` spans the pointer's full width and height.
    fn paint_menu_pointer(bounds: Bounds<gpui::Pixels>, window: &mut Window) {
        let left = f32::from(bounds.origin.x);
        let top = f32::from(bounds.origin.y);
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let tip_x = left + width / 2.0;
        let tip_y = top + height;
        // A small round on the tip, as macOS draws it.
        let round = 1.4;
        let outline = |builder: &mut PathBuilder| {
            builder.move_to(point(px(left), px(top)));
            builder.line_to(point(px(tip_x - round), px(tip_y - round)));
            builder.curve_to(
                point(px(tip_x + round), px(tip_y - round)),
                point(px(tip_x), px(tip_y)),
            );
            builder.line_to(point(px(left + width), px(top)));
        };
        let mut fill = PathBuilder::fill();
        outline(&mut fill);
        fill.close();
        if let Ok(path) = fill.build() {
            window.paint_path(path, rgba(tokens::regular_dark_tint()));
        }
        let mut rim = PathBuilder::stroke(px(MENU_BORDER));
        outline(&mut rim);
        if let Ok(path) = rim.build() {
            window.paint_path(path, rgba(tokens::light_border()));
        }
    }

    /// The alert Files shows before the Trash is emptied from the Dock.
    /// Text follows macOS; the geometry is rmac's standard alert (the Mac
    /// alert was not measured in this pass).
    fn render_empty_trash_alert(
        item_count: usize,
        surface_width: f32,
        surface_height: f32,
        display_id: u64,
        cx: &Context<Dock>,
    ) -> gpui::AnyElement {
        const WIDTH: f32 = 260.0;
        // The count stays in the accessible name; the text is the Mac's.
        let accessible = format!("Empty Trash, {item_count} items");
        let button = |id: &'static str, label: &'static str, default: bool| {
            div()
                .id(format!("dock-trash-alert-{display_id}-{id}"))
                .role(Role::Button)
                .aria_label(label)
                .flex_1()
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .bg(rgba(if default {
                    tokens::accent()
                } else {
                    tokens::light_border()
                }))
                .child(label)
        };
        div()
            .id(format!("dock-trash-alert-{display_id}"))
            .role(Role::Dialog)
            .aria_label(accessible)
            .absolute()
            .left(px(((surface_width - WIDTH) / 2.0).max(0.0)))
            .top(px(surface_height * 0.28))
            .w(px(WIDTH))
            .p(px(16.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.0))
            .rounded(px(tokens::menu_radius() * 2.0))
            .bg(rgba(tokens::regular_dark_tint()))
            .border_1()
            .border_color(rgba(tokens::light_border()))
            .shadow_lg()
            .text_color(rgba(tokens::primary_text()))
            .occlude()
            .children(trash_icon_path(true).map(|path| img(path).w(px(64.0)).h(px(64.0))))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_center()
                    .child("Are you sure you want to permanently erase the items in the Trash?"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_center()
                    .child("You can’t undo this action."),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.0))
                    .text_size(px(13.0))
                    .child(button("cancel", "Cancel", false).on_click(cx.listener(
                        |this, _: &gpui::ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.finish_trash_review(false, cx);
                        },
                    )))
                    .child(button("empty", "Empty Trash", true).on_click(cx.listener(
                        |this, _: &gpui::ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.finish_trash_review(true, cx);
                        },
                    ))),
            )
            .into_any_element()
    }

    /// The separator between the kept/running applications and Trash's
    /// group. `interaction` is `Some` only for that one (not the separator
    /// between kept and running apps): its right-click menu (DOCK-02) and
    /// drag-to-resize (DOCK-03).
    fn dock_separator(
        placement: rmac_shell_settings::DockPlacement,
        metrics: TileMetrics,
        interaction: Option<(f32, &Context<Dock>)>,
    ) -> gpui::AnyElement {
        // Centred on the tile row (1 inside it at each end), 13 clear of the
        // tile gaps on either side.
        let line = div().bg(rgba(tokens::dock_separator()));
        let inset = (metrics.icon_size - metrics.separator_length) / 2.0;
        let Some((anchor, cx)) = interaction else {
            return match placement {
                rmac_shell_settings::DockPlacement::Bottom => line
                    .w(px(SEPARATOR_WIDTH))
                    .h(px(metrics.separator_length))
                    .mx(px(metrics.separator_margin))
                    .mb(px(inset))
                    .into_any_element(),
                rmac_shell_settings::DockPlacement::Left
                | rmac_shell_settings::DockPlacement::Right => line
                    .w(px(metrics.separator_length))
                    .h(px(SEPARATOR_WIDTH))
                    .my(px(metrics.separator_margin))
                    .into_any_element(),
            };
        };
        // A wider invisible hit/drag target around the thin visible line,
        // like the Mac's (not separately measured: S).
        const HIT_WIDTH: f32 = 9.0;
        let horizontal = placement == rmac_shell_settings::DockPlacement::Bottom;
        let mut hit = div()
            .id("dock-separator-main")
            .role(Role::Button)
            .aria_label("Dock separator")
            .flex()
            .items_center()
            .justify_center()
            .child(
                line.when(horizontal, |line| {
                    line.w(px(SEPARATOR_WIDTH)).h(px(metrics.separator_length))
                })
                .when(!horizontal, |line| {
                    line.w(px(metrics.separator_length)).h(px(SEPARATOR_WIDTH))
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.begin_separator_drag(
                        (f32::from(event.position.x), f32::from(event.position.y)),
                        cx,
                    );
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_separator_menu(anchor, cx);
                }),
            );
        hit = if horizontal {
            // A bottom Dock resizes on a vertical drag (up grows it).
            hit.w(px(HIT_WIDTH))
                .h(px(metrics.separator_length))
                .mx(px(
                    metrics.separator_margin - (HIT_WIDTH - SEPARATOR_WIDTH) / 2.0
                ))
                .mb(px(inset))
                .cursor_row_resize()
        } else {
            // A side Dock resizes on a horizontal drag (away from the edge
            // grows it).
            hit.w(px(metrics.separator_length))
                .h(px(HIT_WIDTH))
                .my(px(
                    metrics.separator_margin - (HIT_WIDTH - SEPARATOR_WIDTH) / 2.0
                ))
                .cursor_col_resize()
        };
        hit.into_any_element()
    }

    fn magnified_icon_size(
        center: f32,
        pointer: Option<f32>,
        settings: &rmac_shell_settings::DockSettings,
    ) -> f32 {
        let metrics = TileMetrics::new(settings.tile_size);
        if !settings.magnification {
            return metrics.icon_size;
        }
        let config = rmac_dock::motion::MagnificationConfig {
            icon_size: metrics.icon_size,
            maximum_scale: settings.magnification_scale,
            ..Default::default()
        };
        let pointer = pointer.map(|pointer| metrics.icon_size / 2.0 + pointer - center);
        rmac_dock::motion::magnified_layout(1, pointer, true, false, config)
            .ok()
            .and_then(|layout| layout.items.first().map(|item| item.size))
            .unwrap_or(metrics.icon_size)
    }

    /// A stack's "Sort by" setting, mapped onto `rmac_desktop::SortOrder`
    /// (Linux filesystems do not reliably distinguish "date added" from
    /// "date modified" the way HFS+/APFS do; both fall back to `mtime`, as
    /// `rmac_shell_settings::DockStackSortBy`'s own doc comment records).
    fn stack_sort_order(sort_by: rmac_shell_settings::DockStackSortBy) -> rmac_desktop::SortOrder {
        match sort_by {
            rmac_shell_settings::DockStackSortBy::Name => rmac_desktop::SortOrder::Name,
            rmac_shell_settings::DockStackSortBy::Kind => rmac_desktop::SortOrder::Kind,
            rmac_shell_settings::DockStackSortBy::DateAdded
            | rmac_shell_settings::DockStackSortBy::DateModified
            | rmac_shell_settings::DockStackSortBy::DateCreated => {
                rmac_desktop::SortOrder::DateModified
            }
        }
    }

    /// The stack's own tile artwork (original, not traced): the Downloads
    /// glyph for the special case, a plain folder otherwise, matching
    /// `rmac_dock::presentation::BuiltinIcon::{Downloads,Folder}`.
    fn stack_icon_path(kind: &rmac_shell_settings::DockStackKind) -> Option<PathBuf> {
        dock_asset_path(match kind {
            rmac_shell_settings::DockStackKind::Downloads => "downloads.svg",
            rmac_shell_settings::DockStackKind::Path { .. } => "folder.svg",
        })
    }

    /// One popover item's icon: a folder for a subdirectory, a generic
    /// document otherwise (original artwork, not traced; no per-type icon
    /// theme lookup here, matching the Fan/Grid mock's plain shapes).
    fn stack_popover_item_icon_path(item: &rmac_desktop::Item) -> Option<PathBuf> {
        dock_asset_path(match item.kind {
            rmac_desktop::ItemKind::Directory => "folder.svg",
            rmac_desktop::ItemKind::File
            | rmac_desktop::ItemKind::SymbolicLink
            | rmac_desktop::ItemKind::Other => "stack-item-document.svg",
        })
    }

    fn item_mark(label: &str) -> String {
        let mark = label
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect::<String>();
        if mark.is_empty() {
            "•".into()
        } else {
            mark
        }
    }

    fn item_icon_path(icon: &rmac_dock::presentation::Icon, app_id: &str) -> Option<PathBuf> {
        match icon {
            rmac_dock::presentation::Icon::File(path) if path.is_file() => Some(path.clone()),
            rmac_dock::presentation::Icon::File(_) | rmac_dock::presentation::Icon::Builtin(_) => {
                // An application with no artwork of its own gets the generic
                // application icon, as on the Mac.
                first_party_icon_path(app_id).or_else(|| dock_asset_path("application.svg"))
            }
        }
    }

    /// The minimized tile's own image: the capture taken at minimize time.
    fn minimized_icon_path(entry: &rmac_dock::presentation::Entry) -> Option<PathBuf> {
        match &entry.icon {
            rmac_dock::presentation::Icon::File(path) if path.is_file() => Some(path.clone()),
            _ => None,
        }
    }

    /// The owning application's icon, drawn as the tile badge.
    fn minimized_badge_path(entry: &rmac_dock::presentation::Entry) -> Option<PathBuf> {
        match entry.miniature.as_ref() {
            Some(rmac_dock::presentation::Icon::File(path)) if path.is_file() => Some(path.clone()),
            _ => None,
        }
    }

    fn first_party_icon_path(app_id: &str) -> Option<PathBuf> {
        let identity = app_id.trim_end_matches(".desktop");
        let file = match identity {
            rmac_apps::identity::FILES => "org.rmac.Files.svg",
            rmac_apps::identity::APP_DRAWER => "org.rmac.AppDrawer.svg",
            rmac_apps::identity::TERMINAL => "org.rmac.Terminal.svg",
            rmac_apps::identity::NOTES => "org.rmac.Notes.svg",
            rmac_apps::identity::TEXT_EDITOR => "org.rmac.TextEditor.svg",
            rmac_apps::identity::SYSTEM_MONITOR => "org.rmac.SystemMonitor.svg",
            rmac_apps::identity::SYSTEM_SETTINGS => "org.rmac.SystemSettings.svg",
            rmac_apps::identity::CALCULATOR => "org.rmac.Calculator.svg",
            rmac_apps::identity::PREVIEW => "org.rmac.Preview.svg",
            rmac_apps::identity::CLOCK => "org.rmac.Clock.svg",
            rmac_apps::identity::WEATHER => "org.rmac.Weather.svg",
            rmac_apps::identity::PLAYER => "org.rmac.Player.svg",
            _ => return None,
        };
        // The installed theme copy first (user, then system), then the
        // source tree three levels above this crate (shell/bins/rmac-dock).
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            });
        data_home
            .into_iter()
            .chain([PathBuf::from("/usr/share")])
            .map(|data| data.join("icons/hicolor/scalable/apps").join(file))
            .chain([PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../packaging/rmac-apps/icons")
                .join(file)])
            .find(|path| path.is_file())
    }

    fn trash_icon_path(full: bool) -> Option<PathBuf> {
        dock_asset_path(if full {
            "trash-full.svg"
        } else {
            "trash-empty.svg"
        })
    }

    /// The Dock's own artwork: installed copy first, then the source tree.
    fn dock_asset_path(file: &str) -> Option<PathBuf> {
        [
            PathBuf::from("/usr/share/rmac/dock/icons").join(file),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../crates/rmac-dock/assets/icons")
                .join(file),
        ]
        .into_iter()
        .find(|path| path.is_file())
    }

    fn item_color(app_id: &str, enabled: bool) -> u32 {
        if !enabled {
            return 0x8d929aff;
        }
        match app_id.trim_end_matches(".desktop") {
            rmac_apps::identity::FILES => 0x4a90e2ff,
            rmac_apps::identity::TERMINAL => 0x25282eff,
            rmac_apps::identity::NOTES => 0xf5c94cff,
            rmac_apps::identity::TEXT_EDITOR => 0x5e72e4ff,
            rmac_apps::identity::SYSTEM_MONITOR => 0x34a875ff,
            rmac_apps::identity::SYSTEM_SETTINGS => 0x8d929aff,
            rmac_apps::identity::CALCULATOR => 0xff9500ff,
            rmac_apps::identity::PREVIEW => 0x2f6fd6ff,
            rmac_apps::identity::CLOCK => 0x2c2d31ff,
            rmac_apps::identity::WEATHER => 0x3a86e8ff,
            rmac_apps::identity::PLAYER => 0x2a2c31ff,
            _ => {
                let hash = app_id.bytes().fold(0x811c9dc5u32, |hash, byte| {
                    hash.wrapping_mul(0x01000193) ^ u32::from(byte)
                });
                ((hash & 0xbfbfbf) | 0x303030) << 8 | 0xff
            }
        }
    }

    fn next_activation_id() -> Option<rmac_compositor::ActivationId> {
        let Ok(previous) =
            NEXT_ACTIVATION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
        else {
            return None;
        };
        Some(rmac_compositor::ActivationId(previous + 1))
    }

    fn record_configured_surface(window: &Window, display_id: u64) {
        let Some(path) = env::var_os(READY_FILE_ENV).map(PathBuf::from) else {
            return;
        };
        let scale = window.scale_factor();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("open Dock evidence {path:?}: {error}"));
        writeln!(file, "display={display_id} scale={scale}")
            .unwrap_or_else(|error| panic!("write Dock evidence {path:?}: {error}"));
    }

    fn record_render_count(window: &Window, display_id: u64, render_count: u64) {
        let Some(directory) = env::var_os(RENDER_COUNT_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        window.on_next_frame(move |_, _| {
            fs::create_dir_all(&directory)
                .unwrap_or_else(|error| panic!("create Dock evidence {directory:?}: {error}"));
            let path = directory.join(format!("{display_id}.count"));
            fs::write(&path, format!("{render_count}\n"))
                .unwrap_or_else(|error| panic!("write Dock render count {path:?}: {error}"));
        });
    }

    #[derive(Clone, Debug, PartialEq)]
    struct DockSurface {
        output: Option<rmac_compositor::OutputId>,
        placement: rmac_shell_settings::DockPlacement,
        reserve_space: bool,
        fullscreen: bool,
        overview_visible: bool,
        shelf_extent: f32,
        /// DockSettings::tile_size (DOCK-01): a change recreates this
        /// output's Dock windows, the same as a placement change does.
        tile_size: f32,
        description: Option<rmac_dock::SurfaceDescription>,
    }

    impl Default for DockSurface {
        fn default() -> Self {
            Self {
                output: None,
                placement: rmac_shell_settings::DockPlacement::Bottom,
                reserve_space: true,
                fullscreen: false,
                overview_visible: false,
                shelf_extent: TileMetrics::default().shelf_thickness,
                tile_size: rmac_shell_settings::DEFAULT_DOCK_TILE_SIZE,
                description: None,
            }
        }
    }

    impl DockSurface {
        fn from_description(
            surface: &rmac_dock::SurfaceDescription,
            fullscreen: bool,
            shelf_extent: f32,
            tile_size: f32,
        ) -> Self {
            Self {
                output: Some(surface.output.clone()),
                placement: surface.placement,
                reserve_space: surface.exclusive_zone > 0.0 && !fullscreen,
                fullscreen,
                overview_visible: surface.overview_visible,
                shelf_extent,
                tile_size,
                description: Some(surface.clone()),
            }
        }
    }

    #[derive(Default)]
    struct DockWindows {
        windows:
            std::collections::BTreeMap<uuid::Uuid, (DockSurface, AnyWindowHandle, AnyWindowHandle)>,
    }

    impl DockWindows {
        fn len(&self) -> usize {
            self.windows.len()
        }

        fn reconcile(
            &mut self,
            desired: Option<&[DockSurface]>,
            status: &Entity<DockStatus>,
            cx: &mut App,
        ) {
            let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
            let desired = desired.map(|surfaces| {
                surfaces
                    .iter()
                    .filter_map(|surface| {
                        surface.output.as_ref().map(|output| {
                            (
                                rmac_shell_layer::stable_output_uuid(output),
                                surface.clone(),
                            )
                        })
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            });
            let available = displays
                .keys()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            let unavailable = self
                .windows
                .keys()
                .filter(|uuid| {
                    !available.contains(uuid)
                        || desired
                            .as_ref()
                            .is_some_and(|desired| !desired.contains_key(uuid))
                })
                .copied()
                .collect::<Vec<_>>();
            for uuid in unavailable {
                if let Some((_, foreground, backdrop)) = self.windows.remove(&uuid) {
                    let _ = foreground.update(cx, |_, window, _| window.remove_window());
                    let _ = backdrop.update(cx, |_, window, _| window.remove_window());
                }
            }
            for (uuid, display) in displays {
                let surface = match &desired {
                    Some(desired) => match desired.get(&uuid) {
                        Some(surface) => surface.clone(),
                        None => continue,
                    },
                    None => DockSurface::default(),
                };
                if self
                    .windows
                    .get(&uuid)
                    .is_some_and(|(current, _, _)| *current == surface)
                {
                    continue;
                }
                let backdrop = open_dock_backdrop(display.clone(), &surface, status.clone(), cx);
                let foreground = open_dock(display, surface.clone(), status.clone(), cx);
                if let Some((_, previous_foreground, previous_backdrop)) =
                    self.windows.insert(uuid, (surface, foreground, backdrop))
                {
                    let _ = previous_foreground.update(cx, |_, window, _| window.remove_window());
                    let _ = previous_backdrop.update(cx, |_, window, _| window.remove_window());
                }
            }
        }
    }

    struct DockBackdrop {
        display_id: u64,
        status: Entity<DockStatus>,
        blurred: bool,
        tile_size: f32,
    }

    impl Render for DockBackdrop {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // An auto-hidden Dock takes its material with it.
            let hidden = self
                .status
                .read(cx)
                .hidden_displays
                .contains(&self.display_id);
            if self.blurred == hidden {
                self.blurred = !hidden;
                window.set_background_appearance(if hidden {
                    WindowBackgroundAppearance::Transparent
                } else {
                    WindowBackgroundAppearance::Blurred
                });
            }
            if hidden {
                return div().size_full();
            }
            // The measured shelf: a faint tint over the compositor blur and
            // a 1 pt rim. macOS draws no shadow under the Dock.
            div()
                .size_full()
                .rounded(px(tokens::dock_shelf_radius(self.tile_size)))
                .bg(rgba(tokens::dock_tint()))
                .border_1()
                .border_color(rgba(tokens::dock_border()))
        }
    }

    fn open_dock_backdrop(
        display: Rc<dyn PlatformDisplay>,
        surface: &DockSurface,
        status: Entity<DockStatus>,
        cx: &mut App,
    ) -> AnyWindowHandle {
        let display_id = display.id();
        let surface_tile_size = surface.tile_size;
        let metrics = TileMetrics::new(surface_tile_size);
        let (anchor, size, margin) = match surface.placement {
            rmac_shell_settings::DockPlacement::Bottom => (
                Anchor::BOTTOM,
                Size::new(px(surface.shelf_extent), px(metrics.shelf_thickness)),
                (px(0.0), px(0.0), px(metrics.shelf_bottom_margin), px(0.0)),
            ),
            rmac_shell_settings::DockPlacement::Left => (
                Anchor::LEFT,
                Size::new(px(metrics.shelf_thickness), px(surface.shelf_extent)),
                (px(0.0), px(0.0), px(0.0), px(metrics.shelf_bottom_margin)),
            ),
            rmac_shell_settings::DockPlacement::Right => (
                Anchor::RIGHT,
                Size::new(px(metrics.shelf_thickness), px(surface.shelf_extent)),
                (px(0.0), px(metrics.shelf_bottom_margin), px(0.0), px(0.0)),
            ),
        };
        cx.open_window(
            WindowOptions {
                titlebar: None,
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size,
                })),
                display_id: Some(display_id),
                app_id: Some("dev.rmac.DockMaterial".to_owned()),
                window_background: WindowBackgroundAppearance::Blurred,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: format!("rmac-dock-material-{}", u64::from(display_id)),
                    layer: Layer::Top,
                    anchor,
                    margin: Some(margin),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    // Ignore the foreground Dock's reserved work area; both
                    // surfaces must occupy the same physical shelf bounds.
                    exclusive_zone: Some(px(-1.0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| {
                    cx.observe(&status, |_, _, cx| cx.notify()).detach();
                    DockBackdrop {
                        display_id: u64::from(display_id),
                        status,
                        blurred: true,
                        tile_size: surface_tile_size,
                    }
                })
            },
        )
        .expect("open Dock material layer surface")
        .into()
    }

    fn open_dock(
        display: Rc<dyn PlatformDisplay>,
        surface: DockSurface,
        status: Entity<DockStatus>,
        cx: &mut App,
    ) -> AnyWindowHandle {
        let display_id = display.id();
        let display_size = display.bounds().size;
        let metrics = TileMetrics::new(surface.tile_size);
        let anchor = match surface.placement {
            rmac_shell_settings::DockPlacement::Bottom => {
                Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT
            }
            rmac_shell_settings::DockPlacement::Left => Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT,
            rmac_shell_settings::DockPlacement::Right => {
                Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM
            }
        };
        let exclusive_zone = surface.reserve_space.then_some(px(metrics.exclusive_zone));
        let handle = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: display_size,
                    })),
                    display_id: Some(display_id),
                    app_id: Some("dev.rmac.Dock".to_owned()),
                    // This interaction layer spans the output so menus can
                    // escape the shelf. Blur belongs to the bounded backdrop
                    // surface, never to this transparent host.
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: format!("rmac-dock-{}", u64::from(display_id)),
                        layer: Layer::Top,
                        anchor,
                        keyboard_interactivity: KeyboardInteractivity::None,
                        exclusive_zone,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                {
                    let status = status.clone();
                    move |_, cx| cx.new(|cx| Dock::new(display_id, surface, status, cx))
                },
            )
            .expect("open Dock layer surface");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = handle.update(cx, |_, window, _| {
                record_configured_surface(window, u64::from(display_id));
            });
        })
        .detach();
        handle.into()
    }

    type LauncherSignal = (String, rmac_dock::badges::LauncherUpdate);

    /// Follow `com.canonical.Unity.LauncherEntry` badges, progress and
    /// attention requests on the session bus, reconnecting after failures.
    async fn watch_launcher_entries(sender: async_channel::Sender<LauncherSignal>) {
        loop {
            if let Err(error) = watch_launcher_entries_once(&sender).await {
                eprintln!("Dock badges are unavailable: {error}");
            }
            if sender.is_closed() {
                return;
            }
            async_io::Timer::after(Duration::from_secs(5)).await;
        }
    }

    async fn watch_launcher_entries_once(
        sender: &async_channel::Sender<LauncherSignal>,
    ) -> zbus::Result<()> {
        use futures_util::StreamExt as _;
        let connection = zbus::Connection::session().await?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("com.canonical.Unity.LauncherEntry")?
            .member("Update")?
            .build();
        let mut stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(64)).await?;
        while let Some(message) = stream.next().await {
            let message = message?;
            let Ok((app_uri, properties)) = message.body().deserialize::<(
                String,
                std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            )>() else {
                continue;
            };
            if sender
                .send((app_uri, launcher_update(&properties)))
                .await
                .is_err()
            {
                return Ok(());
            }
        }
        Ok(())
    }

    fn launcher_update(
        properties: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> rmac_dock::badges::LauncherUpdate {
        let value = |key: &str| properties.get(key).map(|value| &**value);
        let flag = |key: &str| value(key).and_then(|value| bool::try_from(value).ok());
        rmac_dock::badges::LauncherUpdate {
            count: value("count").and_then(|value| {
                i64::try_from(value)
                    .ok()
                    .or_else(|| i32::try_from(value).ok().map(i64::from))
                    .or_else(|| u32::try_from(value).ok().map(i64::from))
            }),
            count_visible: flag("count-visible"),
            progress: value("progress").and_then(|value| f64::try_from(value).ok()),
            progress_visible: flag("progress-visible"),
            urgent: flag("urgent"),
        }
    }

    /// ⌃F3 from niri: give the keyboard to the Dock on the focused output
    /// (or the first Dock when niri reports no focused output).
    /// ⌥⌘D (DOCK-04) and the separator menu's Turn Hiding On/Off row: flip
    /// `DockSettings::autohide`. The resident Dock picks the change up the
    /// same way it does any other live settings change (no extra plumbing:
    /// `rmac-dock-runtime` already watches the same store).
    fn toggle_dock_hiding(cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let result = blocking::unblock(|| {
                    let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
                    let mut settings = store.load()?.settings;
                    settings.dock.autohide = !settings.dock.autohide;
                    store.save(&settings)
                })
                .await;
                if let Err(error) = result {
                    eprintln!("could not turn Dock hiding on or off: {error}");
                }
            })
            .detach();
    }

    fn focus_dock(windows: &DockWindows, status: &Entity<DockStatus>, cx: &mut App) {
        let focused_output = status.read(cx).snapshot().and_then(|snapshot| {
            snapshot
                .compositor
                .focus
                .output
                .as_ref()
                .map(rmac_shell_layer::stable_output_uuid)
        });
        let handle = focused_output
            .and_then(|uuid| windows.windows.get(&uuid))
            .or_else(|| windows.windows.values().next())
            .map(|(_, foreground, _)| *foreground);
        let Some(dock) = handle.and_then(|handle| handle.downcast::<Dock>()) else {
            return;
        };
        let _ = dock.update(cx, |dock, window, cx| dock.begin_keyboard(window, cx));
    }

    /// A drag out of Apps (crates/app-drawer): `Hover` previews where it
    /// would land (item 7's live insertion gap), `Drop` keeps the
    /// application in whichever Dock the drag reached, `Cancel` clears the
    /// preview. This is a hint, not an authority (drag_endpoint's own doc
    /// comment): `Drop` still resolves and places the application through
    /// the same catalog/PinCommand path a `.desktop` file drop uses.
    fn apply_drag_command(
        windows: &DockWindows,
        command: crate::drag_endpoint::Command,
        cx: &mut App,
    ) {
        let Some(dock) = windows
            .windows
            .values()
            .next()
            .map(|(_, foreground, _)| *foreground)
            .and_then(|handle| handle.downcast::<Dock>())
        else {
            return;
        };
        let _ = dock.update(cx, |dock, _window, cx| match command {
            crate::drag_endpoint::Command::Hover { fraction, .. } => {
                if dock.apps_drag_hover != Some(fraction) {
                    dock.apps_drag_hover = Some(fraction);
                    cx.notify();
                }
            }
            crate::drag_endpoint::Command::Drop { app_id, fraction } => {
                if dock.apps_drag_hover.take().is_some() {
                    cx.notify();
                }
                dock.keep_dragged_application(app_id, fraction, cx);
            }
            crate::drag_endpoint::Command::Cancel { .. } => {
                if dock.apps_drag_hover.take().is_some() {
                    cx.notify();
                }
            }
        });
    }

    /// Receive Apps' drag-hint datagrams (`drag_endpoint`) on a thread; the
    /// Dock keeps running without drag-to-keep from Apps if the socket
    /// cannot be bound.
    fn listen_for_drag_commands() -> Option<async_channel::Receiver<crate::drag_endpoint::Command>>
    {
        let listener = match crate::drag_endpoint::Listener::bind() {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Dock drag-from-Apps endpoint is unavailable: {error}");
                return None;
            }
        };
        let (command_tx, command_rx) = async_channel::bounded(8);
        let spawned = std::thread::Builder::new()
            .name("rmac-dock-drag".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(command) => {
                        if command_tx.send_blocking(command).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("Dock drag-from-Apps endpoint stopped: {error}");
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not start the Dock drag-from-Apps endpoint: {error}");
            return None;
        }
        Some(command_rx)
    }

    /// Receive `rmac-dock focus` datagrams on a thread; the Dock keeps
    /// running without ⌃F3 if the socket cannot be bound.
    fn listen_for_commands() -> Option<async_channel::Receiver<crate::ipc::Command>> {
        let listener = match crate::ipc::Listener::bind() {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Dock keyboard access (⌃F3) is unavailable: {error}");
                return None;
            }
        };
        let (command_tx, command_rx) = async_channel::bounded(8);
        let spawned = std::thread::Builder::new()
            .name("rmac-dock-ipc".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(command) => {
                        if command_tx.send_blocking(command).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("Dock command endpoint stopped: {error}");
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not start the Dock command endpoint: {error}");
            return None;
        }
        Some(command_rx)
    }

    /// `rmac-dock` runs the Dock; `rmac-dock focus` (niri's ⌃F3 bind) asks
    /// the running Dock to take keyboard focus and exits.
    pub fn run() {
        let arguments = env::args().skip(1).collect::<Vec<_>>();
        match arguments.as_slice() {
            [] => run_service(),
            [command] => match crate::ipc::Command::parse(command) {
                Some(command) => {
                    if let Err(error) = crate::ipc::send(command) {
                        eprintln!("the Dock is not running: {error}");
                        std::process::exit(1);
                    }
                }
                None => {
                    eprintln!("usage: dock [focus]");
                    std::process::exit(2);
                }
            },
            _ => {
                eprintln!("usage: dock [focus]");
                std::process::exit(2);
            }
        }
    }

    fn run_service() {
        let commands = listen_for_commands();
        let drag_commands = listen_for_drag_commands();
        let app = application().with_quit_mode(QuitMode::Explicit);
        app.run(|cx: &mut App| {
            rmac_shell_ui::tokens::install_appearance_watch(cx);
            let (runtime_tx, runtime_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_dock_runtime::watch(runtime_tx).await {
                        eprintln!("Dock runtime stopped: {error}");
                    }
                })
                .detach();
            let (reconcile_tx, reconcile_rx) = async_channel::bounded(1);
            let status = cx.new(|cx| DockStatus::new(runtime_rx, reconcile_tx.clone(), cx));
            let (launcher_tx, launcher_rx) = async_channel::bounded::<LauncherSignal>(64);
            cx.background_executor()
                .spawn(watch_launcher_entries(launcher_tx))
                .detach();
            let launcher_status = status.clone();
            cx.spawn(async move |cx| {
                while let Ok((app_uri, update)) = launcher_rx.recv().await {
                    launcher_status.update(cx, |status, cx| {
                        if status.launcher.apply(&app_uri, update) {
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
            let _ = reconcile_tx.try_send(());
            let windows = Rc::new(std::cell::RefCell::new(DockWindows::default()));
            if let Some(commands) = commands {
                let windows = windows.clone();
                let status = status.clone();
                cx.spawn(async move |cx| {
                    while let Ok(command) = commands.recv().await {
                        cx.update(|cx| match command {
                            crate::ipc::Command::Focus => {
                                focus_dock(&windows.borrow(), &status, cx);
                            }
                            crate::ipc::Command::ToggleHide => {
                                toggle_dock_hiding(cx);
                            }
                        });
                    }
                })
                .detach();
            }
            if let Some(drag_commands) = drag_commands {
                let windows = windows.clone();
                cx.spawn(async move |cx| {
                    while let Ok(command) = drag_commands.recv().await {
                        let _ = cx.update(|cx| apply_drag_command(&windows.borrow(), command, cx));
                    }
                })
                .detach();
            }
            cx.spawn(async move |cx| {
                while reconcile_rx.recv().await.is_ok() {
                    loop {
                        let complete = cx.update(|cx| {
                            let surfaces = status.read(cx).surfaces();
                            if let Some(surfaces) = surfaces {
                                if !surfaces.is_empty() || !status.read(cx).needs_display_fallback()
                                {
                                    let expected = surfaces.len();
                                    let mut windows = windows.borrow_mut();
                                    windows.reconcile(Some(&surfaces), &status, cx);
                                    return windows.len() == expected;
                                }
                            }
                            if status.read(cx).needs_display_fallback() {
                                let expected =
                                    rmac_shell_layer::output_surfaces::newest_displays(cx).len();
                                let mut windows = windows.borrow_mut();
                                windows.reconcile(None, &status, cx);
                                return windows.len() == expected;
                            }
                            false
                        });
                        if complete {
                            break;
                        }
                        let update = reconcile_rx.recv().fuse();
                        let retry = cx
                            .background_executor()
                            .timer(Duration::from_millis(50))
                            .fuse();
                        futures_util::pin_mut!(update, retry);
                        futures_util::select! {
                            update = update => if update.is_err() { return },
                            _ = retry => {}
                        }
                    }
                }
            })
            .detach();
        });
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("Dock requires Linux and: cargo run --features wayland --bin dock");
    std::process::exit(2);
}
