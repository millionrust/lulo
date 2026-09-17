#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px, rgba,
        AnyWindowHandle, App, Bounds, ClickEvent, Context, DisplayId, Entity, FocusHandle,
        FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, PlatformDisplay, Point,
        QuitMode, RenderImage, Role, SharedString, Size, Window, WindowBackgroundAppearance,
        WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_shell_ui::tokens;
    use uuid::Uuid;

    const READY_FILE_ENV: &str = "RMAC_WALLPAPER_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_WALLPAPER_RENDER_COUNT_DIR";
    static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone)]
    struct PreparedSurface {
        image: Arc<RenderImage>,
        layout: rmac_wallpaper::Layout,
    }

    enum PreparedUpdate {
        Render(std::collections::BTreeMap<Uuid, PreparedSurface>),
        Health(rmac_wallpaper_runtime::HealthSnapshot),
        Desktop {
            snapshot: Option<rmac_desktop::Snapshot>,
            error: Option<SharedString>,
            sort: rmac_desktop::SortOrder,
        },
    }

    struct WallpaperStatus {
        surfaces: std::collections::BTreeMap<Uuid, PreparedSurface>,
        health: rmac_wallpaper_runtime::HealthSnapshot,
        compositor: rmac_compositor::State,
        desktop: Option<rmac_desktop::Snapshot>,
        desktop_error: Option<SharedString>,
        desktop_sort: rmac_desktop::SortOrder,
        desktop_requests: async_channel::Sender<rmac_desktop::SortOrder>,
    }

    impl WallpaperStatus {
        fn new(
            receiver: async_channel::Receiver<PreparedUpdate>,
            desktop_requests: async_channel::Sender<rmac_desktop::SortOrder>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| {
                while let Ok(update) = receiver.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            match update {
                                PreparedUpdate::Render(surfaces) => this.surfaces = surfaces,
                                PreparedUpdate::Health(health) => this.health = health,
                                PreparedUpdate::Desktop {
                                    snapshot,
                                    error,
                                    sort,
                                } => {
                                    this.desktop = snapshot;
                                    this.desktop_error = error;
                                    this.desktop_sort = sort;
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            Self {
                surfaces: std::collections::BTreeMap::new(),
                health: rmac_wallpaper_runtime::HealthSnapshot::default(),
                compositor: rmac_compositor::State::default(),
                desktop: None,
                desktop_error: None,
                desktop_sort: rmac_desktop::SortOrder::Name,
                desktop_requests,
            }
        }

        fn set_desktop_sort(&mut self, sort: rmac_desktop::SortOrder) {
            self.desktop_sort = sort;
            let _ = self.desktop_requests.try_send(sort);
        }

        fn app_drawer_window(&self) -> Option<rmac_compositor::WindowId> {
            self.compositor
                .snapshot()
                .windows
                .into_iter()
                .find(|window| window.app_id.as_deref() == Some(rmac_apps::identity::APP_DRAWER))
                .map(|window| window.id)
        }
    }

    #[derive(Clone)]
    enum DesktopMenuTarget {
        Background,
        Item(PathBuf),
    }

    struct DesktopMenu {
        position: Point<Pixels>,
        target: DesktopMenuTarget,
        selected: usize,
        sort_submenu: bool,
    }

    #[derive(Clone, Copy)]
    enum DesktopCommand {
        NewFolder,
        Open,
        Reveal,
        MoveToTrash,
        ShowSort,
        Sort(rmac_desktop::SortOrder),
        WallpaperSettings,
        DesktopDockSettings,
    }

    #[derive(Clone)]
    struct DesktopMenuRow {
        label: SharedString,
        command: DesktopCommand,
        section: u8,
        danger: bool,
        checked: bool,
        submenu: bool,
    }

    struct Wallpaper {
        display_id: u64,
        display_uuid: Uuid,
        render_count: u64,
        status: Entity<WallpaperStatus>,
        focus: FocusHandle,
        selected_item: Option<PathBuf>,
        menu: Option<DesktopMenu>,
        action_error: Option<SharedString>,
    }

    impl Wallpaper {
        fn new(
            display_id: DisplayId,
            display_uuid: Uuid,
            status: Entity<WallpaperStatus>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            Self {
                display_id: u64::from(display_id),
                display_uuid,
                render_count: 0,
                status,
                focus: cx.focus_handle(),
                selected_item: None,
                menu: None,
                action_error: None,
            }
        }

        fn dismiss_app_drawer(&self, cx: &mut App) {
            let Some(window) = self.status.read(cx).app_drawer_window() else {
                return;
            };
            let Ok(previous) =
                NEXT_ACTIVATION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
            else {
                return;
            };
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                        id: rmac_compositor::ActivationId(previous + 1),
                        action: rmac_compositor::Action::CloseWindow { window },
                    })
                    .await;
                })
                .detach();
        }

        fn open_menu(
            &mut self,
            target: DesktopMenuTarget,
            position: Point<Pixels>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.menu = Some(DesktopMenu {
                position,
                target,
                selected: 0,
                sort_submenu: false,
            });
            window.focus(&self.focus, cx);
            cx.notify();
        }

        fn close_menu(&mut self, cx: &mut Context<Self>) {
            if self.menu.take().is_some() {
                cx.notify();
            }
        }

        fn menu_rows(&self, cx: &App) -> Vec<DesktopMenuRow> {
            let Some(menu) = &self.menu else {
                return Vec::new();
            };
            if menu.sort_submenu {
                let current = self.status.read(cx).desktop_sort;
                return [
                    ("Name", rmac_desktop::SortOrder::Name),
                    ("Kind", rmac_desktop::SortOrder::Kind),
                    ("Date Modified", rmac_desktop::SortOrder::DateModified),
                    ("Size", rmac_desktop::SortOrder::Size),
                ]
                .into_iter()
                .map(|(label, sort)| DesktopMenuRow {
                    label: label.into(),
                    command: DesktopCommand::Sort(sort),
                    section: 0,
                    danger: false,
                    checked: current == sort,
                    submenu: false,
                })
                .collect();
            }
            match &menu.target {
                DesktopMenuTarget::Background => vec![
                    menu_row("New Folder", DesktopCommand::NewFolder, 0),
                    DesktopMenuRow {
                        label: "Sort By".into(),
                        command: DesktopCommand::ShowSort,
                        section: 1,
                        danger: false,
                        checked: false,
                        submenu: true,
                    },
                    menu_row("Change Wallpaper…", DesktopCommand::WallpaperSettings, 2),
                    menu_row(
                        "Desktop & Dock Settings…",
                        DesktopCommand::DesktopDockSettings,
                        2,
                    ),
                ],
                DesktopMenuTarget::Item(_) => vec![
                    menu_row("Open", DesktopCommand::Open, 0),
                    menu_row("Show in Finder", DesktopCommand::Reveal, 0),
                    DesktopMenuRow {
                        label: "Move to Trash".into(),
                        command: DesktopCommand::MoveToTrash,
                        section: 1,
                        danger: true,
                        checked: false,
                        submenu: false,
                    },
                ],
            }
        }

        fn activate_menu_command(&mut self, command: DesktopCommand, cx: &mut Context<Self>) {
            if matches!(command, DesktopCommand::ShowSort) {
                if let Some(menu) = &mut self.menu {
                    menu.sort_submenu = true;
                    menu.selected = 0;
                    cx.notify();
                }
                return;
            }
            let target = self.menu.as_ref().and_then(|menu| match &menu.target {
                DesktopMenuTarget::Item(path) => Some(path.clone()),
                DesktopMenuTarget::Background => None,
            });
            self.menu = None;
            match command {
                DesktopCommand::NewFolder => {
                    let directory = self
                        .status
                        .read(cx)
                        .desktop
                        .as_ref()
                        .map(|snapshot| snapshot.directory.clone());
                    let Some(directory) = directory else {
                        self.action_error = Some("The Desktop directory is unavailable".into());
                        cx.notify();
                        return;
                    };
                    cx.spawn(async move |this, cx| {
                        let result =
                            blocking::unblock(move || rmac_desktop::create_folder(&directory))
                                .await;
                        let _ = this.update(cx, |this, cx| {
                            match result {
                                Ok(path) => this.selected_item = Some(path),
                                Err(_) => {
                                    this.action_error =
                                        Some("The new folder could not be created".into())
                                }
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                DesktopCommand::Open => {
                    if let Some(path) = target {
                        spawn_item_action(path, ItemAction::Open, cx);
                    }
                }
                DesktopCommand::Reveal => {
                    if let Some(path) = target {
                        spawn_item_action(path, ItemAction::Reveal, cx);
                    }
                }
                DesktopCommand::MoveToTrash => {
                    if let Some(path) = target {
                        cx.spawn(async move |this, cx| {
                            let result = blocking::unblock(move || trash::delete(&path)).await;
                            let _ = this.update(cx, |this, cx| {
                                if result.is_err() {
                                    this.action_error =
                                        Some("The item could not be moved to Trash".into());
                                } else {
                                    this.selected_item = None;
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                }
                DesktopCommand::Sort(sort) => {
                    self.status
                        .update(cx, |status, _| status.set_desktop_sort(sort));
                }
                DesktopCommand::WallpaperSettings => {
                    spawn_settings("wallpaper", cx);
                }
                DesktopCommand::DesktopDockSettings => {
                    spawn_settings("desktop-dock", cx);
                }
                DesktopCommand::ShowSort => unreachable!(),
            }
            cx.notify();
        }

        fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
            let rows = self.menu_rows(cx);
            let Some(menu) = &mut self.menu else {
                return;
            };
            match event.keystroke.key.as_str() {
                "escape" if menu.sort_submenu => {
                    menu.sort_submenu = false;
                    menu.selected = 1;
                    cx.notify();
                }
                "escape" => self.close_menu(cx),
                "down" if !rows.is_empty() => {
                    menu.selected = (menu.selected + 1) % rows.len();
                    cx.notify();
                }
                "up" if !rows.is_empty() => {
                    menu.selected = menu.selected.checked_sub(1).unwrap_or(rows.len() - 1);
                    cx.notify();
                }
                "right" if rows.get(menu.selected).is_some_and(|row| row.submenu) => {
                    menu.sort_submenu = true;
                    menu.selected = 0;
                    cx.notify();
                }
                "left" if menu.sort_submenu => {
                    menu.sort_submenu = false;
                    menu.selected = 1;
                    cx.notify();
                }
                "enter" | "space" => {
                    if let Some(row) = rows.get(menu.selected) {
                        self.activate_menu_command(row.command, cx);
                    }
                }
                _ => {}
            }
        }
    }

    #[derive(Clone, Copy)]
    enum ItemAction {
        Open,
        Reveal,
    }

    fn menu_row(label: &'static str, command: DesktopCommand, section: u8) -> DesktopMenuRow {
        DesktopMenuRow {
            label: label.into(),
            command,
            section,
            danger: false,
            checked: false,
            submenu: false,
        }
    }

    fn spawn_item_action(path: PathBuf, action: ItemAction, cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let result = match action {
                    ItemAction::Open => rmac_app_launch::open_item(path).await,
                    ItemAction::Reveal => rmac_app_launch::reveal_item(path).await,
                };
                if result.is_err() {
                    eprintln!("a Desktop item action could not be completed");
                }
            })
            .detach();
    }

    fn spawn_settings(pane: &'static str, cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let result = blocking::unblock(move || {
                    std::process::Command::new("/usr/bin/rmac-system-settings")
                        .args(["--pane", pane])
                        .spawn()
                        .map(|_| ())
                })
                .await;
                if result.is_err() {
                    eprintln!("System Settings could not be opened");
                }
            })
            .detach();
    }

    impl Render for Wallpaper {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let palette = rmac_wallpaper::DEFAULT_BUILT_IN.metadata().palette;
            let surface = self
                .status
                .read(cx)
                .surfaces
                .get(&self.display_uuid)
                .cloned();
            let desktop_items = self
                .status
                .read(cx)
                .desktop
                .as_ref()
                .map(|snapshot| snapshot.items.clone())
                .unwrap_or_default();
            let desktop_error = self.status.read(cx).desktop_error.clone();
            let mut root = div()
                .id(format!("wallpaper-{}", self.display_id))
                .role(Role::Image)
                .aria_label("Desktop wallpaper")
                .relative()
                .size_full()
                .track_focus(&self.focus)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if this.menu.is_some() {
                        cx.stop_propagation();
                        this.handle_key(event, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.dismiss_app_drawer(cx);
                        this.menu = None;
                        this.selected_item = None;
                        this.action_error = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.dismiss_app_drawer(cx);
                        this.selected_item = None;
                        this.open_menu(DesktopMenuTarget::Background, event.position, window, cx);
                    }),
                )
                .overflow_hidden()
                .bg(linear_gradient(
                    145.0,
                    linear_color_stop(rgba((palette[0] << 8) | 0xff), 0.0),
                    linear_color_stop(rgba((palette[1] << 8) | 0xff), 1.0),
                )
                .color_space(gpui::ColorSpace::Oklab))
                .child(
                    div().absolute().inset_0().bg(linear_gradient(
                        35.0,
                        linear_color_stop(rgba((palette[2] << 8) | 0xc8), 0.0),
                        linear_color_stop(rgba(palette[2] << 8), 0.72),
                    )
                    .color_space(gpui::ColorSpace::Oklab)),
                )
                .child(
                    div().absolute().inset_0().bg(linear_gradient(
                        315.0,
                        linear_color_stop(rgba(palette[3] << 8), 0.28),
                        linear_color_stop(rgba((palette[3] << 8) | 0xb8), 1.0),
                    )
                    .color_space(gpui::ColorSpace::Oklab)),
                );
            if let Some(surface) = surface {
                root = root
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(rgba(tokens::surface_window())),
                    )
                    .children(render_surface(surface));
            }
            let viewport_height = f32::from(window.bounds().size.height);
            let row_count = (((viewport_height - 132.0) / 94.0).floor() as usize).max(1);
            root = root.children(desktop_items.into_iter().enumerate().map(|(index, item)| {
                let column = index / row_count;
                let row = index % row_count;
                let selected = self.selected_item.as_ref() == Some(&item.path);
                let open_path = item.path.clone();
                let menu_path = item.path.clone();
                let extension = item
                    .path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .chars()
                    .take(4)
                    .collect::<String>()
                    .to_uppercase();
                let icon = desktop_item_icon(item.kind, extension);
                div()
                    .id(format!("desktop-item-{}-{index}", self.display_id))
                    .role(Role::Button)
                    .aria_label(format!("{}, Desktop item", item.name))
                    .absolute()
                    .right(px(22.0 + column as f32 * 96.0))
                    .top(px(42.0 + row as f32 * 94.0))
                    .w(px(82.0))
                    .h(px(88.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .rounded(px(tokens::control_radius()))
                    .when(selected, |element| {
                        element.bg(rgba(tokens::selection_text()))
                    })
                    .hover(|element| element.bg(rgba(tokens::light_hover())))
                    .child(icon)
                    .child(
                        div()
                            .max_w(px(80.0))
                            .px_1()
                            .rounded(px(tokens::menu_item_radius()))
                            .bg(rgba(tokens::overlay_chip()))
                            .text_xs()
                            .text_center()
                            .text_color(rgba(tokens::primary_text()))
                            .line_clamp(2)
                            .child(item.name),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.menu = None;
                        this.selected_item = Some(open_path.clone());
                        if event.click_count() >= 2 {
                            spawn_item_action(open_path.clone(), ItemAction::Open, cx);
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.selected_item = Some(menu_path.clone());
                            this.open_menu(
                                DesktopMenuTarget::Item(menu_path.clone()),
                                event.position,
                                window,
                                cx,
                            );
                        }),
                    )
            }));
            if let Some(menu) = render_desktop_menu(self, window, cx) {
                root = root.child(menu);
            }
            if let Some(message) = self.action_error.clone().or(desktop_error) {
                root = root.child(
                    div()
                        .absolute()
                        .left(px(24.0))
                        .bottom(px(84.0))
                        .max_w(px(360.0))
                        .px_3()
                        .py_2()
                        .rounded(px(tokens::menu_radius()))
                        .bg(rgba(tokens::regular_dark_tint()))
                        .border_1()
                        .border_color(rgba(tokens::separator()))
                        .shadow_lg()
                        .text_sm()
                        .text_color(rgba(tokens::primary_text()))
                        .child(message),
                );
            }
            root
        }
    }

    fn desktop_item_icon(kind: rmac_desktop::ItemKind, extension: String) -> gpui::AnyElement {
        match kind {
            rmac_desktop::ItemKind::Directory => div()
                .relative()
                .mt_1()
                .w(px(52.0))
                .h(px(43.0))
                .child(
                    div()
                        .absolute()
                        .left(px(3.0))
                        .top_0()
                        .w(px(24.0))
                        .h(px(10.0))
                        .rounded_t(px(tokens::menu_item_radius()))
                        .bg(rgba(tokens::system_blue())),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(37.0))
                        .rounded(px(tokens::control_radius()))
                        .bg(linear_gradient(
                            180.0,
                            linear_color_stop(rgba(tokens::accent()), 0.0),
                            linear_color_stop(rgba(tokens::accent()), 1.0),
                        )
                        .color_space(gpui::ColorSpace::Oklab))
                        .border_1()
                        .border_color(rgba(tokens::separator())),
                )
                .into_any_element(),
            _ => div()
                .mt_1()
                .w(px(43.0))
                .h(px(48.0))
                .flex()
                .items_end()
                .justify_center()
                .pb_1()
                .rounded(px(tokens::menu_item_radius()))
                .bg(rgba(tokens::surface_raised()))
                .border_1()
                .border_color(rgba(tokens::separator()))
                .shadow_md()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(tokens::primary_text()))
                .child(if extension.is_empty() {
                    "FILE".to_owned()
                } else {
                    extension
                })
                .into_any_element(),
        }
    }

    fn render_desktop_menu(
        wallpaper: &Wallpaper,
        window: &Window,
        cx: &Context<Wallpaper>,
    ) -> Option<gpui::AnyElement> {
        let menu = wallpaper.menu.as_ref()?;
        let rows = wallpaper.menu_rows(cx);
        let width = 236.0;
        let section_breaks = rows
            .windows(2)
            .filter(|rows| rows[0].section != rows[1].section)
            .count() as f32;
        let height = rows.len() as f32 * 31.0 + section_breaks * 9.0 + 10.0;
        let bounds = window.bounds().size;
        let mut left = f32::from(menu.position.x);
        let mut top = f32::from(menu.position.y);
        if menu.sort_submenu {
            left += width - 8.0;
        }
        left = left.clamp(8.0, (f32::from(bounds.width) - width - 8.0).max(8.0));
        top = top.clamp(8.0, (f32::from(bounds.height) - height - 8.0).max(8.0));
        let mut children = Vec::new();
        let mut previous_section = None;
        for (index, row) in rows.into_iter().enumerate() {
            if previous_section.is_some_and(|section| section != row.section) {
                children.push(
                    div()
                        .mx_2()
                        .my_1()
                        .h(px(1.0))
                        .bg(rgba(tokens::separator()))
                        .into_any_element(),
                );
            }
            previous_section = Some(row.section);
            let command = row.command;
            let foreground = if row.danger {
                rgba(tokens::danger())
            } else {
                rgba(tokens::primary_text())
            };
            children.push(
                div()
                    .id(format!("desktop-menu-row-{index}"))
                    .role(Role::MenuItem)
                    .aria_label(row.label.clone())
                    .h(px(31.0))
                    .mx_1()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(tokens::menu_item_radius()))
                    .text_sm()
                    .text_color(foreground)
                    .when(index == menu.selected, |element| {
                        element.bg(rgba(tokens::accent()))
                    })
                    .hover(|element| element.bg(rgba(tokens::accent())))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(if row.checked { "✓" } else { " " })
                            .child(row.label),
                    )
                    .when(row.submenu, |element| element.child("›"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.activate_menu_command(command, cx);
                    }))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .id("desktop-menu-scrim")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_menu(cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        this.close_menu(cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(left))
                        .top(px(top))
                        .w(px(width))
                        .py_1()
                        .rounded(px(tokens::card_radius()))
                        .bg(rgba(tokens::regular_dark_tint()))
                        .border_1()
                        .border_color(rgba(tokens::separator()))
                        .shadow_lg()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .children(children),
                )
                .into_any_element(),
        )
    }

    fn render_surface(surface: PreparedSurface) -> Vec<gpui::AnyElement> {
        let destination = surface.layout.destination;
        if !surface.layout.tiled {
            return vec![img(surface.image)
                .absolute()
                .left(px(destination.x as f32))
                .top(px(destination.y as f32))
                .w(px(destination.width as f32))
                .h(px(destination.height as f32))
                .object_fit(gpui::ObjectFit::Fill)
                .into_any_element()];
        }

        let width = destination.width as f32;
        let height = destination.height as f32;
        if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
            return Vec::new();
        }
        let mut x = destination.x as f32;
        let mut y = destination.y as f32;
        while x > 0.0 {
            x -= width;
        }
        while y > 0.0 {
            y -= height;
        }
        let viewport_width = (destination.width + destination.x * 2.0).max(1.0) as f32;
        let viewport_height = (destination.height + destination.y * 2.0).max(1.0) as f32;
        let columns = ((viewport_width - x) / width).ceil().max(1.0) as usize;
        let rows = ((viewport_height - y) / height).ceil().max(1.0) as usize;
        if columns.saturating_mul(rows) > 4_096 {
            return vec![img(surface.image)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(gpui::ObjectFit::Fill)
                .into_any_element()];
        }
        let mut tiles = Vec::with_capacity(columns.saturating_mul(rows));
        for row in 0..rows {
            for column in 0..columns {
                tiles.push(
                    img(surface.image.clone())
                        .absolute()
                        .left(px(x + column as f32 * width))
                        .top(px(y + row as f32 * height))
                        .w(px(width))
                        .h(px(height))
                        .object_fit(gpui::ObjectFit::Fill)
                        .into_any_element(),
                );
            }
        }
        tiles
    }

    fn prepare_surface(
        surface: rmac_wallpaper_image::RasterSurface,
    ) -> Option<(Uuid, PreparedSurface)> {
        let expected = u64::from(surface.image.width)
            .checked_mul(u64::from(surface.image.height))?
            .checked_mul(4)?;
        if expected != surface.image.rgba.len() as u64 {
            return None;
        }
        let mut bgra = Vec::with_capacity(surface.image.rgba.len());
        let mut pixels = surface.image.rgba.chunks_exact(4);
        for pixel in &mut pixels {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        if !pixels.remainder().is_empty() {
            return None;
        }
        let buffer = image::RgbaImage::from_raw(surface.image.width, surface.image.height, bgra)?;
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        let uuid = Uuid::new_v5(&Uuid::NAMESPACE_DNS, surface.output.0.as_bytes());
        Some((
            uuid,
            PreparedSurface {
                image,
                layout: surface.layout,
            },
        ))
    }

    fn start_status(cx: &mut App) -> Entity<WallpaperStatus> {
        let (runtime_tx, runtime_rx) = async_channel::bounded(2);
        let (prepared_tx, prepared_rx) = async_channel::bounded(8);
        let (desktop_requests, desktop_request_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_wallpaper_runtime::watch(runtime_tx).await {
                    eprintln!("wallpaper runtime stopped: {error}");
                }
            })
            .detach();
        let wallpaper_prepared_tx = prepared_tx.clone();
        cx.background_executor()
            .spawn(async move {
                while let Ok(update) = runtime_rx.recv().await {
                    let prepared = match update {
                        rmac_wallpaper_runtime::Update::Render {
                            rasterized, health, ..
                        } => {
                            let surfaces = blocking::unblock(move || {
                                rasterized
                                    .surfaces
                                    .into_iter()
                                    .filter_map(prepare_surface)
                                    .collect()
                            })
                            .await;
                            let _ = wallpaper_prepared_tx
                                .send(PreparedUpdate::Health(health))
                                .await;
                            PreparedUpdate::Render(surfaces)
                        }
                        rmac_wallpaper_runtime::Update::Health(health) => {
                            PreparedUpdate::Health(health)
                        }
                    };
                    if wallpaper_prepared_tx.send(prepared).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
        cx.background_executor()
            .spawn(watch_desktop(prepared_tx, desktop_request_rx))
            .detach();
        let status = cx.new(|cx| WallpaperStatus::new(prepared_rx, desktop_requests, cx));
        let (compositor_tx, compositor_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                    eprintln!("wallpaper compositor watcher stopped: {error}");
                }
            })
            .detach();
        let compositor_status = status.clone();
        cx.spawn(async move |cx| {
            while let Ok(event) = compositor_rx.recv().await {
                compositor_status.update(cx, |status, _| {
                    status.compositor.apply(event);
                });
            }
        })
        .detach();
        status
    }

    async fn watch_desktop(
        updates: async_channel::Sender<PreparedUpdate>,
        requests: async_channel::Receiver<rmac_desktop::SortOrder>,
    ) {
        let directory = match blocking::unblock(rmac_desktop::directory_from_environment).await {
            Ok(directory) => directory,
            Err(_) => {
                let _ = updates
                    .send(PreparedUpdate::Desktop {
                        snapshot: None,
                        error: Some("The Desktop directory is unavailable".into()),
                        sort: rmac_desktop::SortOrder::Name,
                    })
                    .await;
                return;
            }
        };
        let (events_tx, events_rx) = async_channel::bounded(1);
        let watched_directory = directory.clone();
        let watcher = blocking::unblock(move || {
            rmac_desktop::watch(&watched_directory, move || {
                let _ = events_tx.try_send(());
            })
        })
        .await;
        let Ok(_watcher) = watcher else {
            let _ = updates
                .send(PreparedUpdate::Desktop {
                    snapshot: None,
                    error: Some("Live Desktop updates are unavailable".into()),
                    sort: rmac_desktop::SortOrder::Name,
                })
                .await;
            return;
        };
        let mut sort = rmac_desktop::SortOrder::Name;
        publish_desktop(&updates, directory.clone(), sort).await;
        loop {
            let event = events_rx.recv().fuse();
            let request = requests.recv().fuse();
            futures_util::pin_mut!(event, request);
            futures_util::select! {
                event = event => {
                    if event.is_err() {
                        break;
                    }
                    async_io::Timer::after(Duration::from_millis(75)).await;
                    while events_rx.try_recv().is_ok() {}
                }
                request = request => match request {
                    Ok(next) => sort = next,
                    Err(_) => break,
                }
            }
            publish_desktop(&updates, directory.clone(), sort).await;
        }
    }

    async fn publish_desktop(
        updates: &async_channel::Sender<PreparedUpdate>,
        directory: PathBuf,
        sort: rmac_desktop::SortOrder,
    ) {
        let result = blocking::unblock(move || rmac_desktop::scan(&directory, sort)).await;
        let (snapshot, error) = match result {
            Ok(snapshot) => (Some(snapshot), None),
            Err(_) => (None, Some("The Desktop directory could not be read".into())),
        };
        let _ = updates
            .send(PreparedUpdate::Desktop {
                snapshot,
                error,
                sort,
            })
            .await;
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
            .unwrap_or_else(|error| panic!("open wallpaper evidence {path:?}: {error}"));
        writeln!(file, "display={display_id} scale={scale}")
            .unwrap_or_else(|error| panic!("write wallpaper evidence {path:?}: {error}"));
    }

    fn record_render_count(window: &Window, display_id: u64, render_count: u64) {
        let Some(directory) = env::var_os(RENDER_COUNT_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        window.on_next_frame(move |_, _| {
            fs::create_dir_all(&directory).unwrap_or_else(|error| {
                panic!("create wallpaper render evidence {directory:?}: {error}")
            });
            let path = directory.join(format!("{display_id}.count"));
            fs::write(&path, format!("{render_count}\n"))
                .unwrap_or_else(|error| panic!("write wallpaper render count {path:?}: {error}"));
        });
    }

    fn open_wallpaper(
        display: Rc<dyn PlatformDisplay>,
        status: Entity<WallpaperStatus>,
        cx: &mut App,
    ) -> AnyWindowHandle {
        let display_id = display.id();
        let display_uuid = display.uuid().expect("wallpaper display UUID");
        let size = display.bounds().size;
        let handle = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: Size::new(size.width, size.height),
                    })),
                    display_id: Some(display_id),
                    app_id: Some("dev.rmac.Wallpaper".to_owned()),
                    window_background: WindowBackgroundAppearance::Opaque,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: format!("rmac-wallpaper-{}", u64::from(display_id)),
                        layer: Layer::Background,
                        anchor: Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                        // The background receives keyboard focus only while a
                        // Desktop context menu is explicitly open.
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        // The protocol's -1 zone extends behind bars without
                        // changing the application work area.
                        exclusive_zone: Some(px(-1.0)),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |_, cx| cx.new(|cx| Wallpaper::new(display_id, display_uuid, status, cx)),
            )
            .expect("open wallpaper layer surface");
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
            let status = start_status(cx);
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_shell_layer::output_surfaces::watch_enabled(output_tx).await
                    {
                        eprintln!("wallpaper output watcher unavailable: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx| {
                let mut tracker = rmac_shell_layer::output_surfaces::Tracker::default();
                let mut removed_outputs = std::collections::BTreeSet::new();
                match output_rx.recv().await {
                    Ok(mut desired) => 'updates: loop {
                        let complete = cx.update(|cx| {
                            tracker.reconcile(Some(&desired), cx, |display, cx| {
                                open_wallpaper(display, status.clone(), cx)
                            });
                            tracker.len() == desired.len()
                        });
                        if complete {
                            let Ok(next) = output_rx.recv().await else {
                                break;
                            };
                            restart_for_reappeared_output(&desired, &next, &mut removed_outputs);
                            desired = next;
                            continue;
                        }
                        let update = output_rx.recv().fuse();
                        let retry = cx
                            .background_executor()
                            .timer(Duration::from_millis(50))
                            .fuse();
                        futures_util::pin_mut!(update, retry);
                        futures_util::select! {
                            next = update => match next {
                                Ok(next) => {
                                    restart_for_reappeared_output(
                                        &desired,
                                        &next,
                                        &mut removed_outputs,
                                    );
                                    desired = next;
                                },
                                Err(_) => break 'updates,
                            },
                            _ = retry => {}
                        }
                    },
                    Err(_) => loop {
                        cx.update(|cx| {
                            tracker.reconcile(None, cx, |display, cx| {
                                open_wallpaper(display, status.clone(), cx)
                            })
                        });
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                    },
                }
            })
            .detach();
        });
    }

    fn restart_for_reappeared_output(
        previous: &std::collections::BTreeSet<Uuid>,
        current: &std::collections::BTreeSet<Uuid>,
        removed: &mut std::collections::BTreeSet<Uuid>,
    ) {
        if rmac_shell_layer::output_reappeared(previous, current, removed) {
            std::process::exit(rmac_shell_layer::WAYLAND_OUTPUT_RESTART_EXIT_CODE);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("wallpaper requires Linux and: cargo run --features wayland --bin wallpaper");
    std::process::exit(2);
}
