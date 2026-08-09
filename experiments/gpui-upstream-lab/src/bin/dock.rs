#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgba, AnyWindowHandle, App, Bounds,
        Context, DisplayId, Entity, FontWeight, PlatformDisplay, QuitMode, Role, Size, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;

    const SURFACE_HEIGHT: f32 = 184.0;
    const SIDE_SURFACE_WIDTH: f32 = 344.0;
    const EXCLUSIVE_ZONE: f32 = 88.0;
    const ICON_SIZE: f32 = 56.0;
    const ICON_GAP: f32 = 8.0;
    const SHELF_PADDING: f32 = 8.0;
    const SEPARATOR_WIDTH: f32 = 1.0;
    const TOOLTIP_WIDTH: f32 = 240.0;
    const TOOLTIP_BOTTOM: f32 = 92.0;
    const READY_FILE_ENV: &str = "RMAC_DOCK_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_DOCK_RENDER_COUNT_DIR";
    static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

    enum SourceEvent {
        Settings(Result<rmac_shell_settings::ShellSettings, String>),
        Catalog(Result<Vec<rmac_apps::Application>, String>),
        Places(Result<rmac_places::Snapshot, String>),
    }

    struct DockStatus {
        settings: rmac_shell_settings::ShellSettings,
        catalog: Vec<rmac_apps::Application>,
        compositor: rmac_compositor::State,
        compositor_ready: bool,
        outputs: std::collections::BTreeSet<uuid::Uuid>,
        removed_outputs: std::collections::BTreeSet<uuid::Uuid>,
        places: rmac_places::Snapshot,
    }

    impl DockStatus {
        fn new(
            compositor: async_channel::Receiver<rmac_compositor::Event>,
            sources: async_channel::Receiver<SourceEvent>,
            reconcile: async_channel::Sender<()>,
            cx: &mut Context<Self>,
        ) -> Self {
            let compositor_reconcile = reconcile.clone();
            cx.spawn(async move |this, cx| {
                while let Ok(event) = compositor.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            let was_ready = this.compositor_ready;
                            if this.compositor.apply(event).visible {
                                cx.notify();
                            }
                            let outputs = this
                                .compositor
                                .snapshot()
                                .outputs
                                .into_iter()
                                .filter(|output| output.enabled())
                                .map(|output| {
                                    rmac_gpui_upstream_lab::stable_output_uuid(&output.id)
                                })
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
                            this.compositor_ready = true;
                        })
                        .is_err()
                    {
                        break;
                    }
                    let _ = compositor_reconcile.try_send(());
                }
            })
            .detach();
            let source_reconcile = reconcile;
            cx.spawn(async move |this, cx| {
                while let Ok(event) = sources.recv().await {
                    let settings_changed = matches!(&event, SourceEvent::Settings(Ok(_)));
                    if this
                        .update(cx, |this, cx| match event {
                            SourceEvent::Settings(Ok(settings)) if this.settings != settings => {
                                this.settings = settings;
                                cx.notify();
                            }
                            SourceEvent::Catalog(Ok(catalog)) if this.catalog != catalog => {
                                this.catalog = catalog;
                                cx.notify();
                            }
                            SourceEvent::Places(Ok(places)) if this.places != places => {
                                this.places = places;
                                cx.notify();
                            }
                            SourceEvent::Settings(Err(detail)) => {
                                eprintln!("Dock settings unavailable: {detail}");
                            }
                            SourceEvent::Catalog(Err(detail)) => {
                                eprintln!("Dock catalog unavailable: {detail}");
                            }
                            SourceEvent::Places(Err(detail)) => {
                                eprintln!("Dock places unavailable: {detail}");
                            }
                            SourceEvent::Settings(Ok(_))
                            | SourceEvent::Catalog(Ok(_))
                            | SourceEvent::Places(Ok(_)) => {}
                        })
                        .is_err()
                    {
                        break;
                    }
                    if settings_changed {
                        let _ = source_reconcile.try_send(());
                    }
                }
            })
            .detach();
            Self {
                settings: rmac_shell_settings::ShellSettings::default(),
                catalog: Vec::new(),
                compositor: rmac_compositor::State::default(),
                compositor_ready: false,
                outputs: std::collections::BTreeSet::new(),
                removed_outputs: std::collections::BTreeSet::new(),
                places: rmac_places::Snapshot {
                    home: rmac_places::Place {
                        path: PathBuf::new(),
                        exists: false,
                    },
                    downloads: rmac_places::Place {
                        path: PathBuf::new(),
                        exists: false,
                    },
                    downloads_configured: false,
                    trash: rmac_places::TrashSnapshot::default(),
                },
            }
        }

        fn model(&self) -> rmac_dock::Model {
            rmac_dock::Model::build_with_places(
                &self.settings.pinned_apps,
                &self.settings.dock,
                &self.catalog,
                &self.compositor.snapshot(),
                &self.places,
            )
        }

        fn surfaces(&self) -> Option<Vec<DockSurface>> {
            if !self.compositor_ready {
                return None;
            }
            let snapshot = self.compositor.snapshot();
            let primary = snapshot
                .outputs
                .iter()
                .filter(|output| output.enabled())
                .map(|output| &output.id)
                .min();
            let fullscreen = rmac_gpui_upstream_lab::top_bar_output_policies(&snapshot);
            rmac_dock::surface_descriptions(&snapshot, &self.settings.dock, primary, false)
                .ok()
                .map(|surfaces| {
                    surfaces
                        .iter()
                        .map(|surface| {
                            let output =
                                rmac_gpui_upstream_lab::stable_output_uuid(&surface.output);
                            DockSurface::from_description(
                                surface,
                                fullscreen.get(&output).copied().unwrap_or(false),
                            )
                        })
                        .collect()
                })
        }
    }

    struct Dock {
        display_id: u64,
        placement: rmac_shell_settings::DockPlacement,
        render_count: u64,
        status: Entity<DockStatus>,
        hovered_item: Option<(f32, String)>,
        input_region: Option<(f32, f32, bool)>,
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
    }

    impl Render for Dock {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let status = self.status.read(cx);
            let dock_settings = status.settings.dock.clone();
            let model = status.model();
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
            let entries = rmac_dock::presentation::ShelfContent::project(&model).applications;
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
            let trash_activation = model.activate_special(rmac_dock::SpecialItemKind::Trash);
            let trash_available = trash_activation == rmac_dock::SpecialActivation::OpenTrash;
            let pinned_count = model.items.iter().take_while(|item| item.pinned).count();
            let separates_running = pinned_count > 0 && pinned_count < entries.len();
            let separator_count = usize::from(separates_running) + usize::from(!entries.is_empty());
            let item_count = entries.len() + 1;
            let child_count = item_count + separator_count;
            let window_size = window.bounds().size;
            let horizontal = self.placement == rmac_shell_settings::DockPlacement::Bottom;
            let axis = if horizontal {
                f32::from(window_size.width)
            } else {
                f32::from(window_size.height)
            };
            let shelf_extent = ICON_SIZE * item_count as f32
                + SEPARATOR_WIDTH * separator_count as f32
                + ICON_GAP * child_count.saturating_sub(1) as f32
                + 2.0 * SHELF_PADDING;
            let shelf_start = (axis - shelf_extent) / 2.0;
            let trash_center = shelf_extent - SHELF_PADDING - ICON_SIZE / 2.0;
            let input_region = (shelf_start, shelf_extent, self.hidden);
            if self.input_region != Some(input_region) {
                let bounds = match (self.placement, self.hidden) {
                    (rmac_shell_settings::DockPlacement::Bottom, true) => Bounds {
                        origin: point(px(shelf_start), px(SURFACE_HEIGHT - 2.0)),
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
                        origin: point(px(shelf_start), px(SURFACE_HEIGHT - EXCLUSIVE_ZONE)),
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
                window.set_input_region(Some(&[bounds]));
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
                                .rounded(px(8.0))
                                .bg(rgba(0x18263aee))
                                .border_1()
                                .border_color(rgba(0xffffff35))
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
                .rounded(px(26.0))
                .bg(rgba(0xe7ecf18c))
                .border_1()
                .border_color(rgba(0xffffffb8))
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
                            }));
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
                        if actionable {
                            item = item
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.88))
                                .on_click(move |_, _, cx| {
                                    dispatch_activation(activation.clone(), cx);
                                });
                        }
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
                            trash = trash.cursor_pointer().on_click(move |_, _, cx| {
                                dispatch_special(trash_activation.clone(), cx)
                            });
                        }
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
        }
    }

    fn dock_separator(placement: rmac_shell_settings::DockPlacement) -> gpui::AnyElement {
        let separator = div().bg(rgba(0x4a56646b));
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

    fn dispatch_activation(activation: rmac_dock::Activation, cx: &mut App) {
        match activation {
            rmac_dock::Activation::Launch { spec, .. } => {
                cx.background_executor()
                    .spawn(async move {
                        if blocking::unblock(move || rmac_apps::launch(&spec))
                            .await
                            .is_err()
                        {
                            eprintln!("could not launch the selected Dock application");
                        }
                    })
                    .detach();
            }
            rmac_dock::Activation::FocusWindow(window) => {
                let Ok(previous) =
                    NEXT_ACTIVATION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    })
                else {
                    eprintln!("could not focus Dock application: activation IDs exhausted");
                    return;
                };
                let request = rmac_compositor::ActionRequest {
                    id: rmac_compositor::ActivationId(previous + 1),
                    action: rmac_compositor::Action::FocusWindow { window },
                };
                cx.background_executor()
                    .spawn(async move {
                        if rmac_compositor_niri::execute(request).await.result.is_err() {
                            eprintln!("could not focus the selected Dock application");
                        }
                    })
                    .detach();
            }
            rmac_dock::Activation::NoAction | rmac_dock::Activation::Unavailable { .. } => {}
        }
    }

    fn dispatch_special(activation: rmac_dock::SpecialActivation, cx: &mut App) {
        if activation != rmac_dock::SpecialActivation::OpenTrash {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                let opened = blocking::unblock(|| {
                    Command::new("gio")
                        .args(["open", "trash:///"])
                        .spawn()
                        .map(|_| ())
                })
                .await;
                if opened.is_err() {
                    eprintln!("could not open Trash");
                }
            })
            .detach();
    }

    async fn watch_settings(sender: async_channel::Sender<SourceEvent>) {
        let setup = blocking::unblock(|| {
            let store = rmac_shell_settings::ShellSettingsStore::from_environment()
                .map_err(|_| "the shell settings authority could not start".to_owned())?;
            let settings = store
                .load()
                .map_err(|_| "the shell settings could not be loaded".to_owned())?
                .settings;
            let watcher = store
                .watch()
                .map_err(|_| "the shell settings watcher could not start".to_owned())?;
            Ok::<_, String>((store, watcher, settings))
        })
        .await;
        let (mut store, watcher, settings) = match setup {
            Ok(setup) => setup,
            Err(detail) => {
                let _ = sender.send(SourceEvent::Settings(Err(detail))).await;
                return;
            }
        };
        if sender
            .send(SourceEvent::Settings(Ok(settings)))
            .await
            .is_err()
        {
            return;
        }
        while watcher.recv().await.is_ok() {
            let (returned, result) = blocking::unblock(move || {
                let result = store
                    .load()
                    .map(|snapshot| snapshot.settings)
                    .map_err(|_| "the changed shell settings could not be loaded".to_owned());
                (store, result)
            })
            .await;
            store = returned;
            if sender.send(SourceEvent::Settings(result)).await.is_err() {
                return;
            }
        }
    }

    async fn watch_catalog(sender: async_channel::Sender<SourceEvent>) {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let setup = blocking::unblock(move || {
            let catalog = rmac_apps::discover()
                .map_err(|_| "the application catalog could not be loaded".to_owned())?;
            let watcher = rmac_apps::watch_catalog(move || {
                let _ = changed_tx.try_send(());
            })
            .map_err(|_| "the application catalog watcher could not start".to_owned())?;
            Ok::<_, String>((watcher, catalog))
        })
        .await;
        let (_watcher, catalog) = match setup {
            Ok(setup) => setup,
            Err(detail) => {
                let _ = sender.send(SourceEvent::Catalog(Err(detail))).await;
                return;
            }
        };
        if sender
            .send(SourceEvent::Catalog(Ok(catalog)))
            .await
            .is_err()
        {
            return;
        }
        while changed_rx.recv().await.is_ok() {
            let result = blocking::unblock(|| {
                rmac_apps::discover()
                    .map_err(|_| "the changed application catalog could not be loaded".to_owned())
            })
            .await;
            if sender.send(SourceEvent::Catalog(result)).await.is_err() {
                return;
            }
        }
    }

    async fn watch_places(sender: async_channel::Sender<SourceEvent>) {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        loop {
            let callback_tx = changed_tx.clone();
            let setup = blocking::unblock(move || {
                let report = rmac_places_system::snapshot(&rmac_places_system::SystemBackend)
                    .map_err(|error| error.to_string())?;
                let watcher = rmac_places_system::watch(&report, move |_| {
                    let _ = callback_tx.try_send(());
                })
                .map_err(|error| error.to_string())?;
                Ok::<_, String>((watcher, report.snapshot))
            })
            .await;
            let (_watcher, places) = match setup {
                Ok(setup) => setup,
                Err(detail) => {
                    let _ = sender.send(SourceEvent::Places(Err(detail))).await;
                    return;
                }
            };
            if sender.send(SourceEvent::Places(Ok(places))).await.is_err() {
                return;
            }
            if changed_rx.recv().await.is_err() {
                return;
            }
        }
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
        let (size, anchor) = match surface.placement {
            rmac_shell_settings::DockPlacement::Bottom => (
                Size::new(display_size.width, px(SURFACE_HEIGHT)),
                Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
            ),
            rmac_shell_settings::DockPlacement::Left => (
                Size::new(px(SIDE_SURFACE_WIDTH), display_size.height),
                Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT,
            ),
            rmac_shell_settings::DockPlacement::Right => (
                Size::new(px(SIDE_SURFACE_WIDTH), display_size.height),
                Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM,
            ),
        };
        let exclusive_zone = surface.reserve_space.then_some(px(EXCLUSIVE_ZONE));
        let handle = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size,
                    })),
                    display_id: Some(display_id),
                    app_id: Some("dev.rmac.Dock".to_owned()),
                    window_background: WindowBackgroundAppearance::Blurred,
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
            let (compositor_tx, compositor_rx) = async_channel::bounded(64);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                        eprintln!("Dock compositor watcher stopped: {error}");
                    }
                })
                .detach();
            let (source_tx, source_rx) = async_channel::bounded(4);
            let (reconcile_tx, reconcile_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(watch_settings(source_tx.clone()))
                .detach();
            cx.background_executor()
                .spawn(watch_catalog(source_tx.clone()))
                .detach();
            cx.background_executor()
                .spawn(watch_places(source_tx))
                .detach();
            let status =
                cx.new(|cx| DockStatus::new(compositor_rx, source_rx, reconcile_tx.clone(), cx));
            let _ = reconcile_tx.try_send(());
            cx.spawn(async move |cx| {
                let mut windows = DockWindows::default();
                while reconcile_rx.recv().await.is_ok() {
                    loop {
                        let complete = cx.update(|cx| {
                            let surfaces = status.read(cx).surfaces();
                            let expected = surfaces
                                .as_ref()
                                .map(Vec::len)
                                .unwrap_or_else(|| cx.displays().len());
                            windows.reconcile(surfaces.as_deref(), &status, cx);
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
