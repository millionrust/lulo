#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::Local;
    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgba, AnyWindowHandle, App, Bounds,
        Context, DisplayId, Entity, FocusHandle, FontWeight, KeyDownEvent, PlatformDisplay,
        QuitMode, Role, Size, Subscription, Window, WindowBackgroundAppearance, WindowBounds,
        WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_gpui_upstream_lab::{
        delay_until_next_clock_tick, top_bar_active_app_name, top_bar_clock_pattern,
        top_bar_indicator_labels, top_bar_workspace_label, TopBarIndicatorKind,
    };
    use uuid::Uuid;

    const BAR_HEIGHT: f32 = 26.0;
    const MENU_SURFACE_HEIGHT: f32 = 420.0;
    const MENU_WIDTH: f32 = 248.0;
    const MENU_ROW_HEIGHT: f32 = 28.0;
    const FULLSCREEN_REVEAL_EDGE: f32 = 2.0;
    const FULLSCREEN_HIDE_DELAY: Duration = Duration::from_millis(500);
    const SYSTEM_MENU_ID: &str = "org.rmac.Desktop.SystemMenu";
    const READY_FILE_ENV: &str = "RMAC_TOP_BAR_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_TOP_BAR_RENDER_COUNT_DIR";

    struct ShellStatus {
        update: rmac_shell_runtime::Update,
        menu_app_id: Option<String>,
        menus: Vec<rmac_app_menu::Menu>,
        menu_generation: u64,
    }

    impl ShellStatus {
        fn new(
            receiver: async_channel::Receiver<rmac_shell_runtime::Update>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| loop {
                let show_seconds = this
                    .read_with(cx, |this, _| this.update.snapshot.status.clock.show_seconds)
                    .unwrap_or(false);
                let epoch_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let update = receiver.recv().fuse();
                let tick = cx
                    .background_executor()
                    .timer(delay_until_next_clock_tick(epoch_millis, show_seconds))
                    .fuse();
                futures_util::pin_mut!(update, tick);
                futures_util::select! {
                    update = update => {
                        let Ok(update) = update else { break };
                        let visible = update.visible;
                        if this.update(cx, |this, cx| {
                            let focused_app = update.snapshot.status.focused.app_id.clone();
                            if focused_app != this.menu_app_id {
                                this.menu_app_id = focused_app.clone();
                                this.menus.clear();
                                this.menu_generation = this.menu_generation.saturating_add(1);
                                if let Some(app_id) = focused_app
                                    .filter(|app_id| rmac_app_menu::bus_name(app_id).is_some())
                                {
                                    request_app_menus(app_id, this.menu_generation, cx);
                                }
                            }
                            this.update = update;
                            if visible {
                                cx.notify();
                            }
                        }).is_err() {
                            break;
                        }
                    }
                    _ = tick => {
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }
            })
            .detach();
            Self {
                update: rmac_shell_runtime::Update::default(),
                menu_app_id: None,
                menus: Vec::new(),
                menu_generation: 0,
            }
        }
    }

    fn request_app_menus(app_id: String, generation: u64, cx: &mut Context<ShellStatus>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let app_id = app_id.clone();
                    async move { rmac_app_menu::fetch(&app_id).await }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.menu_generation == generation
                    && this.menu_app_id.as_deref() == Some(app_id.as_str())
                {
                    this.menus = result.unwrap_or_default();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    struct TopBar {
        display_id: u64,
        render_count: u64,
        status: Entity<ShellStatus>,
        open_menu: Option<usize>,
        selected_item: usize,
        open_app_id: Option<String>,
        pending_system_action: Option<String>,
        fullscreen: bool,
        revealed: bool,
        pointer_inside: bool,
        hide_generation: u64,
        focus: FocusHandle,
        _blur: Subscription,
    }

    impl TopBar {
        fn new(
            display_id: DisplayId,
            status: Entity<ShellStatus>,
            fullscreen: bool,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let display_id = u64::from(display_id);
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            let focus = cx.focus_handle();
            let blur_focus = focus.clone();
            let blur = cx.on_blur(&blur_focus, window, |this, window, cx| {
                this.close_menu(window, cx);
            });
            Self {
                display_id,
                render_count: 0,
                status,
                open_menu: None,
                selected_item: 0,
                open_app_id: None,
                pending_system_action: None,
                fullscreen,
                revealed: !fullscreen,
                pointer_inside: false,
                hide_generation: 0,
                focus,
                _blur: blur,
            }
        }

        fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.open_menu.take().is_some() {
                self.open_app_id = None;
                self.pending_system_action = None;
                self.selected_item = 0;
                window.refresh();
                cx.notify();
            }
            if self.fullscreen && !self.pointer_inside {
                self.schedule_fullscreen_hide(cx);
            }
        }

        fn open_menu(
            &mut self,
            index: usize,
            app_id: String,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.open_menu = Some(index);
            self.hide_generation = self.hide_generation.saturating_add(1);
            self.revealed = true;
            self.selected_item = 0;
            self.open_app_id = Some(app_id);
            self.pending_system_action = None;
            window.focus(&self.focus, cx);
            window.refresh();
            cx.notify();
        }

        fn schedule_fullscreen_hide(&mut self, cx: &mut Context<Self>) {
            if !self.fullscreen {
                return;
            }
            self.hide_generation = self.hide_generation.saturating_add(1);
            let generation = self.hide_generation;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FULLSCREEN_HIDE_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    if this.hide_generation == generation
                        && !this.pointer_inside
                        && this.open_menu.is_none()
                    {
                        this.revealed = false;
                        cx.notify();
                    }
                });
            })
            .detach();
        }

        fn set_pointer_inside(&mut self, inside: bool, cx: &mut Context<Self>) {
            self.pointer_inside = inside;
            self.hide_generation = self.hide_generation.saturating_add(1);
            if !self.fullscreen {
                return;
            }
            if inside {
                if !self.revealed {
                    self.revealed = true;
                    cx.notify();
                }
            } else if self.open_menu.is_none() {
                self.schedule_fullscreen_hide(cx);
            }
        }

        fn handle_key(
            &mut self,
            event: &KeyDownEvent,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let (mut menus, window_id) = {
                let status = self.status.read(cx);
                (
                    status.menus.clone(),
                    status.update.snapshot.status.focused.window_id,
                )
            };
            menus.insert(0, system_menu());
            let Some(menu_index) = self.open_menu else {
                return;
            };
            if let Some(action) = self.pending_system_action.clone() {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        self.pending_system_action = None;
                        cx.notify();
                    }
                    "enter" | "space" => {
                        self.close_menu(window, cx);
                        dispatch_system_menu(action, cx);
                    }
                    _ => {}
                }
                return;
            }
            let Some(menu) = menus.get(menu_index).cloned() else {
                self.close_menu(window, cx);
                return;
            };
            match event.keystroke.key.as_str() {
                "escape" => self.close_menu(window, cx),
                "down" => {
                    self.selected_item = (self.selected_item + 1) % menu.items.len();
                    cx.notify();
                }
                "up" => {
                    self.selected_item = self
                        .selected_item
                        .checked_sub(1)
                        .unwrap_or(menu.items.len() - 1);
                    cx.notify();
                }
                "right" | "left" => {
                    let next = if event.keystroke.key == "right" {
                        (menu_index + 1) % menus.len()
                    } else {
                        menu_index.checked_sub(1).unwrap_or(menus.len() - 1)
                    };
                    self.open_menu = Some(next);
                    self.selected_item = 0;
                    cx.notify();
                }
                "enter" | "space" => {
                    if let (Some(app_id), Some(item)) = (
                        self.open_app_id.clone(),
                        menu.items.get(self.selected_item).cloned(),
                    ) {
                        if app_id == SYSTEM_MENU_ID
                            && system_action_needs_confirmation(&item.action)
                        {
                            self.pending_system_action = Some(item.action);
                            cx.notify();
                        } else {
                            self.close_menu(window, cx);
                            dispatch_menu_action(app_id, item.action, window_id, cx);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    impl Render for TopBar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let now = Local::now();
            let status = self.status.read(cx);
            let snapshot = &status.update.snapshot.status;
            let clock = now
                .format(top_bar_clock_pattern(&snapshot.clock))
                .to_string();
            let clock_label = clock.clone();
            let active_app = top_bar_active_app_name(snapshot);
            let workspace = top_bar_workspace_label(snapshot);
            let indicators = top_bar_indicator_labels(snapshot);
            let focused_app_id = snapshot.focused.app_id.clone();
            let focused_window_id = snapshot.focused.window_id;
            let mut menus = status.menus.clone();
            menus.insert(0, system_menu());

            if self.open_menu.is_some()
                && (self.open_menu >= Some(menus.len())
                    || (self.open_menu != Some(0) && self.open_app_id != focused_app_id))
            {
                self.open_menu = None;
                self.open_app_id = None;
                self.selected_item = 0;
            }
            let visible = !self.fullscreen || self.revealed || self.open_menu.is_some();
            let menu_left = self
                .open_menu
                .map(|index| menu_anchor_x(&active_app, &menus, index));
            let menu_height = if self.pending_system_action.is_some() {
                Some(150.0)
            } else {
                self.open_menu
                    .and_then(|index| menus.get(index))
                    .map(menu_panel_height)
            };
            let bar_region = Bounds {
                origin: point(px(0.0), px(0.0)),
                size: Size::new(
                    window.bounds().size.width,
                    px(if visible {
                        BAR_HEIGHT
                    } else {
                        FULLSCREEN_REVEAL_EDGE
                    }),
                ),
            };
            if visible {
                if let (Some(left), Some(height)) = (menu_left, menu_height) {
                    let popup_region = Bounds {
                        origin: point(px(left), px(BAR_HEIGHT)),
                        size: Size::new(px(MENU_WIDTH), px(height + 4.0)),
                    };
                    window.set_input_region(Some(&[bar_region, popup_region]));
                } else {
                    window.set_input_region(Some(&[bar_region]));
                }
            } else {
                window.set_input_region(Some(&[bar_region]));
            }

            let app_id_for_buttons = focused_app_id.clone();
            let menu_buttons = menus
                .iter()
                .enumerate()
                .skip(1)
                .map(|(index, menu)| {
                    let app_id = app_id_for_buttons.clone().unwrap_or_default();
                    let open = self.open_menu == Some(index);
                    div()
                        .id(format!("app-menu-{}-{index}", self.display_id))
                        .role(Role::Button)
                        .aria_label(format!("{} menu", menu.label))
                        .focusable()
                        .tab_stop(true)
                        .h(px(22.0))
                        .px_1()
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .when(open, |style| style.bg(rgba(0xffffff2d)))
                        .hover(|style| style.bg(rgba(0xffffff22)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            if this.open_menu == Some(index) {
                                this.close_menu(window, cx);
                            } else {
                                this.open_menu(index, app_id.clone(), window, cx);
                            }
                        }))
                        .child(menu.label.clone())
                })
                .collect::<Vec<_>>();

            let popup = self.open_menu.and_then(|menu_index| {
                let menu = menus.get(menu_index)?.clone();
                let app_id = if menu_index == 0 {
                    SYSTEM_MENU_ID.to_owned()
                } else {
                    focused_app_id.clone()?
                };
                let left = menu_left?;
                let selected = self.selected_item.min(menu.items.len().saturating_sub(1));
                let mut panel = div()
                    .id(format!("app-menu-panel-{}-{menu_index}", self.display_id))
                    .role(Role::Menu)
                    .aria_label(format!("{} menu", menu.label))
                    .absolute()
                    .top(px(BAR_HEIGHT + 2.0))
                    .left(px(left))
                    .w(px(MENU_WIDTH))
                    .py_1()
                    .rounded(px(9.0))
                    .bg(rgba(0x202630f4))
                    .border_1()
                    .border_color(rgba(0xffffff35))
                    .shadow_lg()
                    .occlude();
                if let Some(action) = self.pending_system_action.clone() {
                    let (title, detail, confirm) = system_confirmation_copy(&action);
                    let cancel_action = action.clone();
                    let confirm_action = action;
                    panel = panel.child(
                        div()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(div().text_color(rgba(0xf7f8faaa)).child(detail))
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .mt_2()
                                    .child(
                                        div()
                                            .id(format!(
                                                "system-cancel-{}-{cancel_action}",
                                                self.display_id
                                            ))
                                            .role(Role::Button)
                                            .px_3()
                                            .h(px(28.0))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.0))
                                            .bg(rgba(0xffffff1f))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(0xffffff35)))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.pending_system_action = None;
                                                cx.notify();
                                            }))
                                            .child("Cancel"),
                                    )
                                    .child(
                                        div()
                                            .id(format!(
                                                "system-confirm-{}-{confirm_action}",
                                                self.display_id
                                            ))
                                            .role(Role::Button)
                                            .px_3()
                                            .h(px(28.0))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.0))
                                            .bg(rgba(0x2878d4ff))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(0x3488e8ff)))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.close_menu(window, cx);
                                                dispatch_system_menu(confirm_action.clone(), cx);
                                            }))
                                            .child(confirm),
                                    ),
                            ),
                    );
                    return Some(panel);
                }
                for (item_index, item) in menu.items.into_iter().enumerate() {
                    if item.separator_before {
                        panel = panel.child(div().h(px(1.0)).mx_2().my_1().bg(rgba(0xffffff25)));
                    }
                    let action = item.action.clone();
                    let item_app_id = app_id.clone();
                    let enabled = item.enabled;
                    let mut row = div()
                        .id(format!(
                            "app-menu-item-{}-{menu_index}-{item_index}",
                            self.display_id
                        ))
                        .role(Role::MenuItem)
                        .aria_label(item.label.clone())
                        .h(px(MENU_ROW_HEIGHT))
                        .mx_1()
                        .px_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(px(5.0))
                        .when(selected == item_index, |style| style.bg(rgba(0x2878d4ff)))
                        .when(!enabled, |style| style.text_color(rgba(0xf7f8fa66)))
                        .child(item.label);
                    if !item.shortcut.is_empty() {
                        row = row.child(div().text_color(rgba(0xf7f8faaa)).child(item.shortcut));
                    }
                    if enabled {
                        row = row
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0x2878d4ff)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if item_app_id == SYSTEM_MENU_ID
                                    && system_action_needs_confirmation(&action)
                                {
                                    this.pending_system_action = Some(action.clone());
                                    cx.notify();
                                } else {
                                    this.close_menu(window, cx);
                                    dispatch_menu_action(
                                        item_app_id.clone(),
                                        action.clone(),
                                        focused_window_id,
                                        cx,
                                    );
                                }
                            }));
                    }
                    panel = panel.child(row);
                }
                Some(panel)
            });

            let bar = div()
                .id(format!("top-bar-strip-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("rmac top bar")
                .absolute()
                .top(px(if visible { 0.0 } else { -BAR_HEIGHT }))
                .left_0()
                .right_0()
                .h(px(BAR_HEIGHT))
                .flex()
                .items_center()
                .px_4()
                .bg(rgba(0x0b0d143d))
                .text_color(rgba(0xf7f8faff))
                .text_size(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .flex_1()
                        .child(
                            div()
                                .id(format!("desktop-mark-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("rmac menu")
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .when(self.open_menu == Some(0), |style| {
                                    style.bg(rgba(0xffffff2d))
                                })
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    if this.open_menu == Some(0) {
                                        this.close_menu(window, cx);
                                    } else {
                                        this.open_menu(0, SYSTEM_MENU_ID.to_owned(), window, cx);
                                    }
                                }))
                                .child(img(shell_icon_path("rmac.svg")).w(px(15.0)).h(px(15.0))),
                        )
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(active_app))
                        .children(menu_buttons)
                        .children(workspace.map(|workspace| {
                            div()
                                .id(format!("workspace-{}", self.display_id))
                                .text_color(rgba(0xf7f8faaa))
                                .aria_label(format!("Workspace {workspace}"))
                                .child(workspace)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .justify_end()
                        .children(
                            indicators
                                .into_iter()
                                .filter(|indicator| {
                                    indicator.kind != TopBarIndicatorKind::Notifications
                                })
                                .enumerate()
                                .map(|(index, indicator)| {
                                    let icon = indicator_icon_path(indicator.kind);
                                    let mut item = div()
                                        .id(format!("status-{}-{index}", self.display_id))
                                        .role(Role::Button)
                                        .aria_label(indicator.accessible)
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .rounded(px(5.0))
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgba(0xffffff22)))
                                        .on_click(|_, _, cx| {
                                            dispatch_shortcut("quick-settings", cx)
                                        })
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(img(icon).w(px(14.0)).h(px(14.0)));
                                    if !indicator.visible.is_empty() {
                                        item = item.child(indicator.visible);
                                    }
                                    item
                                }),
                        )
                        .child(
                            div()
                                .id(format!("spotlight-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Spotlight")
                                .w(px(22.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("launcher", cx))
                                .child(
                                    img(shell_icon_path("spotlight.svg"))
                                        .w(px(14.0))
                                        .h(px(14.0)),
                                ),
                        )
                        .child(
                            div()
                                .id(format!("control-center-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Control Center")
                                .w(px(22.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("quick-settings", cx))
                                .child(
                                    img(shell_icon_path("control-center.svg"))
                                        .w(px(15.0))
                                        .h(px(15.0)),
                                ),
                        )
                        .child(
                            div()
                                .id(format!("clock-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label(format!(
                                    "Date and time: {clock_label}. Open Notification Center"
                                ))
                                .px_1()
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("notification-center", cx))
                                .font_weight(FontWeight::MEDIUM)
                                .child(clock),
                        ),
                );

            div()
                .id(format!("top-bar-{}", self.display_id))
                .size_full()
                .track_focus(&self.focus)
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.set_pointer_inside(*hovered, cx);
                }))
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.open_menu.is_some() {
                        cx.stop_propagation();
                        this.handle_key(event, window, cx);
                    }
                }))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_menu(window, cx);
                }))
                .child(bar)
                .children(popup)
        }
    }

    fn menu_anchor_x(active_app: &str, menus: &[rmac_app_menu::Menu], index: usize) -> f32 {
        if index == 0 {
            return 4.0;
        }
        let app_width = active_app.chars().count() as f32 * 7.2 + 16.0;
        let preceding = menus
            .iter()
            .skip(1)
            .take(index - 1)
            .map(|menu| menu.label.chars().count() as f32 * 7.0 + 16.0)
            .sum::<f32>();
        (44.0 + app_width + preceding).max(16.0)
    }

    fn system_menu() -> rmac_app_menu::Menu {
        use rmac_app_menu::Item;

        let logout_label = account_display_name()
            .map(|name| format!("Log Out {name}…"))
            .unwrap_or_else(|| "Log Out…".into());
        rmac_app_menu::Menu {
            label: "rmac".into(),
            items: vec![
                Item {
                    label: "About This rmac".into(),
                    action: "system::about".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: false,
                },
                Item {
                    label: "System Settings…".into(),
                    action: "system::settings".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: true,
                },
                Item {
                    label: "App Center".into(),
                    action: "system::app-center".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: false,
                },
                Item {
                    label: "Recent Items".into(),
                    action: "system::recents".into(),
                    shortcut: "›".into(),
                    enabled: true,
                    separator_before: true,
                },
                Item {
                    label: "Force Quit…".into(),
                    action: "system::force-quit".into(),
                    shortcut: "⌥⌘⎋".into(),
                    enabled: true,
                    separator_before: true,
                },
                Item {
                    label: "Sleep".into(),
                    action: "system::sleep".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: true,
                },
                Item {
                    label: "Restart…".into(),
                    action: "system::restart".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: false,
                },
                Item {
                    label: "Shut Down…".into(),
                    action: "system::shutdown".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: false,
                },
                Item {
                    label: "Lock Screen".into(),
                    action: "system::lock".into(),
                    shortcut: "⌃⌘Q".into(),
                    enabled: true,
                    separator_before: true,
                },
                Item {
                    label: logout_label,
                    action: "system::logout".into(),
                    shortcut: "⇧⌘Q".into(),
                    enabled: true,
                    separator_before: false,
                },
            ],
        }
    }

    fn account_display_name() -> Option<String> {
        let username = env::var("USER").ok()?;
        let passwd = fs::read_to_string("/etc/passwd").ok()?;
        passwd.lines().find_map(|line| {
            let fields = line.split(':').collect::<Vec<_>>();
            (fields.len() > 4 && fields[0] == username)
                .then(|| fields[4].split(',').next().unwrap_or_default().trim())
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        })
    }

    fn system_action_needs_confirmation(action: &str) -> bool {
        matches!(
            action,
            "system::restart" | "system::shutdown" | "system::logout"
        )
    }

    fn system_confirmation_copy(action: &str) -> (&'static str, &'static str, &'static str) {
        match action {
            "system::restart" => (
                "Restart this computer?",
                "Open documents may contain unsaved changes.",
                "Restart",
            ),
            "system::shutdown" => (
                "Shut down this computer?",
                "Open documents may contain unsaved changes.",
                "Shut Down",
            ),
            "system::logout" => (
                "Log out now?",
                "Open documents may contain unsaved changes.",
                "Log Out",
            ),
            _ => ("Continue?", "Confirm this system action.", "Continue"),
        }
    }

    fn menu_panel_height(menu: &rmac_app_menu::Menu) -> f32 {
        let separators = menu
            .items
            .iter()
            .filter(|item| item.separator_before)
            .count() as f32;
        8.0 + MENU_ROW_HEIGHT * menu.items.len() as f32 + separators * 9.0
    }

    fn indicator_icon_path(kind: TopBarIndicatorKind) -> PathBuf {
        let file = match kind {
            TopBarIndicatorKind::Focus => "focus.svg",
            TopBarIndicatorKind::Vpn => "vpn.svg",
            TopBarIndicatorKind::Network => "wifi.svg",
            TopBarIndicatorKind::Bluetooth => "bluetooth.svg",
            TopBarIndicatorKind::Sound => "sound.svg",
            TopBarIndicatorKind::Battery => "battery.svg",
            TopBarIndicatorKind::Notifications => "notifications.svg",
        };
        shell_icon_path(file)
    }

    fn shell_icon_path(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/status")
            .join(file)
    }

    fn dispatch_shortcut(shortcut: &'static str, cx: &mut App) {
        let local = env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/libexec/rmac/rmac-shortcut-dispatch"));
        let dispatcher = local
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/usr/libexec/rmac/rmac-shortcut-dispatch"));
        cx.background_executor()
            .spawn(async move {
                let result = blocking::unblock(move || {
                    Command::new(dispatcher).arg(shortcut).spawn().map(|_| ())
                })
                .await;
                if result.is_err() {
                    eprintln!("could not open {shortcut}");
                }
            })
            .detach();
    }

    fn dispatch_menu_action(
        app_id: String,
        action: String,
        window_id: Option<rmac_compositor::WindowId>,
        cx: &mut App,
    ) {
        if app_id == SYSTEM_MENU_ID {
            dispatch_system_menu(action, cx);
            return;
        }
        static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(1);
        cx.background_executor()
            .spawn(async move {
                if let Some(window) = window_id {
                    let id = NEXT_ACTIVATION.fetch_add(1, Ordering::Relaxed).max(1);
                    let request = rmac_compositor::ActionRequest {
                        id: rmac_compositor::ActivationId(id),
                        action: rmac_compositor::Action::FocusWindow { window },
                    };
                    if rmac_compositor_niri::execute(request).await.result.is_err() {
                        eprintln!("could not return focus to the application menu owner");
                    }
                }
                if let Err(error) = rmac_app_menu::activate(&app_id, &action).await {
                    eprintln!("could not activate {app_id} menu command: {error}");
                }
            })
            .detach();
    }

    fn dispatch_system_menu(action: String, cx: &mut App) {
        match action.as_str() {
            "system::about" => {
                spawn_command("/usr/bin/rmac-system-settings", &["--pane", "general"], cx)
            }
            "system::settings" => spawn_command("/usr/bin/rmac-system-settings", &[], cx),
            "system::app-center" => spawn_command("gtk-launch", &["snap-store_snap-store"], cx),
            "system::recents" => dispatch_shortcut("launcher", cx),
            "system::force-quit" => spawn_command("/usr/bin/rmac-system-monitor", &[], cx),
            "system::sleep" => spawn_command("systemctl", &["suspend"], cx),
            "system::restart" => spawn_command("systemctl", &["reboot"], cx),
            "system::shutdown" => spawn_command("systemctl", &["poweroff"], cx),
            "system::lock" => dispatch_shortcut("lock", cx),
            "system::logout" => spawn_command(
                "niri",
                &["msg", "action", "quit", "--skip-confirmation"],
                cx,
            ),
            _ => eprintln!("unknown rmac system menu action: {action}"),
        }
    }

    fn spawn_command(program: &'static str, args: &'static [&'static str], cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let result =
                    blocking::unblock(move || Command::new(program).args(args).spawn().map(|_| ()))
                        .await;
                if let Err(error) = result {
                    eprintln!("could not run {program}: {error}");
                }
            })
            .detach();
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
            .unwrap_or_else(|error| panic!("open top-bar evidence {path:?}: {error}"));
        writeln!(file, "display={display_id} scale={scale}")
            .unwrap_or_else(|error| panic!("write top-bar evidence {path:?}: {error}"));
    }

    fn record_render_count(window: &Window, display_id: u64, render_count: u64) {
        let Some(directory) = env::var_os(RENDER_COUNT_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        window.on_next_frame(move |_, _| {
            fs::create_dir_all(&directory).unwrap_or_else(|error| {
                panic!("create top-bar render evidence {directory:?}: {error}")
            });
            let path = directory.join(format!("{display_id}.count"));
            fs::write(&path, format!("{render_count}\n"))
                .unwrap_or_else(|error| panic!("write top-bar render count {path:?}: {error}"));
        });
    }

    fn start_status(cx: &mut App) -> Entity<ShellStatus> {
        let (status_tx, status_rx) = async_channel::bounded(16);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_shell_runtime::watch(status_tx).await {
                    eprintln!("shell status runtime stopped: {error}");
                }
            })
            .detach();
        cx.new(|cx| ShellStatus::new(status_rx, cx))
    }

    #[derive(Default)]
    struct TopBarTracker {
        windows: BTreeMap<Uuid, (bool, AnyWindowHandle)>,
    }

    impl TopBarTracker {
        fn len(&self) -> usize {
            self.windows.len()
        }

        fn reconcile(
            &mut self,
            desired: Option<&BTreeMap<Uuid, bool>>,
            status: &Entity<ShellStatus>,
            cx: &mut App,
        ) {
            let available = rmac_gpui_upstream_lab::output_surfaces::newest_displays(cx);
            let target = desired
                .map(|desired| {
                    desired
                        .iter()
                        .filter(|(uuid, _)| available.contains_key(uuid))
                        .map(|(uuid, fullscreen)| (*uuid, *fullscreen))
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_else(|| available.keys().map(|uuid| (*uuid, false)).collect());

            let unavailable = self
                .windows
                .keys()
                .filter(|uuid| !target.contains_key(uuid))
                .copied()
                .collect::<Vec<_>>();
            for uuid in unavailable {
                if let Some((_, handle)) = self.windows.remove(&uuid) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }

            for (uuid, fullscreen) in target {
                if self
                    .windows
                    .get(&uuid)
                    .is_some_and(|(current, _)| *current == fullscreen)
                {
                    continue;
                }
                if let Some(display) = available.get(&uuid) {
                    let handle = open_top_bar(display.clone(), status.clone(), fullscreen, cx);
                    if let Some((_, previous)) = self.windows.insert(uuid, (fullscreen, handle)) {
                        let _ = previous.update(cx, |_, window, _| window.remove_window());
                    }
                }
            }
        }
    }

    fn open_top_bar(
        display: Rc<dyn PlatformDisplay>,
        status: Entity<ShellStatus>,
        fullscreen: bool,
        cx: &mut App,
    ) -> AnyWindowHandle {
        let display_id = display.id();
        let width = display.bounds().size.width;
        let handle = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: Size::new(width, px(MENU_SURFACE_HEIGHT)),
                    })),
                    display_id: Some(display_id),
                    app_id: Some("dev.rmac.TopBar".to_owned()),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: format!("rmac-top-bar-{}", u64::from(display_id)),
                        layer: Layer::Top,
                        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                        keyboard_interactivity: KeyboardInteractivity::OnDemand,
                        exclusive_zone: Some(px(if fullscreen { 0.0 } else { BAR_HEIGHT })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                {
                    let status = status.clone();
                    move |window, cx| {
                        cx.new(|cx| TopBar::new(display_id, status, fullscreen, window, cx))
                    }
                },
            )
            .expect("open top-bar layer surface");
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
            let status = start_status(cx);
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_gpui_upstream_lab::output_surfaces::watch_top_bar(output_tx).await
                    {
                        eprintln!("top-bar output watcher unavailable: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx| {
                let mut tracker = TopBarTracker::default();
                let mut removed_outputs = BTreeSet::new();
                match output_rx.recv().await {
                    Ok(mut desired) => 'updates: loop {
                        let complete = cx.update(|cx| {
                            tracker.reconcile(Some(&desired), &status, cx);
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
                        cx.update(|cx| tracker.reconcile(None, &status, cx));
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
        previous: &BTreeMap<Uuid, bool>,
        current: &BTreeMap<Uuid, bool>,
        removed: &mut BTreeSet<Uuid>,
    ) {
        let previous = previous.keys().copied().collect();
        let current = current.keys().copied().collect();
        if rmac_gpui_upstream_lab::output_reappeared(&previous, &current, removed) {
            std::process::exit(rmac_gpui_upstream_lab::WAYLAND_OUTPUT_RESTART_EXIT_CODE);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("top-bar requires Linux and: cargo run --features wayland --bin top-bar");
    std::process::exit(2);
}
