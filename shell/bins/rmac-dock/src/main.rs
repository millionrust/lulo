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
        Bounds, Context, DisplayId, Entity, FontWeight, MouseButton, PathBuilder, PlatformDisplay,
        QuitMode, Role, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
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
    // squircle at 112/128 of the image, so the image is scaled to match.
    const ICON_SQUIRCLE: f32 = 0.8125;
    const ICON_ART_SCALE: f32 = ICON_SQUIRCLE * 128.0 / 112.0;
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

    struct DockMenu {
        anchor: f32,
        session: rmac_dock::menu::Session,
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
        fullscreen: bool,
        overview_visible: bool,
        visibility_policy: Option<(bool, bool)>,
        hide_generation: u64,
        surface_description: Option<rmac_dock::SurfaceDescription>,
        content: rmac_dock::presentation::ShelfContent,
        drag: Option<rmac_dock::drag::DragSession>,
        drag_order: Option<Vec<String>>,
    }

    impl Dock {
        fn new(
            display_id: DisplayId,
            surface: DockSurface,
            status: Entity<DockStatus>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
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
                hidden: surface.fullscreen && !surface.overview_visible,
                fullscreen: surface.fullscreen,
                overview_visible: surface.overview_visible,
                visibility_policy: None,
                hide_generation: 0,
                surface_description: surface.description.clone(),
                content: rmac_dock::presentation::ShelfContent::default(),
                drag: None,
                drag_order: None,
            }
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
                        this.hidden = true;
                        this.hovered_item = None;
                        this.input_region = None;
                        cx.notify();
                    }
                });
            })
            .detach();
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
                self.dispatch_action(
                    rmac_dock::menu::Action::ActivateEntry(
                        rmac_dock::presentation::EntryId::Application(app_id.to_owned()),
                    ),
                    cx,
                );
            }
        }

        fn dispatch_action(&mut self, action: rmac_dock::menu::Action, cx: &mut Context<Self>) {
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
            let (dock_settings, model, mut entries, content, description) = {
                let status = self.status.read(cx);
                let snapshot = status
                    .snapshot()
                    .expect("a Dock surface is opened only after runtime readiness");
                let description = snapshot.surface_plan.as_ref().ok().and_then(|surfaces| {
                    surfaces
                        .iter()
                        .find(|surface| Some(&surface.output) == self.output.as_ref())
                        .cloned()
                });
                (
                    snapshot.settings.clone(),
                    snapshot.model.clone(),
                    snapshot.content.applications.clone(),
                    snapshot.content.clone(),
                    description,
                )
            };
            self.content = content;
            self.surface_description = description;
            let effective_autohide = dock_settings.autohide || self.fullscreen;
            let visibility_policy = (effective_autohide, self.overview_visible);
            if self.visibility_policy != Some(visibility_policy) {
                self.visibility_policy = Some(visibility_policy);
                if self.overview_visible || !effective_autohide {
                    self.hide_generation = self.hide_generation.saturating_add(1);
                    self.hidden = false;
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
            if let Some(order) = self.drag_order.clone() {
                if pinned_count > 0 {
                    entries[..pinned_count].sort_by_key(|entry| match &entry.id {
                        rmac_dock::presentation::EntryId::Application(app_id) => order
                            .iter()
                            .position(|candidate| candidate == app_id)
                            .unwrap_or(usize::MAX),
                        _ => usize::MAX,
                    });
                }
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
            let input_region = (
                shelf_start,
                shelf_extent,
                self.hidden,
                menu_geometry.is_some(),
            );
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
                let regions = if menu_geometry.is_some() {
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
            let tooltip = (!self.hidden)
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
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.pointer_inside = *hovered;
                    this.hide_generation = this.hide_generation.saturating_add(1);
                    if *hovered {
                        if this.hidden {
                            this.hidden = false;
                            this.input_region = None;
                            cx.notify();
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
                .opacity(if self.hidden { 0.0 } else { 1.0 });
            let mut shelf = if horizontal {
                shelf.items_end()
            } else {
                shelf.flex_col().items_center()
            };
            // The shelf owns pointer frames for an in-progress tile drag, so the
            // reorder keeps tracking even when the pointer leaves the tile.
            shelf = shelf
                .on_mouse_move(
                    cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        let axis = match this.placement {
                            rmac_shell_settings::DockPlacement::Bottom => {
                                f32::from(event.position.x)
                            }
                            _ => f32::from(event.position.y),
                        };
                        let active_order = {
                            let Some(session) = this.drag.as_mut() else {
                                return;
                            };
                            match session.update(axis) {
                                Ok(update) if update.active => Some(update.preview_order.to_vec()),
                                _ => None,
                            }
                        };
                        if let Some(order) = active_order {
                            this.drag_order = Some(order);
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                        let Some(session) = this.drag.take() else {
                            return;
                        };
                        this.drag_order = None;
                        match session.finish() {
                            rmac_dock::drag::DropOutcome::Click { entry } => {
                                if let rmac_dock::presentation::EntryId::Application(app_id) = entry
                                {
                                    this.activate_entry(&app_id, event.modifiers.platform, cx);
                                }
                            }
                            rmac_dock::drag::DropOutcome::Reorder(intent) => {
                                let model = this.model_snapshot(cx);
                                if let Some(revalidated) =
                                    model.as_ref().and_then(|model| intent.revalidate(model))
                                {
                                    let command = revalidated.command().clone();
                                    this.dispatch_action(
                                        rmac_dock::menu::Action::Context(
                                            rmac_dock::ContextAction::UpdatePins(command),
                                        ),
                                        cx,
                                    );
                                }
                            }
                            _ => cx.notify(),
                        }
                    }),
                );
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
                        .hover(|style| style.opacity(0.88))
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
                        let actionable = matches!(
                            activation,
                            rmac_dock::Activation::Launch { .. }
                                | rmac_dock::Activation::FocusWindow(_)
                        );
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
                            .opacity(if available { 1.0 } else { 0.58 });
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
                                let drag_entry =
                                    rmac_dock::presentation::EntryId::Application(app_id.clone());
                                visual
                                    .cursor_pointer()
                                    .hover(|style| style.opacity(0.88))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &gpui::MouseDownEvent, _, cx| {
                                                cx.stop_propagation();
                                                if index >= pinned_count {
                                                    return;
                                                }
                                                let axis = match this.placement {
                                                    rmac_shell_settings::DockPlacement::Bottom => {
                                                        f32::from(event.position.x)
                                                    }
                                                    _ => f32::from(event.position.y),
                                                };
                                                let Some(description) =
                                                    this.surface_description.clone()
                                                else {
                                                    return;
                                                };
                                                let Ok(plan) =
                                                    this.content.prepare_layout(&description)
                                                else {
                                                    return;
                                                };
                                                let Some(model) = this.model_snapshot(cx) else {
                                                    return;
                                                };
                                                if let Ok(session) =
                                                    rmac_dock::drag::DragSession::begin(
                                                        &model,
                                                        &plan,
                                                        &drag_entry,
                                                        axis,
                                                    )
                                                {
                                                    this.drag = Some(session);
                                                    this.drag_order = None;
                                                    cx.notify();
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_move(cx.listener(
                                        move |this, event: &gpui::MouseMoveEvent, _, cx| {
                                            if event.pressed_button != Some(MouseButton::Left) {
                                                return;
                                            }
                                            let axis = match this.placement {
                                                rmac_shell_settings::DockPlacement::Bottom => {
                                                    f32::from(event.position.x)
                                                }
                                                _ => f32::from(event.position.y),
                                            };
                                            let active_order = {
                                                let Some(session) = this.drag.as_mut() else {
                                                    return;
                                                };
                                                match session.update(axis) {
                                                    Ok(update) if update.active => {
                                                        Some(update.preview_order.to_vec())
                                                    }
                                                    _ => None,
                                                }
                                            };
                                            if let Some(order) = active_order {
                                                this.drag_order = Some(order);
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &gpui::MouseUpEvent, _, cx| {
                                                let platform = event.modifiers.platform;
                                                let Some(session) = this.drag.take() else {
                                                    this.activate_entry(
                                                        &activate_app_id,
                                                        platform,
                                                        cx,
                                                    );
                                                    return;
                                                };
                                                this.drag_order = None;
                                                match session.finish() {
                                                    rmac_dock::drag::DropOutcome::Click { .. } => {
                                                        this.activate_entry(
                                                            &activate_app_id,
                                                            platform,
                                                            cx,
                                                        );
                                                    }
                                                    rmac_dock::drag::DropOutcome::Reorder(
                                                        intent,
                                                    ) => {
                                                        let model = this.model_snapshot(cx);
                                                        if let Some(revalidated) = model
                                                            .as_ref()
                                                            .and_then(|model| {
                                                                intent.revalidate(model)
                                                            })
                                                        {
                                                            let command =
                                                                revalidated.command().clone();
                                                            this.dispatch_action(
                                                                rmac_dock::menu::Action::Context(
                                                                    rmac_dock::ContextAction::UpdatePins(command),
                                                                ),
                                                                cx,
                                                            );
                                                        }
                                                    }
                                                    _ => cx.notify(),
                                                }
                                            },
                                        ),
                                    )
                            });
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
                        let squircle = visual_size * ICON_SQUIRCLE;
                        if let Some(path) = icon_path {
                            visual = visual.child(
                                img(path)
                                    .w(px(visual_size * ICON_ART_SCALE))
                                    .h(px(visual_size * ICON_ART_SCALE))
                                    .rounded(px(tokens::dock_tile_radius(visual_size))),
                            );
                        } else {
                            // Without artwork, a lettered squircle the size
                            // of a real icon's visible shape.
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
                        if menu_open {
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
                                });
                                this.input_region = None;
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
                        if entry.urgent {
                            item = item.child(
                                div()
                                    .absolute()
                                    .top(px(-3.0))
                                    .right(px(-3.0))
                                    .w(px(10.0))
                                    .h(px(10.0))
                                    .rounded_full()
                                    .bg(rgba(tokens::system_red())),
                            );
                        }
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
                            .hover(|style| style.opacity(0.88))
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
                                        .and_then(|mut menu| {
                                            // Permanent deletion remains behind a dedicated
                                            // confirmation sheet. The first Dock-menu slice
                                            // exposes the safe Open Trash command only.
                                            menu.empty_trash = None;
                                            rmac_dock::menu::Session::special(&menu).ok()
                                        })
                                };
                                this.context_menu = session.map(|session| DockMenu {
                                    anchor: trash_center,
                                    session,
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
            .children(render_context_menu(
                self.context_menu.as_ref(),
                self.placement,
                menu_geometry,
                self.display_id,
                cx,
            ))
        }
    }

    /// Whether any row carries a check mark; macOS then gives every row a
    /// leading check column.
    fn dock_menu_has_checks(menu: &DockMenu) -> bool {
        menu.session.rows().iter().any(|row| row.checked)
    }

    /// The Dock menu is exactly as wide as its widest row plus padding; unlike
    /// menu bar menus it has no minimum (the Trash menu is 92 wide on macOS).
    fn dock_menu_width(menu: &DockMenu, window: &Window) -> f32 {
        let text = menu
            .session
            .rows()
            .iter()
            .map(|row| rmac_shell_ui::text_width(window, &row.label, FontWeight::NORMAL))
            .fold(0.0, f32::max);
        let leading = if dock_menu_has_checks(menu) {
            MENU_CHECK_INSET + MENU_CHECK_COLUMN
        } else {
            MENU_ROW_INSET
        };
        (text + leading + MENU_ROW_INSET + 2.0 * (MENU_PADDING + MENU_BORDER)).ceil()
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

    fn render_context_menu(
        menu: Option<&DockMenu>,
        placement: rmac_shell_settings::DockPlacement,
        geometry: Option<(f32, f32, f32)>,
        display_id: u64,
        cx: &Context<Dock>,
    ) -> Option<gpui::AnyElement> {
        let menu = menu?;
        let (start, tip, width) = geometry?;
        let selected = menu.session.selected().cloned();
        let rows = menu.session.rows().to_vec();
        let has_checks = dock_menu_has_checks(menu);
        // macOS Dock menus have no title row; the app name stays the
        // accessible title.
        let mut panel = div()
            .id(format!("dock-menu-{display_id}"))
            .role(Role::Menu)
            .aria_label(menu.session.accessible_title().to_owned())
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
            .occlude();
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
        for (index, row) in rows.iter().enumerate() {
            if index > 0 && rows[index - 1].section != row.section {
                // An 11-tall separator whose line is inset 16 from the edge.
                panel = panel.child(
                    div()
                        .h(px(MENU_BORDER))
                        .mx(px(MENU_ROW_INSET))
                        .my(px((MENU_SEPARATOR - MENU_BORDER) / 2.0))
                        .bg(rgba(tokens::separator())),
                );
            }
            let row_id = row.id.clone();
            let primary = row.primary.clone();
            let secondary = row.secondary.clone();
            let enabled = row.enabled && primary.is_some();
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
                .when(selected.as_ref() == Some(&row.id), |style| {
                    style.bg(rgba(tokens::accent()))
                })
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
            element = element.child(row.label.clone());
            if enabled {
                let primary = primary.expect("enabled Dock menu row has an action");
                let click_row_id = row.id.clone();
                element = element
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(tokens::accent())))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered
                            && this
                                .context_menu
                                .as_mut()
                                .is_some_and(|menu| menu.session.select(&row_id))
                        {
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        let action = if matches!(click_row_id, rmac_dock::menu::RowId::Quit)
                            && event.modifiers().alt
                        {
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
                        if authorized {
                            this.dispatch_action(action, cx);
                        } else {
                            eprintln!("the selected Dock command is no longer current");
                        }
                        cx.notify();
                    }));
            }
            panel = panel.child(element);
        }
        Some(panel.into_any_element())
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
                first_party_icon_path(app_id)
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
            _ => return None,
        };
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../packaging/rmac-apps/icons")
                .join(file),
        )
    }

    fn trash_icon_path(full: bool) -> Option<PathBuf> {
        let file = if full {
            "trash-full.svg"
        } else {
            "trash-empty.svg"
        };
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
                let backdrop = open_dock_backdrop(display.clone(), &surface, cx);
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

    struct DockBackdrop;

    impl Render for DockBackdrop {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
            |_, cx| cx.new(|_| DockBackdrop),
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
