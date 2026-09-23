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
        canvas, div, img, layer_shell::*, point, prelude::*, px, rgba, AnyWindowHandle, App,
        Bounds, Context, DisplayId, Entity, ExternalPaths, FontWeight, MouseButton, PathBuilder,
        PlatformDisplay, QuitMode, Role, Size, Window, WindowBackgroundAppearance, WindowBounds,
        WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_shell_ui::tokens;

    // Measured from the owner's Mac 2026-09-23 and drawn in
    // design-lab/dock.html. ICON_SIZE is the Dock tile (the icon canvas, the
    // macOS "Size" preference); everything else is a measured ratio of it.
    const ICON_SIZE: f32 = 64.0;
    // Pitch 68: tiles sit 4 apart (their visible squircles 16 apart).
    const ICON_GAP: f32 = ICON_SIZE * 0.0625;
    // Tile → shelf rim on every side (16 from the visible squircle).
    const SHELF_PADDING: f32 = ICON_SIZE * 0.15625;
    const SHELF_THICKNESS: f32 = ICON_SIZE + 2.0 * SHELF_PADDING;
    // Shelf rim → screen edge.
    const SHELF_BOTTOM_MARGIN: f32 = ICON_SIZE * 0.078125;
    const EXCLUSIVE_ZONE: f32 = SHELF_THICKNESS + SHELF_BOTTOM_MARGIN;
    // The macOS icon grid shows a 52-of-64 squircle; rmac art draws its
    // squircle at 824/1024 of the image, so the image is scaled to match.
    const ICON_SQUIRCLE: f32 = 0.8125;
    const ICON_ART_SCALE: f32 = ICON_SQUIRCLE * 1024.0 / 824.0;
    // Separator: a 1 × 62 line centred in the shelf with 13 either side
    // (plus the tile gap), so the pitch across it is 99.
    const SEPARATOR_WIDTH: f32 = 1.0;
    const SEPARATOR_LENGTH: f32 = ICON_SIZE * 0.96875;
    const SEPARATOR_MARGIN: f32 = ICON_SIZE * 0.203125;
    const SEPARATOR_SLOT: f32 = SEPARATOR_WIDTH + 2.0 * SEPARATOR_MARGIN;
    // Running dot: 4 across, its centre 4 below the tile.
    const INDICATOR_SIZE: f32 = ICON_SIZE * 0.0625;
    const INDICATOR_OFFSET: f32 = ICON_SIZE * 0.03125;
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
    const TOOLTIP_BOTTOM: f32 = EXCLUSIVE_ZONE + 6.0;
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
                        )
                    })
                    .collect()
            })
        }
    }

    fn dock_shelf_extent(snapshot: &rmac_dock_runtime::Snapshot) -> f32 {
        let entries = snapshot.content.applications.len();
        let minimized = snapshot
            .content
            .places
            .iter()
            .filter(|entry| matches!(entry.id, rmac_dock::presentation::EntryId::Minimized(_)))
            .count();
        let pinned = snapshot
            .model
            .items
            .iter()
            .take_while(|item| item.pinned)
            .count();
        let separates_running = pinned > 0 && pinned < entries;
        let separators = usize::from(separates_running) + usize::from(entries > 0);
        let items = entries + minimized + 1;
        let children = items + separators;
        ICON_SIZE * items as f32
            + SEPARATOR_SLOT * separators as f32
            + ICON_GAP * children.saturating_sub(1) as f32
            + 2.0 * SHELF_PADDING
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

    struct DockMenu {
        anchor: f32,
        session: rmac_dock::menu::Session,
        /// Options ▸ is open (the pointer rested on its parent row).
        submenu_open: bool,
        /// Options ▸ Open at Login, once the XDG autostart state is read.
        login: Option<LoginOption>,
    }

    /// The app's XDG autostart state behind Options ▸ Open at Login.
    #[derive(Clone)]
    struct LoginOption {
        desktop_entry: PathBuf,
        /// Autostart entry ID: the desktop file name.
        id: String,
        enabled: bool,
    }

    struct Dock {
        display_id: u64,
        output: Option<rmac_compositor::OutputId>,
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
        /// Option held (read from pointer events; the Dock never takes the
        /// keyboard): Dock menus show Force Quit instead of Quit.
        option_held: bool,
        /// Reviewed Trash contents waiting for the Empty Trash alert.
        trash_review: Option<rmac_dock_system::dispatch::ReviewedTrash>,
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
                output: surface.output.clone(),
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
            }
        }

        fn now_ms(&self) -> u64 {
            u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        /// Distance of a surface point beyond the shelf's inner edge (towards
        /// the screen centre); negative inside the shelf.
        fn lift_of(&self, x: f32, y: f32) -> f32 {
            let (width, height) = self.surface_size;
            let depth = SHELF_BOTTOM_MARGIN + SHELF_THICKNESS;
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
            self.shelf_start
                + SHELF_PADDING
                + ICON_SIZE / 2.0
                + index as f32 * (ICON_SIZE + ICON_GAP)
        }

        fn begin_tile_drag(
            &mut self,
            app_id: &str,
            icon: Option<PathBuf>,
            position: (f32, f32),
            cx: &mut Context<Self>,
        ) {
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
            let Some(drag) =
                rmac_dock::reorder::TileDrag::begin(source, centers, axis, lift, ICON_SIZE, true)
            else {
                return;
            };
            // Where the pointer sits inside the icon, so the lifted icon
            // does not jump to the pointer.
            let center = self.pinned_center(source);
            let (width, height) = self.surface_size;
            let tile_origin = match self.placement {
                rmac_shell_settings::DockPlacement::Bottom => (
                    center - ICON_SIZE / 2.0,
                    height - SHELF_BOTTOM_MARGIN - SHELF_PADDING - ICON_SIZE,
                ),
                rmac_shell_settings::DockPlacement::Left => (
                    SHELF_BOTTOM_MARGIN + SHELF_PADDING,
                    center - ICON_SIZE / 2.0,
                ),
                rmac_shell_settings::DockPlacement::Right => (
                    width - SHELF_BOTTOM_MARGIN - SHELF_PADDING - ICON_SIZE,
                    center - ICON_SIZE / 2.0,
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
                let pitch = ICON_SIZE + ICON_GAP;
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

        fn finish_tile_drag(&mut self, platform: bool, cx: &mut Context<Self>) {
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
                    self.activate_entry(&ui.app_id, platform, cx);
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
                    if this.hide_generation == generation && !this.pointer_inside {
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
        /// otherwise launch/focus through the system authority.
        fn activate_entry(&mut self, app_id: &str, platform: bool, cx: &mut Context<Self>) {
            if platform {
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
            cx.spawn(async move |_, cx| {
                let completion = execution.await;
                status.update(cx, |status, cx| {
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
                });
            })
            .detach();
        }
    }

    impl Render for Dock {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let (dock_settings, model, mut entries, content) = {
                let status = self.status.read(cx);
                let snapshot = status
                    .snapshot()
                    .expect("a Dock surface is opened only after runtime readiness");
                (
                    snapshot.settings.clone(),
                    snapshot.model.clone(),
                    snapshot.content.applications.clone(),
                    snapshot.content.clone(),
                )
            };
            self.content = content;
            // Launch and attention bounces follow the authoritative model:
            // a window appearing ends a launch, urgency asks for attention.
            let now = self.now_ms();
            let running_apps: std::collections::BTreeSet<String> = model
                .items
                .iter()
                .filter(|item| item.running)
                .map(|item| item.id.clone())
                .collect();
            let attention_apps: std::collections::BTreeSet<String> = model
                .items
                .iter()
                .filter(|item| item.urgent && !item.active)
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
            let trash = model
                .special_items
                .iter()
                .find(|item| item.kind == rmac_dock::SpecialItemKind::Trash);
            let trash_full = trash
                .and_then(|trash| trash.item_count)
                .is_some_and(|count| count > 0);
            let trash_label = match trash.and_then(|trash| trash.item_count) {
                Some(0) => "Trash, empty".into(),
                Some(1) => "Trash, 1 item".into(),
                Some(count) => format!("Trash, {count} items"),
                None => "Trash unavailable".into(),
            };
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
            // Minimized tiles sit between the application group and Trash.
            let item_count = entries.len() + minimized_entries.len() + 1;
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
            let shelf_extent = ICON_SIZE * item_count as f32
                + SEPARATOR_SLOT * separator_count as f32
                + ICON_GAP * child_count.saturating_sub(1) as f32
                + 2.0 * SHELF_PADDING;
            let shelf_start = (axis - shelf_extent) / 2.0;
            self.shelf_start = shelf_start;
            self.surface_size = (surface_width, surface_height);
            let trash_center = shelf_extent - SHELF_PADDING - ICON_SIZE / 2.0;
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
                .is_some_and(|ui| ui.drag.is_active());
            let modal = menu_geometry.is_some() || self.trash_review.is_some() || dragging;
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
                        origin: point(px(shelf_start), px(surface_height - EXCLUSIVE_ZONE)),
                        size: Size::new(px(shelf_extent), px(EXCLUSIVE_ZONE)),
                    },
                    (rmac_shell_settings::DockPlacement::Left, false) => Bounds {
                        origin: point(px(0.0), px(shelf_start)),
                        size: Size::new(px(EXCLUSIVE_ZONE), px(shelf_extent)),
                    },
                    (rmac_shell_settings::DockPlacement::Right, false) => Bounds {
                        origin: point(
                            px(f32::from(window_size.width) - EXCLUSIVE_ZONE),
                            px(shelf_start),
                        ),
                        size: Size::new(px(EXCLUSIVE_ZONE), px(shelf_extent)),
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
            let tooltip = (!self.hidden && self.context_menu.is_none() && !dragging)
                .then_some(self.hovered_item.as_ref())
                .flatten()
                .map(|(relative_center, label)| {
                    let icon_center = shelf_start + *relative_center;
                    let tooltip_bottom = TOOLTIP_BOTTOM.max(
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
                            .left(px(EXCLUSIVE_ZONE + 8.0))
                            .top(px(icon_center - 18.0)),
                        rmac_shell_settings::DockPlacement::Right => tooltip
                            .right(px(EXCLUSIVE_ZONE + 8.0))
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
                .aria_label("rmac Dock")
                .size_full()
                .relative()
                .flex()
                .font_features(rmac_shell_ui::tabular_font_features())
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.context_menu.take().is_some() {
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
                        this.finish_tile_drag(event.modifiers.platform, cx);
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
                    .pb(px(SHELF_BOTTOM_MARGIN)),
                rmac_shell_settings::DockPlacement::Left => root
                    .items_start()
                    .justify_center()
                    .pl(px(SHELF_BOTTOM_MARGIN)),
                rmac_shell_settings::DockPlacement::Right => root
                    .items_end()
                    .justify_center()
                    .pr(px(SHELF_BOTTOM_MARGIN)),
            };
            let shelf = div()
                .flex()
                .gap(px(ICON_GAP))
                .p(px(SHELF_PADDING))
                .rounded(px(tokens::dock_shelf_radius(ICON_SIZE)))
                .bg(rgba(tokens::transparent()))
                .relative()
                .opacity(if hide_progress >= 1.0 { 0.0 } else { 1.0 });
            // Auto-hide slides the shelf off its screen edge.
            let slide = hide_progress * EXCLUSIVE_ZONE;
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
            // An application dropped anywhere else on the shelf is kept.
            shelf = shelf.on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.keep_dropped_applications(paths.paths().to_vec(), cx);
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
                        - (minimized_entries.len() - index) as f32 * (ICON_SIZE + ICON_GAP);
                    let visual_size = magnified_icon_size(
                        center,
                        self.hovered_item.as_ref().map(|(center, _)| *center),
                        &dock_settings,
                    );
                    let visual_offset = (ICON_SIZE - visual_size) / 2.0;
                    let thumbnail = minimized_icon_path(entry);
                    let badge = minimized_badge_path(entry);
                    // Prefer the captured thumbnail; without one, show the
                    // application icon rather than a bare letter.
                    let main_icon = thumbnail.clone().or_else(|| badge.clone());
                    let badge_overlay = thumbnail.is_some().then(|| badge.clone()).flatten();
                    let label = entry.label.clone();
                    let tooltip_label = entry.label.clone();
                    let mut tile = div()
                        .id(format!("dock-minimized-{}-{index}", self.display_id))
                        .role(Role::Button)
                        .aria_label(entry.accessible_label.clone())
                        .relative()
                        .w(px(ICON_SIZE))
                        .h(px(ICON_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgba(tokens::primary_text()))
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .rounded(px(tokens::dock_tile_radius(ICON_SIZE)))
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
            root.child(
                shelf
                    .children(entries.into_iter().enumerate().flat_map(|(index, entry)| {
                        let relative_center = SHELF_PADDING
                            + ICON_SIZE / 2.0
                            + index as f32 * (ICON_SIZE + ICON_GAP)
                            + if separates_running && index >= pinned_count {
                                SEPARATOR_SLOT + ICON_GAP
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
                        let running =
                            entry.activity != rmac_dock::presentation::ActivityIndicator::None;
                        let icon_path = item_icon_path(&entry.icon, &app_id);
                        let tooltip_label = entry.label.clone();
                        let visual_size = magnified_icon_size(
                            relative_center,
                            self.hovered_item.as_ref().map(|(center, _)| *center),
                            &dock_settings,
                        );
                        let visual_offset = (ICON_SIZE - visual_size) / 2.0;
                        let lift = self.bounces.lift(&app_id, now, ICON_SIZE);
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
                            .w(px(ICON_SIZE))
                            .h(px(ICON_SIZE))
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
                                                        event.modifiers.platform,
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
                        if menu_open || self.pressed.as_deref() == Some(app_id.as_str()) {
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
                        item = item.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let session = {
                                    let status = this.status.read(cx);
                                    status.model().and_then(|model| {
                                        model.context_menu(&context_app_id).and_then(|menu| {
                                            rmac_dock::menu::Session::context(&menu).ok()
                                        })
                                    })
                                };
                                this.context_menu = session.map(|session| DockMenu {
                                    anchor: relative_center,
                                    session,
                                    submenu_open: false,
                                    login: None,
                                });
                                this.input_region = None;
                                this.load_login_state(&context_app_id, cx);
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
                                .w(px(INDICATOR_SIZE))
                                .h(px(INDICATOR_SIZE))
                                .rounded_full()
                                .bg(rgba(tokens::dock_indicator()));
                            let outside = -(INDICATOR_OFFSET + INDICATOR_SIZE);
                            let along = (ICON_SIZE - INDICATOR_SIZE) / 2.0;
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
                        // tracker), never a dot or badge.
                        let mut children: Vec<gpui::AnyElement> = Vec::with_capacity(2);
                        if separates_running && index == pinned_count {
                            children.push(dock_separator(self.placement));
                        }
                        children.push(item.into_any_element());
                        children
                    }))
                    .when(!model.items.is_empty(), |shelf| {
                        shelf.child(dock_separator(self.placement))
                    })
                    .children(minimized_children)
                    .child({
                        let visual_size = magnified_icon_size(
                            trash_center,
                            self.hovered_item.as_ref().map(|(center, _)| *center),
                            &dock_settings,
                        );
                        let visual_offset = (ICON_SIZE - visual_size) / 2.0;
                        let mut trash = div()
                            .id(format!("dock-trash-{}", self.display_id))
                            .role(Role::Button)
                            .aria_label(trash_label)
                            .relative()
                            .w(px(ICON_SIZE))
                            .h(px(ICON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(tokens::dock_tile_radius(ICON_SIZE)))
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
                            let image = img(path)
                                .absolute()
                                .w(px(art))
                                .h(px(art))
                                .when(menu_anchor == Some(trash_center), |image| {
                                    image.opacity(0.47)
                                });
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
                        let art = ICON_SIZE * ICON_ART_SCALE;
                        let inset = (ICON_SIZE - art) / 2.0;
                        div()
                            .absolute()
                            .left(px(left))
                            .top(px(top))
                            .w(px(ICON_SIZE))
                            .h(px(ICON_SIZE))
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
                                        .left(px(ICON_SIZE / 2.0 - TOOLTIP_WIDTH / 2.0))
                                        .bottom(px(ICON_SIZE + 12.0))
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
                let art = ICON_SIZE * ICON_ART_SCALE;
                let inset = (ICON_SIZE - art) / 2.0;
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
                axis,
                self.display_id,
                self.option_held,
                window,
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
                    let authorized = match &action {
                        rmac_dock::menu::Action::Context(action) => this
                            .status
                            .read(cx)
                            .model()
                            .is_some_and(|model| model.authorizes_context_action(action)),
                        rmac_dock::menu::Action::ActivateEntry(_)
                        | rmac_dock::menu::Action::SpecialContext(_) => true,
                    };
                    this.context_menu = None;
                    this.input_region = None;
                    if let rmac_dock::menu::Action::Context(rmac_dock::ContextAction::LaunchNew {
                        app_id,
                        ..
                    }) = &action
                    {
                        if authorized {
                            this.note_launch(app_id);
                        }
                    }
                    if authorized {
                        this.dispatch_action(action, cx);
                    } else {
                        eprintln!("the selected Dock command is no longer current");
                    }
                    cx.notify();
                }));
        }
        element.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_context_menu(
        menu: Option<&DockMenu>,
        placement: rmac_shell_settings::DockPlacement,
        geometry: Option<(f32, f32, f32)>,
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
        let offset = EXCLUSIVE_ZONE + MENU_SHELF_GAP;
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

    fn dock_separator(placement: rmac_shell_settings::DockPlacement) -> gpui::AnyElement {
        // Centred on the tile row (1 inside it at each end), 13 clear of the
        // tile gaps on either side.
        let separator = div().bg(rgba(tokens::dock_separator()));
        let inset = (ICON_SIZE - SEPARATOR_LENGTH) / 2.0;
        match placement {
            rmac_shell_settings::DockPlacement::Bottom => separator
                .w(px(SEPARATOR_WIDTH))
                .h(px(SEPARATOR_LENGTH))
                .mx(px(SEPARATOR_MARGIN))
                .mb(px(inset))
                .into_any_element(),
            rmac_shell_settings::DockPlacement::Left
            | rmac_shell_settings::DockPlacement::Right => separator
                .w(px(SEPARATOR_LENGTH))
                .h(px(SEPARATOR_WIDTH))
                .my(px(SEPARATOR_MARGIN))
                .into_any_element(),
        }
    }

    fn magnified_icon_size(
        center: f32,
        pointer: Option<f32>,
        settings: &rmac_shell_settings::DockSettings,
    ) -> f32 {
        if !settings.magnification {
            return ICON_SIZE;
        }
        let config = rmac_dock::motion::MagnificationConfig {
            icon_size: ICON_SIZE,
            maximum_scale: settings.magnification_scale,
            ..Default::default()
        };
        let pointer = pointer.map(|pointer| ICON_SIZE / 2.0 + pointer - center);
        rmac_dock::motion::magnified_layout(1, pointer, true, false, config)
            .ok()
            .and_then(|layout| layout.items.first().map(|item| item.size))
            .unwrap_or(ICON_SIZE)
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
                shelf_extent: SHELF_THICKNESS,
                description: None,
            }
        }
    }

    impl DockSurface {
        fn from_description(
            surface: &rmac_dock::SurfaceDescription,
            fullscreen: bool,
            shelf_extent: f32,
        ) -> Self {
            Self {
                output: Some(surface.output.clone()),
                placement: surface.placement,
                reserve_space: surface.exclusive_zone > 0.0 && !fullscreen,
                fullscreen,
                overview_visible: surface.overview_visible,
                shelf_extent,
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
                .rounded(px(tokens::dock_shelf_radius(ICON_SIZE)))
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
        let (anchor, size, margin) = match surface.placement {
            rmac_shell_settings::DockPlacement::Bottom => (
                Anchor::BOTTOM,
                Size::new(px(surface.shelf_extent), px(SHELF_THICKNESS)),
                (px(0.0), px(0.0), px(SHELF_BOTTOM_MARGIN), px(0.0)),
            ),
            rmac_shell_settings::DockPlacement::Left => (
                Anchor::LEFT,
                Size::new(px(SHELF_THICKNESS), px(surface.shelf_extent)),
                (px(0.0), px(0.0), px(0.0), px(SHELF_BOTTOM_MARGIN)),
            ),
            rmac_shell_settings::DockPlacement::Right => (
                Anchor::RIGHT,
                Size::new(px(SHELF_THICKNESS), px(surface.shelf_extent)),
                (px(0.0), px(SHELF_BOTTOM_MARGIN), px(0.0), px(0.0)),
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
        let anchor = match surface.placement {
            rmac_shell_settings::DockPlacement::Bottom => {
                Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT
            }
            rmac_shell_settings::DockPlacement::Left => Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT,
            rmac_shell_settings::DockPlacement::Right => {
                Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM
            }
        };
        let exclusive_zone = surface.reserve_space.then_some(px(EXCLUSIVE_ZONE));
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

    pub fn run() {
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
            let _ = reconcile_tx.try_send(());
            cx.spawn(async move |cx| {
                let mut windows = DockWindows::default();
                while reconcile_rx.recv().await.is_ok() {
                    loop {
                        let complete = cx.update(|cx| {
                            let surfaces = status.read(cx).surfaces();
                            let Some(surfaces) = surfaces else {
                                return false;
                            };
                            let expected = surfaces.len();
                            windows.reconcile(Some(&surfaces), &status, cx);
                            windows.len() == expected
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
