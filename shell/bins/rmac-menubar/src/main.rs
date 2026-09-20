#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::borrow::Cow;
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::Local;
    use futures_util::FutureExt as _;
    use gpui::{
        div, layer_shell::*, point, prelude::*, px, rgba, svg, AnyWindowHandle, App, AssetSource,
        Bounds, Context, DisplayId, Entity, FocusHandle, FontWeight, KeyDownEvent, PlatformDisplay,
        QuitMode, Role, SharedString, Size, Subscription, Window, WindowBackgroundAppearance,
        WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_shell_ui::tokens;
    use rmac_shell_ui::{
        app_display_name, delay_until_next_clock_tick, top_bar_active_app_name,
        top_bar_clock_pattern, top_bar_indicator_labels, top_bar_workspace_label,
        TopBarIndicatorKind,
    };
    use uuid::Uuid;

    // Measured from the reference Mac 2026-09-18 (FEEL_SPEC.md §C.2): the bar
    // occupies rows 0–28 and is fully transparent.
    const BAR_HEIGHT: f32 = 29.0;
    const MENU_SURFACE_HEIGHT: f32 = 520.0;
    const MENU_WIDTH: f32 = 248.0;
    const RECENT_MENU_WIDTH: f32 = 286.0;
    const MENU_ROW_HEIGHT: f32 = 28.0;
    const MAX_RECENT_ITEMS: usize = 10;
    const FULLSCREEN_REVEAL_EDGE: f32 = 2.0;
    const FULLSCREEN_HIDE_DELAY: Duration = Duration::from_millis(500);
    const SYSTEM_MENU_ID: &str = "org.rmac.Desktop.SystemMenu";
    const READY_FILE_ENV: &str = "RMAC_TOP_BAR_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_TOP_BAR_RENDER_COUNT_DIR";

    const STATUS_ASSET_NAMES: [&str; 10] = [
        "status/battery.svg",
        "status/bluetooth.svg",
        "status/control-center.svg",
        "status/focus.svg",
        "status/notifications.svg",
        "status/rmac.svg",
        "status/sound.svg",
        "status/spotlight.svg",
        "status/vpn.svg",
        "status/wifi.svg",
    ];

    struct MenuBarAssets;

    impl AssetSource for MenuBarAssets {
        fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            let bytes: Option<&'static [u8]> = match path {
                "status/battery.svg" => Some(include_bytes!("../../../assets/status/battery.svg")),
                "status/bluetooth.svg" => {
                    Some(include_bytes!("../../../assets/status/bluetooth.svg"))
                }
                "status/control-center.svg" => {
                    Some(include_bytes!("../../../assets/status/control-center.svg"))
                }
                "status/focus.svg" => Some(include_bytes!("../../../assets/status/focus.svg")),
                "status/notifications.svg" => {
                    Some(include_bytes!("../../../assets/status/notifications.svg"))
                }
                "status/rmac.svg" => Some(include_bytes!("../../../assets/status/rmac.svg")),
                "status/sound.svg" => Some(include_bytes!("../../../assets/status/sound.svg")),
                "status/spotlight.svg" => {
                    Some(include_bytes!("../../../assets/status/spotlight.svg"))
                }
                "status/vpn.svg" => Some(include_bytes!("../../../assets/status/vpn.svg")),
                "status/wifi.svg" => Some(include_bytes!("../../../assets/status/wifi.svg")),
                _ => None,
            };
            Ok(bytes.map(Cow::Borrowed))
        }

        fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
            Ok(STATUS_ASSET_NAMES
                .iter()
                .filter(|asset| asset.starts_with(path))
                .map(|asset| SharedString::from(*asset))
                .collect())
        }
    }

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
                            // The compositor projection can momentarily report no
                            // focused window during unrelated events; keep the
                            // last known app's menus rather than clearing them.
                            if let Some(app_id) = focused_app
                                .filter(|app_id| Some(app_id) != this.menu_app_id.as_ref())
                            {
                                this.menu_app_id = Some(app_id.clone());
                                this.menus.clear();
                                this.menu_generation = this.menu_generation.saturating_add(1);
                                if rmac_app_menu::bus_name(&app_id).is_some() {
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
            // The focused app may still be registering its menu interface on
            // D-Bus when focus first arrives, so retry a few times while it
            // stays focused instead of leaving the bar permanently empty.
            for _ in 0..6 {
                let result = cx
                    .background_executor()
                    .spawn({
                        let app_id = app_id.clone();
                        async move { rmac_app_menu::fetch(&app_id).await }
                    })
                    .await;
                let fetched = result.ok().filter(|menus| !menus.is_empty());
                let stop = this
                    .update(cx, |this, cx| {
                        if this.menu_generation != generation
                            || this.menu_app_id.as_deref() != Some(app_id.as_str())
                        {
                            return true;
                        }
                        if let Some(menus) = fetched {
                            this.menus = menus;
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
                async_io::Timer::after(std::time::Duration::from_millis(300)).await;
            }
        })
        .detach();
    }

    #[derive(Clone, Debug, PartialEq)]
    struct MenuBackdropPanel {
        left: f32,
        top: f32,
        width: f32,
        height: f32,
    }

    type MenuBackdropUpdate = (Uuid, Option<Vec<MenuBackdropPanel>>);

    struct TopBar {
        display_id: u64,
        output_uuid: Uuid,
        render_count: u64,
        status: Entity<ShellStatus>,
        open_menu: Option<usize>,
        selected_item: usize,
        open_app_id: Option<String>,
        recent_items: Vec<PathBuf>,
        recent_items_loading: bool,
        recent_items_unavailable: bool,
        recent_generation: u64,
        recent_submenu_open: bool,
        recent_selected_item: usize,
        pending_system_action: Option<String>,
        fullscreen: bool,
        revealed: bool,
        pointer_inside: bool,
        hide_generation: u64,
        parking: rmac_compositor::ParkingStore,
        backdrop_panels: Option<Vec<MenuBackdropPanel>>,
        backdrop_tx: async_channel::Sender<MenuBackdropUpdate>,
        focus: FocusHandle,
        _blur: Subscription,
    }

    impl TopBar {
        fn new(
            display_id: DisplayId,
            output_uuid: Uuid,
            status: Entity<ShellStatus>,
            fullscreen: bool,
            backdrop_tx: async_channel::Sender<MenuBackdropUpdate>,
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
                output_uuid,
                render_count: 0,
                status,
                open_menu: None,
                selected_item: 0,
                open_app_id: None,
                recent_items: Vec::new(),
                recent_items_loading: false,
                recent_items_unavailable: false,
                recent_generation: 0,
                recent_submenu_open: false,
                recent_selected_item: 0,
                pending_system_action: None,
                fullscreen,
                revealed: !fullscreen,
                pointer_inside: false,
                hide_generation: 0,
                parking: rmac_compositor::ParkingStore::load_default(),
                backdrop_panels: None,
                backdrop_tx,
                focus,
                _blur: blur,
            }
        }

        fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.open_menu.take().is_some() {
                self.open_app_id = None;
                self.recent_submenu_open = false;
                self.recent_selected_item = 0;
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
            self.parking = rmac_compositor::ParkingStore::load_default();
            self.recent_submenu_open = false;
            self.recent_selected_item = 0;
            self.pending_system_action = None;
            if index == 0 {
                self.load_recent_items(cx);
            }
            window.focus(&self.focus, cx);
            window.refresh();
            cx.notify();
        }

        fn load_recent_items(&mut self, cx: &mut Context<Self>) {
            self.recent_generation = self.recent_generation.saturating_add(1);
            let generation = self.recent_generation;
            self.recent_items_loading = true;
            self.recent_items_unavailable = false;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        blocking::unblock(move || {
                            let cancel = AtomicBool::new(false);
                            let mut options = rmac_search::Options::new(&cancel);
                            options.limit = MAX_RECENT_ITEMS;
                            rmac_search::recents(options)
                        })
                        .await
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.recent_generation != generation {
                        return;
                    }
                    this.recent_items_loading = false;
                    match result {
                        Ok(paths) => {
                            this.recent_items = paths;
                            this.recent_items_unavailable = false;
                        }
                        Err(_) => {
                            this.recent_items.clear();
                            this.recent_items_unavailable = true;
                        }
                    }
                    this.recent_selected_item = 0;
                    cx.notify();
                });
            })
            .detach();
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
            if self.recent_submenu_open {
                let action_count = recent_action_count(
                    &self.recent_items,
                    self.recent_items_loading,
                    self.recent_items_unavailable,
                );
                match event.keystroke.key.as_str() {
                    "escape" | "left" => {
                        self.recent_submenu_open = false;
                        self.recent_selected_item = 0;
                        cx.notify();
                    }
                    "down" if action_count > 0 => {
                        self.recent_selected_item = (self.recent_selected_item + 1) % action_count;
                        cx.notify();
                    }
                    "up" if action_count > 0 => {
                        self.recent_selected_item = self
                            .recent_selected_item
                            .checked_sub(1)
                            .unwrap_or(action_count - 1);
                        cx.notify();
                    }
                    "enter" | "space" if action_count > 0 => {
                        let selected = self.recent_selected_item.min(action_count - 1);
                        let path = self.recent_items.get(selected).cloned();
                        let clear = selected == self.recent_items.len();
                        self.close_menu(window, cx);
                        if let Some(path) = path {
                            dispatch_recent_item(path, cx);
                        } else if clear {
                            clear_recent_items(cx);
                        }
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
                    self.recent_submenu_open = false;
                    cx.notify();
                }
                "up" => {
                    self.selected_item = self
                        .selected_item
                        .checked_sub(1)
                        .unwrap_or(menu.items.len() - 1);
                    self.recent_submenu_open = false;
                    cx.notify();
                }
                "right"
                    if menu_index == 0
                        && menu
                            .items
                            .get(self.selected_item)
                            .is_some_and(|item| item.action == "system::recents") =>
                {
                    self.recent_submenu_open = true;
                    self.recent_selected_item = 0;
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
                    self.recent_submenu_open = false;
                    cx.notify();
                }
                "enter" | "space" => {
                    if let (Some(app_id), Some(item)) = (
                        self.open_app_id.clone(),
                        menu.items.get(self.selected_item).cloned(),
                    ) {
                        if app_id == SYSTEM_MENU_ID && item.action == "system::recents" {
                            self.recent_submenu_open = true;
                            self.recent_selected_item = 0;
                            cx.notify();
                        } else if app_id == SYSTEM_MENU_ID
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
            // Opening a popup moves keyboard focus to this layer surface, so the
            // live focused window drops to None. Use the last app the status
            // runtime reported (kept across those blips) for the app menu, and
            // name it rather than letting the desktop identity take over.
            let active_app_id = status
                .menu_app_id
                .clone()
                .or_else(|| snapshot.focused.app_id.clone());
            let active_app = if self.open_menu.is_some() {
                active_app_id
                    .as_deref()
                    .map(app_display_name)
                    .unwrap_or_else(|| top_bar_active_app_name(snapshot))
            } else {
                top_bar_active_app_name(snapshot)
            };
            let workspace = top_bar_workspace_label(snapshot);
            let indicators = top_bar_indicator_labels(snapshot);
            let focused_window_id = snapshot.focused.window_id;
            let mut menus = status.menus.clone();
            menus.insert(0, system_menu());
            // The bold app name is the app menu (§3.3): it is synthesized, not
            // exported, so every app gets About/Hide/Hide Others/Show All/Quit.
            menus.insert(
                1,
                app_menu(
                    &active_app,
                    active_app_id.as_deref(),
                    !self.parking.is_empty(),
                ),
            );

            if self.open_menu.is_some()
                && (self.open_menu >= Some(menus.len())
                    || (self.open_menu != Some(0)
                        && status
                            .menu_app_id
                            .as_deref()
                            .is_some_and(|id| self.open_app_id.as_deref() != Some(id))))
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
            let recent_submenu_top = self.recent_submenu_open.then(|| {
                menus
                    .first()
                    .and_then(|menu| {
                        menu.items
                            .iter()
                            .position(|item| item.action == "system::recents")
                            .map(|index| menu_item_top(menu, index))
                    })
                    .unwrap_or(BAR_HEIGHT + 2.0)
            });
            let backdrop_panels = if visible {
                match (menu_left, menu_height) {
                    (Some(left), Some(height)) => {
                        let mut panels = vec![MenuBackdropPanel {
                            left,
                            top: BAR_HEIGHT + 2.0,
                            width: MENU_WIDTH,
                            height,
                        }];
                        if let Some(top) = recent_submenu_top {
                            panels.push(MenuBackdropPanel {
                                left: left + MENU_WIDTH - 4.0,
                                top,
                                width: RECENT_MENU_WIDTH,
                                height: recent_menu_height(
                                    self.recent_items.len(),
                                    self.recent_items_loading,
                                    self.recent_items_unavailable,
                                ),
                            });
                        }
                        Some(panels)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if self.backdrop_panels != backdrop_panels {
                self.backdrop_panels = backdrop_panels.clone();
                let _ = self
                    .backdrop_tx
                    .try_send((self.output_uuid, backdrop_panels));
            }
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
            let mut input_regions = vec![bar_region];
            if visible {
                if let (Some(left), Some(height)) = (menu_left, menu_height) {
                    input_regions.push(Bounds {
                        origin: point(px(left), px(BAR_HEIGHT)),
                        size: Size::new(px(MENU_WIDTH), px(height + 4.0)),
                    });
                    if let Some(top) = recent_submenu_top {
                        input_regions.push(Bounds {
                            origin: point(px(left + MENU_WIDTH - 4.0), px(top)),
                            size: Size::new(
                                px(RECENT_MENU_WIDTH),
                                px(recent_menu_height(
                                    self.recent_items.len(),
                                    self.recent_items_loading,
                                    self.recent_items_unavailable,
                                )),
                            ),
                        });
                    }
                }
            }
            window.set_input_region(Some(&input_regions));

            let app_id_for_buttons = status.menu_app_id.clone();
            let menu_buttons = menus
                .iter()
                .enumerate()
                .skip(2)
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
                        .rounded(px(tokens::menu_item_radius()))
                        .cursor_pointer()
                        .when(open, |style| style.bg(rgba(tokens::light_selection())))
                        .hover(|style| style.bg(rgba(tokens::light_hover())))
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
                    // Use the app the menu was opened for; the live focus
                    // projection can blip to None during unrelated events.
                    self.open_app_id.clone()?
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
                    .rounded(px(tokens::menu_radius()))
                    .bg(rgba(tokens::transparent()))
                    .text_size(px(tokens::body_text_size()))
                    .text_color(rgba(tokens::primary_text()))
                    .border_1()
                    .border_color(rgba(tokens::light_border()))
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
                            .child(
                                div()
                                    .text_color(rgba(tokens::secondary_text()))
                                    .child(detail),
                            )
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
                                            .rounded(px(tokens::menu_item_radius()))
                                            .bg(rgba(tokens::separator()))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(tokens::separator())))
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
                                            .rounded(px(tokens::menu_item_radius()))
                                            .bg(rgba(tokens::accent()))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(tokens::accent_hover())))
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
                        panel = panel
                            .child(div().h(px(1.0)).mx_2().my_1().bg(rgba(tokens::separator())));
                    }
                    let action = item.action.clone();
                    let item_app_id = app_id.clone();
                    let enabled = item.enabled;
                    let opens_recents =
                        item_app_id == SYSTEM_MENU_ID && action == "system::recents";
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
                        .rounded(px(tokens::menu_item_radius()))
                        .when(selected == item_index, |style| {
                            style.bg(rgba(tokens::accent()))
                        })
                        .when(!enabled, |style| {
                            style.text_color(rgba(tokens::disabled_text()))
                        })
                        .child(item.label);
                    if !item.shortcut.is_empty() {
                        row = row.child(
                            div()
                                .text_color(rgba(tokens::secondary_text()))
                                .child(item.shortcut),
                        );
                    }
                    if enabled {
                        row = row
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(tokens::accent())))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered && this.recent_submenu_open != opens_recents {
                                    this.recent_submenu_open = opens_recents;
                                    this.recent_selected_item = 0;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if item_app_id == SYSTEM_MENU_ID && action == "system::recents" {
                                    this.recent_submenu_open = true;
                                    this.recent_selected_item = 0;
                                    cx.notify();
                                } else if item_app_id == SYSTEM_MENU_ID
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
                let mut surfaces = div()
                    .id(format!("menu-surfaces-{}-{menu_index}", self.display_id))
                    .child(panel);
                if self.recent_submenu_open && menu_index == 0 {
                    let submenu_top = recent_submenu_top?;
                    let selected = self.recent_selected_item;
                    let mut submenu = div()
                        .id(format!("recent-items-panel-{}", self.display_id))
                        .role(Role::Menu)
                        .aria_label("Recent Items")
                        .absolute()
                        .top(px(submenu_top))
                        .left(px(left + MENU_WIDTH - 4.0))
                        .w(px(RECENT_MENU_WIDTH))
                        .py_1()
                        .rounded(px(tokens::menu_radius()))
                        .bg(rgba(tokens::transparent()))
                        .text_size(px(tokens::body_text_size()))
                        .text_color(rgba(tokens::primary_text()))
                        .border_1()
                        .border_color(rgba(tokens::light_border()))
                        .shadow_lg()
                        .occlude()
                        .child(
                            div()
                                .h(px(22.0))
                                .px_3()
                                .flex()
                                .items_center()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgba(tokens::disabled_text()))
                                .child("Documents"),
                        );
                    if self.recent_items_loading {
                        submenu = submenu.child(recent_status_row("Loading…"));
                    } else if self.recent_items_unavailable {
                        submenu = submenu.child(recent_status_row("Recent Items Unavailable"));
                    } else if self.recent_items.is_empty() {
                        submenu = submenu.child(recent_status_row("None"));
                    } else {
                        for (index, path) in self.recent_items.iter().cloned().enumerate() {
                            let label = recent_item_label(&path);
                            submenu = submenu.child(
                                div()
                                    .id(format!("recent-item-{}-{index}", self.display_id))
                                    .role(Role::MenuItem)
                                    .aria_label(format!("Open {label}"))
                                    .h(px(MENU_ROW_HEIGHT))
                                    .mx_1()
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .rounded(px(tokens::menu_item_radius()))
                                    .when(selected == index, |style| {
                                        style.bg(rgba(tokens::accent()))
                                    })
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(tokens::accent())))
                                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                        if *hovered && this.recent_selected_item != index {
                                            this.recent_selected_item = index;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_menu(window, cx);
                                        dispatch_recent_item(path.clone(), cx);
                                    }))
                                    .child(label),
                            );
                        }
                        let clear_index = self.recent_items.len();
                        submenu = submenu
                            .child(div().h(px(1.0)).mx_2().my_1().bg(rgba(tokens::separator())))
                            .child(
                                div()
                                    .id(format!("recent-items-clear-{}", self.display_id))
                                    .role(Role::MenuItem)
                                    .aria_label("Clear Recent Items")
                                    .h(px(MENU_ROW_HEIGHT))
                                    .mx_1()
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .rounded(px(tokens::menu_item_radius()))
                                    .when(selected == clear_index, |style| {
                                        style.bg(rgba(tokens::accent()))
                                    })
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgba(tokens::accent())))
                                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                        if *hovered && this.recent_selected_item != clear_index {
                                            this.recent_selected_item = clear_index;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_menu(window, cx);
                                        clear_recent_items(cx);
                                    }))
                                    .child("Clear Menu"),
                            );
                    }
                    surfaces = surfaces.child(submenu);
                }
                Some(surfaces)
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
                .text_color(rgba(tokens::menubar_text()))
                .text_size(px(tokens::body_text_size()))
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
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .when(self.open_menu == Some(0), |style| {
                                    style.bg(rgba(tokens::light_selection()))
                                })
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    if this.open_menu == Some(0) {
                                        this.close_menu(window, cx);
                                    } else {
                                        this.open_menu(0, SYSTEM_MENU_ID.to_owned(), window, cx);
                                    }
                                }))
                                .child(
                                    svg()
                                        .path(shell_icon_path("rmac.svg"))
                                        .w(px(15.0))
                                        .h(px(15.0))
                                        .text_color(rgba(tokens::primary_text())),
                                ),
                        )
                        .child({
                            let app_id = active_app_id.clone().unwrap_or_default();
                            let open = self.open_menu == Some(1);
                            div()
                                .id(format!("app-menu-name-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label(format!("{active_app} menu"))
                                .focusable()
                                .tab_stop(true)
                                .h(px(22.0))
                                .px_1()
                                .flex()
                                .items_center()
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .when(open, |style| style.bg(rgba(tokens::light_selection())))
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    if this.open_menu == Some(1) {
                                        this.close_menu(window, cx);
                                    } else {
                                        this.open_menu(1, app_id.clone(), window, cx);
                                    }
                                }))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(active_app.clone()),
                                )
                        })
                        .children(menu_buttons)
                        .children(workspace.map(|workspace| {
                            div()
                                .id(format!("workspace-{}", self.display_id))
                                .text_color(rgba(tokens::secondary_text()))
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
                                        .rounded(px(tokens::menu_item_radius()))
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgba(tokens::light_hover())))
                                        .on_click(|_, _, cx| {
                                            dispatch_shortcut("quick-settings", cx)
                                        })
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(
                                            svg()
                                                .path(icon)
                                                .w(px(14.0))
                                                .h(px(14.0))
                                                .text_color(rgba(tokens::primary_text())),
                                        );
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
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
                                .on_click(|_, _, cx| dispatch_shortcut("launcher", cx))
                                .child(
                                    svg()
                                        .path(shell_icon_path("spotlight.svg"))
                                        .w(px(14.0))
                                        .h(px(14.0))
                                        .text_color(rgba(tokens::primary_text())),
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
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
                                .on_click(|_, _, cx| dispatch_shortcut("quick-settings", cx))
                                .child(
                                    svg()
                                        .path(shell_icon_path("control-center.svg"))
                                        .w(px(15.0))
                                        .h(px(15.0))
                                        .text_color(rgba(tokens::primary_text())),
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
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
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
        // Index 1 is the synthesized app menu, drawn as the bold app name
        // itself, so only menus after it add width before `index`.
        let preceding = menus
            .iter()
            .skip(2)
            .take(index.saturating_sub(2))
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
                    label: "Software Center".into(),
                    action: "system::software-center".into(),
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

    /// The bold-name app menu every application gets, exported or not (§3.3).
    /// Hide and Hide Others park the app's windows; Show All unparks them.
    fn app_menu(app_name: &str, app_id: Option<&str>, any_parked: bool) -> rmac_app_menu::Menu {
        use rmac_app_menu::Item;

        let known = app_id.is_some();
        let row =
            |label: String, action: &str, shortcut: &str, enabled: bool, separator_before| Item {
                label,
                action: action.into(),
                shortcut: shortcut.into(),
                enabled,
                separator_before,
            };
        let mut items = vec![
            // No app ships About metadata yet, so the row is present but
            // disabled rather than inventing facts (§FD-8).
            row(format!("About {app_name}"), "app::about", "", false, false),
            row("Services".into(), "app::services", "›", false, true),
            row(format!("Hide {app_name}"), "app::hide", "⌘H", known, true),
            row(
                "Hide Others".into(),
                "app::hide-others",
                "⌥⌘H",
                known,
                false,
            ),
            row("Show All".into(), "app::show-all", "", any_parked, false),
        ];
        // Files, like Finder, can never be quit (§2.9).
        if app_id != Some(rmac_apps::identity::FILES) {
            items.push(row(
                format!("Quit {app_name}"),
                "app::quit",
                "⌘Q",
                known,
                true,
            ));
        }
        rmac_app_menu::Menu {
            label: app_name.to_owned(),
            items,
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

    fn menu_item_top(menu: &rmac_app_menu::Menu, index: usize) -> f32 {
        let separators = menu
            .items
            .iter()
            .take(index + 1)
            .filter(|item| item.separator_before)
            .count() as f32;
        BAR_HEIGHT + 6.0 + MENU_ROW_HEIGHT * index as f32 + separators * 9.0
    }

    fn recent_action_count(items: &[PathBuf], loading: bool, unavailable: bool) -> usize {
        if loading || unavailable || items.is_empty() {
            0
        } else {
            items.len() + 1
        }
    }

    fn recent_menu_height(items: usize, loading: bool, unavailable: bool) -> f32 {
        let rows = if loading || unavailable || items == 0 {
            1
        } else {
            items + 1
        };
        30.0 + rows as f32 * MENU_ROW_HEIGHT
            + if items > 0 && !loading && !unavailable {
                9.0
            } else {
                0.0
            }
    }

    fn recent_status_row(label: &'static str) -> impl IntoElement {
        div()
            .id(format!("recent-items-status-{label}"))
            .role(Role::MenuItem)
            .aria_label(label)
            .h(px(MENU_ROW_HEIGHT))
            .mx_1()
            .px_2()
            .flex()
            .items_center()
            .text_color(rgba(tokens::disabled_text()))
            .child(label)
    }

    fn recent_item_label(path: &std::path::Path) -> String {
        const MAX_CHARACTERS: usize = 38;
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Document".into());
        let mut characters = label.chars();
        let shortened = characters.by_ref().take(MAX_CHARACTERS).collect::<String>();
        if characters.next().is_some() {
            format!("{shortened}…")
        } else {
            shortened
        }
    }

    fn indicator_icon_path(kind: TopBarIndicatorKind) -> &'static str {
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

    fn shell_icon_path(file: &'static str) -> &'static str {
        match file {
            "battery.svg" => "status/battery.svg",
            "bluetooth.svg" => "status/bluetooth.svg",
            "control-center.svg" => "status/control-center.svg",
            "focus.svg" => "status/focus.svg",
            "notifications.svg" => "status/notifications.svg",
            "rmac.svg" => "status/rmac.svg",
            "sound.svg" => "status/sound.svg",
            "spotlight.svg" => "status/spotlight.svg",
            "vpn.svg" => "status/vpn.svg",
            "wifi.svg" => "status/wifi.svg",
            _ => unreachable!("unknown menu-bar icon"),
        }
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
        if action.starts_with("app::") {
            dispatch_app_menu_action(app_id, action, cx);
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

    /// Hide/Hide Others/Show All use the parking model (§2.2): niri has no
    /// minimize, so a window is hidden by moving it to `rmac-parking` and its
    /// origin workspace is recorded so Show All can bring it back.
    fn dispatch_app_menu_action(app_id: String, action: String, cx: &mut App) {
        cx.spawn(async move |_cx: &mut gpui::AsyncApp| {
            let Ok(snapshot) = rmac_compositor_niri::snapshot().await else {
                eprintln!("could not read windows to {action}");
                return;
            };
            let mut store = rmac_compositor::ParkingStore::load_default();
            store.prune(&snapshot);
            let mut actions = Vec::new();
            match action.as_str() {
                "app::hide" => {
                    let windows = rmac_compositor::application_windows(&snapshot, &app_id);
                    store.record_from(&snapshot, &windows);
                    actions = windows
                        .into_iter()
                        .map(|window| rmac_compositor::Action::MinimizeWindow { window })
                        .collect();
                }
                "app::hide-others" => {
                    let windows = rmac_compositor::visible_windows_except(&snapshot, &app_id);
                    store.record_from(&snapshot, &windows);
                    actions = windows
                        .into_iter()
                        .map(|window| rmac_compositor::Action::MinimizeWindow { window })
                        .collect();
                }
                "app::show-all" => {
                    let windows = store
                        .entries()
                        .iter()
                        .map(|entry| entry.window)
                        .collect::<Vec<_>>();
                    actions = store.restore_actions(&windows);
                }
                "app::quit" => {
                    let windows = rmac_compositor::windows_of_application(&snapshot, &app_id);
                    for window in &windows {
                        store.forget(*window);
                    }
                    actions = windows
                        .into_iter()
                        .map(|window| rmac_compositor::Action::CloseWindow { window })
                        .collect();
                }
                other => eprintln!("unknown app menu action: {other}"),
            }
            for action in &actions {
                if let Err(error) = rmac_compositor_niri::execute_action(action).await {
                    eprintln!("could not run app menu action: {error:?}");
                }
            }
            if let Err(error) = store.save_default() {
                eprintln!("could not save the parking set: {error}");
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
            "system::software-center" => {
                spawn_command("gtk-launch", &["snap-store_snap-store"], cx)
            }
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

    fn dispatch_recent_item(path: PathBuf, cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                if rmac_app_launch::open_item(path).await.is_err() {
                    eprintln!("could not open the recent item");
                }
            })
            .detach();
    }

    fn clear_recent_items(cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let result = blocking::unblock(move || {
                    rmac_recent_documents::Store::from_environment().and_then(|store| store.clear())
                })
                .await;
                if result.is_err() {
                    eprintln!("could not clear Recent Items");
                }
            })
            .detach();
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

    struct MenuBackdrop;

    impl Render for MenuBackdrop {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .rounded(px(tokens::menu_radius()))
                .bg(rgba(tokens::regular_dark_tint()))
        }
    }

    #[derive(Default)]
    struct MenuBackdropTracker {
        windows: BTreeMap<Uuid, (Vec<MenuBackdropPanel>, Vec<AnyWindowHandle>)>,
    }

    impl MenuBackdropTracker {
        fn update(&mut self, output: Uuid, desired: Option<Vec<MenuBackdropPanel>>, cx: &mut App) {
            if self
                .windows
                .get(&output)
                .is_some_and(|(current, _)| Some(current) == desired.as_ref())
            {
                return;
            }
            if let Some((_, handles)) = self.windows.remove(&output) {
                for handle in handles {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }
            let Some(panels) = desired else {
                return;
            };
            let Some(display) = rmac_shell_layer::output_surfaces::newest_displays(cx)
                .get(&output)
                .cloned()
            else {
                return;
            };
            let handles = panels
                .iter()
                .enumerate()
                .map(|(index, panel)| open_menu_backdrop(display.clone(), output, index, panel, cx))
                .collect();
            self.windows.insert(output, (panels, handles));
        }
    }

    fn open_menu_backdrop(
        display: Rc<dyn PlatformDisplay>,
        output: Uuid,
        index: usize,
        panel: &MenuBackdropPanel,
        cx: &mut App,
    ) -> AnyWindowHandle {
        cx.open_window(
            WindowOptions {
                titlebar: None,
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: Size::new(px(panel.width), px(panel.height)),
                })),
                display_id: Some(display.id()),
                app_id: Some("dev.rmac.MenuMaterial".to_owned()),
                window_background: WindowBackgroundAppearance::Blurred,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: format!("rmac-menu-material-{output}-{index}"),
                    layer: Layer::Top,
                    anchor: Anchor::TOP | Anchor::LEFT,
                    margin: Some((px(panel.top), px(0.0), px(0.0), px(panel.left))),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    exclusive_zone: Some(px(0.0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MenuBackdrop),
        )
        .expect("open menu material layer surface")
        .into()
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
            backdrop_tx: &async_channel::Sender<MenuBackdropUpdate>,
            cx: &mut App,
        ) {
            let available = rmac_shell_layer::output_surfaces::newest_displays(cx);
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
                let _ = backdrop_tx.try_send((uuid, None));
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
                    let handle = open_top_bar(
                        display.clone(),
                        uuid,
                        status.clone(),
                        fullscreen,
                        backdrop_tx.clone(),
                        cx,
                    );
                    if let Some((_, previous)) = self.windows.insert(uuid, (fullscreen, handle)) {
                        let _ = previous.update(cx, |_, window, _| window.remove_window());
                    }
                }
            }
        }
    }

    fn open_top_bar(
        display: Rc<dyn PlatformDisplay>,
        output_uuid: Uuid,
        status: Entity<ShellStatus>,
        fullscreen: bool,
        backdrop_tx: async_channel::Sender<MenuBackdropUpdate>,
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
                    // This interaction layer includes the menu drop-down
                    // region. Bounded companion surfaces request blur for the
                    // visible panels without softening the whole desktop.
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: format!("rmac-top-bar-{}", u64::from(display_id)),
                        layer: Layer::Overlay,
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
                        cx.new(|cx| {
                            TopBar::new(
                                display_id,
                                output_uuid,
                                status,
                                fullscreen,
                                backdrop_tx,
                                window,
                                cx,
                            )
                        })
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
        let app = application()
            .with_assets(MenuBarAssets)
            .with_quit_mode(QuitMode::Explicit);
        app.run(|cx: &mut App| {
            rmac_shell_ui::tokens::install_appearance_watch(cx);
            let status = start_status(cx);
            let (backdrop_tx, backdrop_rx) = async_channel::bounded(16);
            cx.spawn(async move |cx| {
                let mut tracker = MenuBackdropTracker::default();
                while let Ok((output, desired)) = backdrop_rx.recv().await {
                    cx.update(|cx| tracker.update(output, desired, cx));
                }
            })
            .detach();
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_shell_layer::output_surfaces::watch_top_bar(output_tx).await
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
                            tracker.reconcile(Some(&desired), &status, &backdrop_tx, cx);
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
                        cx.update(|cx| tracker.reconcile(None, &status, &backdrop_tx, cx));
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
        if rmac_shell_layer::output_reappeared(&previous, &current, removed) {
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
    eprintln!("top-bar requires Linux and: cargo run --features wayland --bin top-bar");
    std::process::exit(2);
}
