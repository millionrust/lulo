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
        div, img, layer_shell::*, point, prelude::*, px, rgba, AnyWindowHandle, App, Bounds,
        Context, DisplayId, Entity, FontWeight, MouseButton, PlatformDisplay, QuitMode, Role, Size,
        Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_gpui_upstream_lab::shell_visuals as visuals;

    const EXCLUSIVE_ZONE: f32 = 88.0;
    const ICON_SIZE: f32 = 56.0;
    const ICON_GAP: f32 = 8.0;
    const SHELF_PADDING: f32 = 8.0;
    const SEPARATOR_WIDTH: f32 = 1.0;
    const TOOLTIP_WIDTH: f32 = 240.0;
    const TOOLTIP_BOTTOM: f32 = 92.0;
    const MENU_WIDTH: f32 = 248.0;
    const MENU_ROW_HEIGHT: f32 = 28.0;
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
                                .map(|output| rmac_gpui_upstream_lab::stable_output_uuid(output))
                                .collect();
                            if was_ready
                                && rmac_gpui_upstream_lab::output_reappeared(
                                    &this.outputs,
                                    &outputs,
                                    &mut this.removed_outputs,
                                )
                            {
                                std::process::exit(
                                    rmac_gpui_upstream_lab::WAYLAND_OUTPUT_RESTART_EXIT_CODE,
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
            let fullscreen = rmac_gpui_upstream_lab::top_bar_output_policies(&snapshot.compositor);
            snapshot.surface_plan.as_ref().ok().map(|surfaces| {
                surfaces
                    .iter()
                    .map(|surface| {
                        let output = rmac_gpui_upstream_lab::stable_output_uuid(&surface.output);
                        DockSurface::from_description(
                            surface,
                            fullscreen.get(&output).copied().unwrap_or(false),
                        )
                    })
                    .collect()
            })
        }
    }

    struct DockMenu {
        anchor: f32,
        session: rmac_dock::menu::Session,
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
        fullscreen: bool,
        overview_visible: bool,
        visibility_policy: Option<(bool, bool)>,
        hide_generation: u64,
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
                let _ = self.status.update(cx, |status, cx| {
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
                let _ = status.update(cx, |status, cx| {
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
            let (dock_settings, model, entries) = {
                let status = self.status.read(cx);
                let snapshot = status
                    .snapshot()
                    .expect("a Dock surface is opened only after runtime readiness");
                (
                    snapshot.settings.clone(),
                    snapshot.model.clone(),
                    snapshot.content.applications.clone(),
                )
            };
            if self.render_count == 1 {
                for item in &model.items {
                    eprintln!(
                        "Dock item {} launchable={} running={} windows={}",
                        item.id,
                        item.launchable,
                        item.running,
                        item.windows.len()
                    );
                }
            }
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
            let separates_running = pinned_count > 0 && pinned_count < entries.len();
            let separator_count = usize::from(separates_running) + usize::from(!entries.is_empty());
            let item_count = entries.len() + 1;
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
                + SEPARATOR_WIDTH * separator_count as f32
                + ICON_GAP * child_count.saturating_sub(1) as f32
                + 2.0 * SHELF_PADDING;
            let shelf_start = (axis - shelf_extent) / 2.0;
            let trash_center = shelf_extent - SHELF_PADDING - ICON_SIZE / 2.0;
            let menu_geometry = self.context_menu.as_ref().map(|menu| {
                let row_count = menu.session.rows().len() as f32;
                let section_breaks = menu
                    .session
                    .rows()
                    .windows(2)
                    .filter(|rows| rows[0].section != rows[1].section)
                    .count() as f32;
                let height = 38.0 + row_count * MENU_ROW_HEIGHT + section_breaks * 9.0;
                let start = (shelf_start + menu.anchor - MENU_WIDTH / 2.0)
                    .clamp(8.0, (axis - MENU_WIDTH - 8.0).max(8.0));
                (start, height)
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
                                .rounded(px(visuals::TOOLTIP_RADIUS))
                                .bg(rgba(0x18263aee))
                                .border_1()
                                .border_color(rgba(visuals::LIGHT_BORDER))
                                .shadow_lg()
                                .text_sm()
                                .text_color(rgba(0xffffffff))
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
                rmac_shell_settings::DockPlacement::Bottom => {
                    root.items_end().justify_center().pb_2()
                }
                rmac_shell_settings::DockPlacement::Left => {
                    root.items_start().justify_center().pl_2()
                }
                rmac_shell_settings::DockPlacement::Right => {
                    root.items_end().justify_center().pr_2()
                }
            };
            let shelf = div()
                .flex()
                .gap_2()
                .p_2()
                .rounded(px(visuals::DOCK_RADIUS))
                .bg(rgba(visuals::DOCK_TINT))
                .border_1()
                .border_color(rgba(visuals::DOCK_BORDER))
                .shadow_lg()
                .opacity(if self.hidden { 0.0 } else { 1.0 });
            let shelf = if horizontal {
                shelf.items_end()
            } else {
                shelf.flex_col().items_center()
            };
            root.child(
                shelf
                    .children(entries.into_iter().enumerate().flat_map(|(index, entry)| {
                        let relative_center = SHELF_PADDING
                            + ICON_SIZE / 2.0
                            + index as f32 * (ICON_SIZE + ICON_GAP)
                            + if separates_running && index >= pinned_count {
                                SEPARATOR_WIDTH + ICON_GAP
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
                        let active =
                            entry.activity == rmac_dock::presentation::ActivityIndicator::Active;
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
                        let reveal_app_id = app_id.clone();
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
                            .text_color(rgba(0xffffffff))
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
                            .rounded(px(13.0 * visual_size / ICON_SIZE))
                            .bg(rgba(if icon_path.is_some() {
                                0x00000000
                            } else {
                                item_color(&app_id, available)
                            }))
                            .when(actionable, |visual| {
                                visual
                                    .cursor_pointer()
                                    .hover(|style| style.opacity(0.88))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this,
                                                  event: &gpui::MouseDownEvent,
                                                  _,
                                                  cx| {
                                                cx.stop_propagation();
                                                eprintln!(
                                                    "Dock activation requested for {activate_app_id}"
                                                );
                                                if event.modifiers.platform {
                                                    let reveal = {
                                                        let status = this.status.read(cx);
                                                        status
                                                            .model()
                                                            .and_then(|model| {
                                                                model.context_menu(&reveal_app_id)
                                                            })
                                                            .and_then(|menu| menu.show_in_finder)
                                                            .filter(|action| {
                                                                status.model().is_some_and(|model| {
                                                                    model.authorizes_context_action(
                                                                        action,
                                                                    )
                                                                })
                                                            })
                                                    };
                                                    if let Some(action) = reveal {
                                                        this.dispatch_action(
                                                            rmac_dock::menu::Action::Context(action),
                                                            cx,
                                                        );
                                                    }
                                                } else {
                                                    this.dispatch_action(
                                                        rmac_dock::menu::Action::ActivateEntry(
                                                            rmac_dock::presentation::EntryId::Application(
                                                                activate_app_id.clone(),
                                                            ),
                                                        ),
                                                        cx,
                                                    );
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
                        if let Some(path) = icon_path {
                            visual = visual.child(
                                img(path)
                                    .w(px(visual_size - 2.0))
                                    .h(px(visual_size - 2.0))
                                    .rounded(px(14.0 * visual_size / ICON_SIZE)),
                            );
                        } else {
                            visual = visual.child(item_mark(&entry.label));
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
                            let indicator = div()
                                .absolute()
                                .w(px(if active { 7.0 } else { 5.0 }))
                                .h(px(if active { 7.0 } else { 5.0 }))
                                .rounded_full()
                                .bg(rgba(if active { 0x2563ebff } else { 0x60656dff }));
                            let indicator = match self.placement {
                                rmac_shell_settings::DockPlacement::Bottom => {
                                    indicator.bottom(px(-7.0))
                                }
                                rmac_shell_settings::DockPlacement::Left => {
                                    indicator.right(px(-7.0))
                                }
                                rmac_shell_settings::DockPlacement::Right => {
                                    indicator.left(px(-7.0))
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
                                    .bg(rgba(0xff3b30ff)),
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
                            .rounded(px(13.0))
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
                            let image = img(path)
                                .absolute()
                                .w(px(visual_size - 2.0))
                                .h(px(visual_size - 2.0))
                                .rounded(px(14.0 * visual_size / ICON_SIZE));
                            let image = match self.placement {
                                rmac_shell_settings::DockPlacement::Bottom => {
                                    image.left(px(visual_offset)).bottom_0()
                                }
                                rmac_shell_settings::DockPlacement::Left => {
                                    image.left_0().top(px(visual_offset))
                                }
                                rmac_shell_settings::DockPlacement::Right => {
                                    image.right_0().top(px(visual_offset))
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

    fn render_context_menu(
        menu: Option<&DockMenu>,
        placement: rmac_shell_settings::DockPlacement,
        geometry: Option<(f32, f32)>,
        display_id: u64,
        cx: &Context<Dock>,
    ) -> Option<gpui::AnyElement> {
        let menu = menu?;
        let (start, _) = geometry?;
        let selected = menu.session.selected().cloned();
        let rows = menu.session.rows().to_vec();
        let mut panel = div()
            .id(format!("dock-menu-{display_id}"))
            .role(Role::Menu)
            .aria_label(menu.session.accessible_title().to_owned())
            .absolute()
            .w(px(MENU_WIDTH))
            .p_1()
            .rounded(px(visuals::MENU_RADIUS))
            .bg(rgba(visuals::REGULAR_DARK_TINT))
            .border_1()
            .border_color(rgba(visuals::LIGHT_BORDER))
            .shadow_lg()
            .text_size(px(13.0))
            .text_color(rgba(visuals::PRIMARY_TEXT))
            .occlude()
            .child(
                div()
                    .h(px(30.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(menu.session.title().to_owned()),
            );
        panel = match placement {
            rmac_shell_settings::DockPlacement::Bottom => {
                panel.left(px(start)).bottom(px(EXCLUSIVE_ZONE + 8.0))
            }
            rmac_shell_settings::DockPlacement::Left => {
                panel.left(px(EXCLUSIVE_ZONE + 8.0)).top(px(start))
            }
            rmac_shell_settings::DockPlacement::Right => {
                panel.right(px(EXCLUSIVE_ZONE + 8.0)).top(px(start))
            }
        };
        for (index, row) in rows.iter().enumerate() {
            if index > 0 && rows[index - 1].section != row.section {
                panel = panel.child(div().h(px(1.0)).mx_2().my_1().bg(rgba(0xffffff25)));
            }
            let row_id = row.id.clone();
            let primary = row.primary.clone();
            let secondary = row.secondary.clone();
            let enabled = row.enabled && primary.is_some();
            let mut element = div()
                .id(format!("dock-menu-{display_id}-{index}"))
                .role(Role::MenuItem)
                .aria_label(row.accessible_label.clone())
                .h(px(MENU_ROW_HEIGHT))
                .px_2()
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(visuals::MENU_ITEM_RADIUS))
                .when(selected.as_ref() == Some(&row.id), |style| {
                    style.bg(rgba(visuals::ACCENT))
                })
                .when(!enabled, |style| {
                    style.text_color(rgba(visuals::DISABLED_TEXT))
                })
                .child(row.label.clone());
            if row.checked {
                element = element.child("✓");
            }
            if enabled {
                let primary = primary.expect("enabled Dock menu row has an action");
                let click_row_id = row.id.clone();
                element = element
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(visuals::ACCENT)))
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
        let separator = div().bg(rgba(visuals::SEPARATOR));
        match placement {
            rmac_shell_settings::DockPlacement::Bottom => separator
                .w(px(SEPARATOR_WIDTH))
                .h(px(48.0))
                .mb_1()
                .into_any_element(),
            rmac_shell_settings::DockPlacement::Left
            | rmac_shell_settings::DockPlacement::Right => separator
                .w(px(48.0))
                .h(px(SEPARATOR_WIDTH))
                .mx_1()
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
            _ => return None,
        };
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../packaging/rmac-apps/icons")
                .join(file),
        )
    }

    fn trash_icon_path(full: bool) -> Option<PathBuf> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/rmac-dock/assets/icons")
            .join(if full {
                "trash-full.svg"
            } else {
                "trash-empty.svg"
            });
        path.is_file().then_some(path)
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
    }

    impl Default for DockSurface {
        fn default() -> Self {
            Self {
                output: None,
                placement: rmac_shell_settings::DockPlacement::Bottom,
                reserve_space: true,
                fullscreen: false,
                overview_visible: false,
            }
        }
    }

    impl DockSurface {
        fn from_description(surface: &rmac_dock::SurfaceDescription, fullscreen: bool) -> Self {
            Self {
                output: Some(surface.output.clone()),
                placement: surface.placement,
                reserve_space: surface.exclusive_zone > 0.0 && !fullscreen,
                fullscreen,
                overview_visible: surface.overview_visible,
            }
        }
    }

    #[derive(Default)]
    struct DockWindows {
        windows: std::collections::BTreeMap<uuid::Uuid, (DockSurface, AnyWindowHandle)>,
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
            let displays = rmac_gpui_upstream_lab::output_surfaces::newest_displays(cx);
            let desired = desired.map(|surfaces| {
                surfaces
                    .iter()
                    .filter_map(|surface| {
                        surface.output.as_ref().map(|output| {
                            (
                                rmac_gpui_upstream_lab::stable_output_uuid(output),
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
                if let Some((_, handle)) = self.windows.remove(&uuid) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
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
                    .is_some_and(|(current, _)| *current == surface)
                {
                    continue;
                }
                let handle = open_dock(display, surface.clone(), status.clone(), cx);
                if let Some((_, previous)) = self.windows.insert(uuid, (surface, handle)) {
                    let _ = previous.update(cx, |_, window, _| window.remove_window());
                }
            }
        }
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
                    // The layer is deliberately much larger than the visible
                    // shelf so Dock menus can open above it. A blurred window
                    // background would therefore blur a large band of every
                    // application behind the otherwise transparent surface.
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
