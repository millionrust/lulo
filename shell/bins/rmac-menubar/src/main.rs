#[cfg_attr(not(all(target_os = "linux", feature = "wayland")), allow(dead_code))]
mod menu_model;

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use chrono::Local;
    use futures_util::FutureExt as _;
    use gpui::{
        canvas, div, layer_shell::*, point, prelude::*, px, rgba, svg, AnyElement, AnyWindowHandle,
        App, AssetSource, Bounds, BoxShadow, ClickEvent, Context, DisplayId, Entity, FocusHandle,
        FontWeight, KeyDownEvent, ModifiersChangedEvent, PlatformDisplay, QuitMode, Role,
        SharedString, Size, Subscription, Window, WindowBackgroundAppearance, WindowBounds,
        WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_shell_ui::tokens;
    use rmac_shell_ui::{
        app_display_name, delay_until_next_clock_tick, top_bar_active_app_name,
        top_bar_clock_parts, top_bar_indicator_labels, top_bar_workspace_label,
        TopBarIndicatorKind,
    };
    use uuid::Uuid;

    use crate::menu_model::{
        self, app_menu_height, app_menu_item_top, app_menu_width, battery_menu_rows,
        menu_item_icon, next_status_selection, quit_all_interrupted_copy, quit_all_progress,
        split_shortcut, status_menu_height, status_menu_left, wifi_menu_rows, BadgeGlyph,
        IconColumn, LowBatteryWatch, QuitAllProgress, StatusAction, StatusMenuKind, StatusRow,
        WifiMenuInput, QUIT_ALL_CHECK,
    };

    // Measured from the reference Mac 2026-09-18 (FEEL_SPEC.md §C.2): the bar
    // occupies rows 0–28 and is fully transparent.
    const BAR_HEIGHT: f32 = 29.0;
    /// Tall enough for the longest status menu (Option-click Wi-Fi with
    /// Other Networks expanded); input regions keep the rest click-through.
    const MENU_SURFACE_HEIGHT: f32 = 680.0;
    /// Panel edge width; layout offsets measured from the outer edge
    /// subtract it because GPUI lays children out inside the border.
    const EDGE: f32 = 1.0;
    /// Scans finish a moment after the menu opens; list them when they land.
    const WIFI_RESCAN_DELAY: Duration = Duration::from_millis(2500);
    /// Width of the logout/restart confirmation shown in place of a menu.
    const CONFIRMATION_MENU_WIDTH: f32 = 248.0;
    // Menu bar geometry measured on macOS 26 (design-lab/menubar.html).
    const BAR_LEAD: f32 = 10.0;
    const BAR_TRAIL: f32 = 7.0;
    const LOGO_SLOT: f32 = 34.0;
    /// The Lulo OS mark (docs/brand.md) is a 14 × 14 disc: the height of
    /// the Mac's own menu glyph (14.5, measured), one point over the 13 pt
    /// status icons.
    const LOGO_GLYPH: f32 = 14.0;
    const TITLE_PAD: f32 = 11.0;
    const STATUS_PAD: f32 = 10.0;
    const CLOCK_DATE_TIME_GAP: f32 = 7.0;
    const SLOT_HEIGHT: f32 = 22.0;
    /// macOS opens a menu with nothing highlighted until hover or arrow keys.
    const NO_ITEM: usize = usize::MAX;
    const MENU_TEXT_SIZE: f32 = 13.0;
    const RECENT_MENU_WIDTH: f32 = 286.0;
    /// "Documents" heading row of the Recent Items submenu.
    const RECENT_HEADER_HEIGHT: f32 = 22.0;
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

    /// Menu glyphs (shell/assets/menu), original drawings sized to the
    /// macOS 26 menu symbols.
    macro_rules! menu_icons {
        ($($name:literal),* $(,)?) => {
            const MENU_ICON_NAMES: &[&str] = &[$(concat!("menu/", $name, ".svg")),*];

            fn menu_icon_bytes(path: &str) -> Option<&'static [u8]> {
                $(
                    if path == concat!("menu/", $name, ".svg") {
                        return Some(include_bytes!(concat!(
                            "../../../assets/menu/",
                            $name,
                            ".svg"
                        )));
                    }
                )*
                None
            }
        };
    }

    menu_icons!(
        "battery-low",
        "checkmark",
        "chevron-down",
        "chevron-right",
        "clipboard",
        "clock",
        "close",
        "copy",
        "cut",
        "duplicate",
        "eject",
        "eye",
        "force-quit",
        "full-screen",
        "gear",
        "help-book",
        "hide-others",
        "hide",
        "info",
        "laptop",
        "lock-fill",
        "lock",
        "minimize",
        "new-folder",
        "new-tab",
        "new-window",
        "open",
        "paste",
        "person",
        "power",
        "print",
        "redo",
        "rename",
        "restart",
        "search",
        "select-all",
        "services",
        "settings",
        "share",
        "show-all",
        "sidebar",
        "sleep",
        "star",
        "store",
        "trash",
        "undo",
        "warning",
        "wifi-1",
        "wifi-2",
        "wifi-3",
        "zoom",
    );

    fn menu_icon_path(name: &str) -> SharedString {
        SharedString::from(format!("menu/{name}.svg"))
    }

    struct MenuBarAssets;

    impl AssetSource for MenuBarAssets {
        fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            if let Some(bytes) = menu_icon_bytes(path) {
                return Ok(Some(Cow::Borrowed(bytes)));
            }
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
                .chain(MENU_ICON_NAMES.iter())
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
        low_battery: LowBatteryWatch,
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
                            // One process serves every display, so each
                            // low-battery level is announced exactly once.
                            if let Some(battery) = this.update.snapshot.status.battery {
                                if let Some(level) = this
                                    .low_battery
                                    .observe(battery.percentage, battery.on_battery)
                                {
                                    let (summary, body) =
                                        menu_model::low_battery_copy(level, battery.percentage);
                                    post_system_notice(summary, body, cx);
                                }
                            }
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
            watch_menu_owners(cx);
            Self {
                update: rmac_shell_runtime::Update::default(),
                menu_app_id: None,
                menus: Vec::new(),
                menu_generation: 0,
                low_battery: LowBatteryWatch::default(),
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
            let menus = match result {
                Ok(menus) => menus,
                // Not running yet, or still starting up: the menu owner
                // watcher fetches again once the app publishes its menu.
                Err(rmac_app_menu::Error::NotPublished) => return,
                Err(error) => {
                    eprintln!("could not read {app_id} menus: {error}");
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.menu_generation == generation
                    && this.menu_app_id.as_deref() == Some(app_id.as_str())
                {
                    this.menus = menus;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Follow first-party menu endpoints on the bus: fetch the active app's
    /// menus as soon as it publishes them, however long it takes to start,
    /// and drop them when it exits so the bar never offers a dead app's
    /// commands.
    fn watch_menu_owners(cx: &mut Context<ShellStatus>) {
        cx.spawn(async move |this, cx| {
            let mut owners = match rmac_app_menu::watch_menu_owners().await {
                Ok(owners) => owners,
                Err(error) => {
                    eprintln!("could not follow application menus: {error}");
                    return;
                }
            };
            while let Some((app_id, published)) = owners.next().await {
                let alive = this.update(cx, |this, cx| {
                    if this.menu_app_id.as_deref() != Some(app_id) {
                        return;
                    }
                    if published {
                        if this.menus.is_empty() {
                            request_app_menus(app_id.to_owned(), this.menu_generation, cx);
                        }
                        return;
                    }
                    this.menus.clear();
                    this.menu_generation = this.menu_generation.saturating_add(1);
                    // With no window focused, the app that just quit no
                    // longer names the bar either.
                    if this.update.snapshot.status.focused.app_id.is_none() {
                        this.menu_app_id = None;
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            eprintln!("stopped following application menus: the session bus closed");
        })
        .detach();
    }

    #[derive(Clone, Debug, PartialEq)]
    struct MenuBackdropPanel {
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        radius: f32,
        tint: u32,
    }

    /// The live data a status menu is drawn from, loaded when it opens.
    struct WifiMenuData {
        wifi: rmac_network::WifiSnapshot,
        device: Option<rmac_network::NetworkDevice>,
    }

    /// Measured macOS 26 dark menu colours (design-lab/menus.html); light
    /// appearance keeps the design tokens.
    struct MenuPalette {
        text: u32,
        status_text: u32,
        secondary: u32,
        value: u32,
        disabled: u32,
        separator: u32,
        status_separator: u32,
        edge: u32,
        status_edge: u32,
        hairline: u32,
        badge: u32,
        badge_on: u32,
        switch_off: u32,
        knob: u32,
        status_hover: u32,
        selected_text: u32,
        selected_shortcut: u32,
        app_tint: u32,
        status_tint: u32,
    }

    fn menu_palette() -> MenuPalette {
        let accent = tokens::accent();
        let app_tint = tokens::regular_dark_tint();
        // Dark text is light: the primary label's red channel says which
        // appearance is live.
        if tokens::primary_text() >> 24 > 0x80 {
            MenuPalette {
                text: 0xFFFFFFD9,
                status_text: 0xFFFFFFE6,
                secondary: 0xFFFFFFAB,
                value: 0xFFFFFFB3,
                disabled: 0xFFFFFF40,
                separator: 0xFFFFFF24,
                status_separator: 0xFFFFFF17,
                edge: 0xFFFFFF4D,
                status_edge: 0xFFFFFF24,
                hairline: 0x000000D9,
                badge: 0xFFFFFF1A,
                badge_on: accent,
                switch_off: 0xFFFFFF24,
                knob: 0xE1EBFEFF,
                status_hover: 0xFFFFFF1A,
                selected_text: 0xFFFFFFFF,
                selected_shortcut: 0xFFFFFFB3,
                app_tint,
                // The status menus measure ≈ 20% darker than app menus.
                status_tint: darken(app_tint, 0.8),
            }
        } else {
            MenuPalette {
                text: tokens::primary_text(),
                status_text: tokens::primary_text(),
                secondary: tokens::secondary_text(),
                value: tokens::secondary_text(),
                disabled: tokens::disabled_text(),
                separator: tokens::separator(),
                status_separator: tokens::separator(),
                edge: 0x0000001A,
                status_edge: 0x0000001A,
                hairline: 0x00000026,
                badge: 0x0000000F,
                badge_on: accent,
                switch_off: 0x00000017,
                knob: 0xFFFFFFFF,
                status_hover: 0x0000000F,
                selected_text: 0xFFFFFFFF,
                selected_shortcut: 0xFFFFFFB3,
                app_tint,
                status_tint: app_tint,
            }
        }
    }

    fn darken(rgba_hex: u32, factor: f32) -> u32 {
        let channel = |shift: u32| {
            let value = ((rgba_hex >> shift) & 0xFF) as f32 * factor;
            (value.round() as u32).min(0xFF) << shift
        };
        channel(24) | channel(16) | channel(8) | (rgba_hex & 0xFF)
    }

    fn menu_shadows(hairline: u32) -> Vec<BoxShadow> {
        vec![
            BoxShadow {
                color: rgba(hairline).into(),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(0.5),
                inset: false,
            },
            BoxShadow {
                color: rgba(0x00000059).into(),
                offset: point(px(0.0), px(10.0)),
                blur_radius: px(32.0),
                spread_radius: px(0.0),
                inset: false,
            },
        ]
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
        /// The Wi-Fi or Battery menu, open under its status item.
        status_menu: Option<StatusMenuKind>,
        status_selected: Option<usize>,
        /// Option-click: the Wi-Fi menu shows interface and connection details.
        status_option: bool,
        status_generation: u64,
        wifi_menu: Option<WifiMenuData>,
        wifi_others_expanded: bool,
        /// The network a `Join` is waiting on a D-Bus reply for.
        wifi_joining: Option<rmac_network::WifiNetworkId>,
        /// The last Wi-Fi mutation failure, visible until dismissed or a
        /// fresh mutation starts.
        wifi_error: Option<String>,
        battery_menu: Option<rmac_power::Snapshot>,
        /// The last energy-mode mutation failure, visible until dismissed.
        battery_error: Option<String>,
        /// Each status item's highlight edges (left, right), recorded while
        /// painting, so its menu opens exactly under it.
        status_slots: Rc<RefCell<BTreeMap<StatusMenuKind, (f32, f32)>>>,
        fullscreen: bool,
        revealed: bool,
        pointer_inside: bool,
        hide_generation: u64,
        parking: rmac_compositor::ParkingStore,
        backdrop_panels: Option<Vec<MenuBackdropPanel>>,
        backdrop_tx: async_channel::Sender<MenuBackdropUpdate>,
        focus: FocusHandle,
        /// Evidence capture: `RMAC_CAPTURE_MENU=<index>` opens that menu on
        /// the first frame so screenshots can compare it with macOS.
        capture_menu: Option<usize>,
        /// `RMAC_CAPTURE_STATUS_MENU=wifi|wifi-option|battery` does the
        /// same for a status menu.
        capture_status: Option<(StatusMenuKind, bool)>,
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
                selected_item: NO_ITEM,
                open_app_id: None,
                recent_items: Vec::new(),
                recent_items_loading: false,
                recent_items_unavailable: false,
                recent_generation: 0,
                recent_submenu_open: false,
                recent_selected_item: 0,
                pending_system_action: None,
                status_menu: None,
                status_selected: None,
                status_option: false,
                status_generation: 0,
                wifi_menu: None,
                wifi_others_expanded: false,
                wifi_joining: None,
                wifi_error: None,
                battery_menu: None,
                battery_error: None,
                status_slots: Rc::new(RefCell::new(BTreeMap::new())),
                fullscreen,
                revealed: !fullscreen,
                pointer_inside: false,
                hide_generation: 0,
                parking: rmac_compositor::ParkingStore::load_default(),
                backdrop_panels: None,
                backdrop_tx,
                capture_menu: std::env::var("RMAC_CAPTURE_MENU")
                    .ok()
                    .and_then(|value| value.parse().ok()),
                capture_status: std::env::var("RMAC_CAPTURE_STATUS_MENU")
                    .ok()
                    .and_then(|value| menu_model::parse_capture_status(&value)),
                focus,
                _blur: blur,
            }
        }

        fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.status_menu.take().is_some() {
                self.status_selected = None;
                self.status_option = false;
                self.status_generation = self.status_generation.saturating_add(1);
                window.refresh();
                cx.notify();
            }
            if self.open_menu.take().is_some() {
                self.open_app_id = None;
                self.recent_submenu_open = false;
                self.recent_selected_item = 0;
                self.pending_system_action = None;
                self.selected_item = NO_ITEM;
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
            self.status_menu = None;
            self.status_selected = None;
            self.open_menu = Some(index);
            self.hide_generation = self.hide_generation.saturating_add(1);
            self.revealed = true;
            self.selected_item = NO_ITEM;
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

        fn open_status_menu(
            &mut self,
            kind: StatusMenuKind,
            option: bool,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.open_menu = None;
            self.open_app_id = None;
            self.selected_item = NO_ITEM;
            self.recent_submenu_open = false;
            self.pending_system_action = None;
            self.status_menu = Some(kind);
            self.status_selected = None;
            self.status_option = option;
            self.wifi_others_expanded = false;
            self.hide_generation = self.hide_generation.saturating_add(1);
            self.revealed = true;
            self.load_status_menu(kind, true, cx);
            window.focus(&self.focus, cx);
            window.refresh();
            cx.notify();
        }

        /// Reads the menu's live data off the UI thread. Opening the Wi-Fi
        /// menu also asks for a scan and reloads once it has had time to
        /// finish, as macOS lists fresh networks while the menu is open.
        fn load_status_menu(&mut self, kind: StatusMenuKind, rescan: bool, cx: &mut Context<Self>) {
            self.status_generation = self.status_generation.saturating_add(1);
            let generation = self.status_generation;
            match kind {
                StatusMenuKind::Wifi => {
                    cx.spawn(async move |this, cx| {
                        let data = cx
                            .background_executor()
                            .spawn(async move {
                                blocking::unblock(move || load_wifi_menu(rescan)).await
                            })
                            .await;
                        let loaded = this
                            .update(cx, |this, cx| {
                                if this.status_generation != generation
                                    || this.status_menu != Some(StatusMenuKind::Wifi)
                                {
                                    return false;
                                }
                                this.wifi_menu = data;
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);
                        if loaded && rescan {
                            cx.background_executor().timer(WIFI_RESCAN_DELAY).await;
                            let _ = this.update(cx, |this, cx| {
                                if this.status_generation == generation
                                    && this.status_menu == Some(StatusMenuKind::Wifi)
                                {
                                    this.load_status_menu(StatusMenuKind::Wifi, false, cx);
                                }
                            });
                        }
                    })
                    .detach();
                }
                StatusMenuKind::Battery => {
                    cx.spawn(async move |this, cx| {
                        let snapshot = cx
                            .background_executor()
                            .spawn(async move {
                                blocking::unblock(move || rmac_power::snapshot().ok()).await
                            })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if this.status_generation == generation
                                && this.status_menu == Some(StatusMenuKind::Battery)
                            {
                                this.battery_menu = snapshot;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
        }

        fn status_rows(&self, kind: StatusMenuKind) -> Vec<StatusRow> {
            match kind {
                StatusMenuKind::Wifi => wifi_menu_rows(WifiMenuInput {
                    wifi: self.wifi_menu.as_ref().map(|data| &data.wifi),
                    device: self
                        .wifi_menu
                        .as_ref()
                        .and_then(|data| data.device.as_ref()),
                    option: self.status_option,
                    others_expanded: self.wifi_others_expanded,
                    joining: self.wifi_joining.as_ref(),
                    error: self.wifi_error.as_deref(),
                }),
                StatusMenuKind::Battery => {
                    battery_menu_rows(self.battery_menu.as_ref(), self.battery_error.as_deref())
                }
            }
        }

        fn run_status_action(
            &mut self,
            action: StatusAction,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if action.closes_menu() {
                self.close_menu(window, cx);
            }
            match action {
                StatusAction::ToggleWifi => {
                    let Some(data) = self.wifi_menu.as_mut() else {
                        return;
                    };
                    let enabled = !data.wifi.enabled;
                    // Flip the switch now; the reload confirms or reverts it.
                    data.wifi.enabled = enabled;
                    self.wifi_error = None;
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                blocking::unblock(move || rmac_network::set_enabled(enabled)).await
                            })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if let Err(error) = result {
                                eprintln!(
                                    "could not turn Wi-Fi {}: {error}",
                                    if enabled { "on" } else { "off" }
                                );
                                this.wifi_error = Some(format!(
                                    "Couldn't turn Wi-Fi {}: {error}",
                                    if enabled { "on" } else { "off" }
                                ));
                            }
                            if this.status_menu == Some(StatusMenuKind::Wifi) {
                                this.load_status_menu(StatusMenuKind::Wifi, enabled, cx);
                            } else {
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
                StatusAction::Join(network) => {
                    // The row's action is already gone once `wifi_joining` is
                    // set (see `network_row`), so this cannot double-fire.
                    let ssid = self.wifi_menu.as_ref().and_then(|data| {
                        data.wifi
                            .networks
                            .iter()
                            .find(|candidate| candidate.id == network)
                            .map(|candidate| candidate.ssid.clone())
                    });
                    self.wifi_joining = Some(network.clone());
                    self.wifi_error = None;
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                blocking::unblock(move || rmac_network::connect(&network)).await
                            })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            this.wifi_joining = None;
                            if let Err(error) = result {
                                eprintln!("could not join the Wi-Fi network: {error}");
                                this.wifi_error = Some(match &ssid {
                                    Some(ssid) => format!("Couldn't join “{ssid}”: {error}"),
                                    None => format!("Couldn't join the network: {error}"),
                                });
                            }
                            if this.status_menu == Some(StatusMenuKind::Wifi) {
                                this.load_status_menu(StatusMenuKind::Wifi, false, cx);
                            } else {
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
                StatusAction::ToggleOtherNetworks => {
                    self.wifi_others_expanded = !self.wifi_others_expanded;
                    cx.notify();
                }
                StatusAction::OpenSettings(pane) => open_settings_pane(pane, cx),
                StatusAction::ToggleLowPower => {
                    let Some(profiles) = self
                        .battery_menu
                        .as_mut()
                        .map(|snapshot| &mut snapshot.profiles)
                    else {
                        return;
                    };
                    let target = if profiles.active == Some(rmac_power::PowerProfile::PowerSaver) {
                        rmac_power::PowerProfile::Balanced
                    } else {
                        rmac_power::PowerProfile::PowerSaver
                    };
                    profiles.active = Some(target);
                    self.battery_error = None;
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                blocking::unblock(move || rmac_power::set_profile(target)).await
                            })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if let Err(error) = result {
                                eprintln!("could not change the energy mode: {error}");
                                this.battery_error =
                                    Some(format!("Couldn't change the energy mode: {error}"));
                            }
                            if this.status_menu == Some(StatusMenuKind::Battery) {
                                this.load_status_menu(StatusMenuKind::Battery, false, cx);
                            } else {
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
                StatusAction::DismissWifiError => {
                    self.wifi_error = None;
                    cx.notify();
                }
                StatusAction::DismissBatteryError => {
                    self.battery_error = None;
                    cx.notify();
                }
            }
        }

        fn handle_status_key(
            &mut self,
            kind: StatusMenuKind,
            event: &KeyDownEvent,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let rows = self.status_rows(kind);
            match event.keystroke.key.as_str() {
                "escape" => self.close_menu(window, cx),
                "down" | "up" => {
                    self.status_selected = next_status_selection(
                        &rows,
                        self.status_selected,
                        event.keystroke.key == "down",
                    );
                    cx.notify();
                }
                "enter" | "space" => {
                    if let Some(action) = self
                        .status_selected
                        .and_then(|index| rows.get(index))
                        .and_then(StatusRow::action)
                    {
                        self.run_status_action(action, window, cx);
                    }
                }
                _ => {}
            }
        }

        /// One row of the Wi-Fi or Battery menu at the measured macOS 26
        /// geometry (menu_model and design-lab/menus.html).
        fn render_status_row(
            &self,
            index: usize,
            row: StatusRow,
            selected: bool,
            palette: &MenuPalette,
            cx: &Context<Self>,
        ) -> AnyElement {
            let id = format!("status-row-{}-{index}", self.display_id);
            let height = row.height();
            let action = row.action();
            let hover_fill = palette.status_hover;
            // A pointer-reachable row: hover and arrow keys share the
            // selection, and choosing it runs its action.
            let interactive = |element: gpui::Stateful<gpui::Div>| {
                let action = action.clone();
                element
                    .h(px(height))
                    .mx(px(menu_model::ROW_INSET - EDGE))
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(px(tokens::menu_item_radius()))
                    .when(selected, |style| style.bg(rgba(hover_fill)))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.status_selected != Some(index) {
                            this.status_selected = Some(index);
                            cx.notify();
                        } else if !*hovered && this.status_selected == Some(index) {
                            this.status_selected = None;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if let Some(action) = action.clone() {
                            this.run_status_action(action, window, cx);
                        }
                    }))
            };
            // Left padding inside a highlighted row so its text sits at the
            // measured 14.5 pt column.
            let row_text_pad = menu_model::STATUS_TEXT_INSET - menu_model::ROW_INSET;
            match row {
                StatusRow::Title {
                    label,
                    value,
                    switch,
                    action,
                } => div()
                    .id(id)
                    .h(px(height))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl(px(menu_model::STATUS_TEXT_INSET - EDGE))
                    .pr(px(menu_model::STATUS_SWITCH_RIGHT - EDGE))
                    .font_weight(FontWeight::BOLD)
                    .child(div().flex_1().child(label))
                    .children(value.map(|value| {
                        div()
                            .mr(px(1.0))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgba(palette.value))
                            .child(value)
                    }))
                    .children(switch.map(|on| {
                        let knob = if on { palette.knob } else { 0xFFFFFFFF };
                        div()
                            .id(format!("status-switch-{}", self.display_id))
                            .role(Role::Switch)
                            .aria_label(if on {
                                "Turn Wi-Fi Off"
                            } else {
                                "Turn Wi-Fi On"
                            })
                            .relative()
                            .flex_none()
                            .w(px(menu_model::SWITCH_WIDTH))
                            .h(px(menu_model::SWITCH_HEIGHT))
                            .rounded(px(menu_model::SWITCH_HEIGHT / 2.0))
                            .bg(rgba(if on {
                                tokens::accent()
                            } else {
                                palette.switch_off
                            }))
                            .cursor_pointer()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(2.0))
                                    .when(on, |knob| knob.right(px(2.0)))
                                    .when(!on, |knob| knob.left(px(2.0)))
                                    .w(px(menu_model::SWITCH_KNOB_WIDTH))
                                    .h(px(menu_model::SWITCH_KNOB_HEIGHT))
                                    .rounded(px(menu_model::SWITCH_KNOB_HEIGHT / 2.0))
                                    .bg(rgba(knob)),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(action) = action.clone() {
                                    this.run_status_action(action, window, cx);
                                }
                            }))
                    }))
                    .into_any_element(),
                StatusRow::Info(label) => div()
                    .id(id)
                    .h(px(height))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl(px(menu_model::STATUS_TEXT_INSET - EDGE))
                    // The Mac sets this line 1.25 pt above the row's centre.
                    .pb(px(2.5))
                    .whitespace_nowrap()
                    .text_color(rgba(palette.secondary))
                    .child(label)
                    .into_any_element(),
                StatusRow::Item { label, warning, .. } => {
                    interactive(div().id(id).role(Role::MenuItem).aria_label(label.clone()))
                        .pl(px(row_text_pad))
                        .pr(px(9.4))
                        .child(div().flex_1().whitespace_nowrap().child(label))
                        .when(warning, |row| {
                            row.child(
                                svg()
                                    .flex_none()
                                    .w(px(14.0))
                                    .h(px(14.0))
                                    .path(menu_icon_path("warning"))
                                    .text_color(rgba(palette.secondary)),
                            )
                        })
                        .into_any_element()
                }
                StatusRow::Separator => div()
                    .id(id)
                    .flex_none()
                    .h(px(1.0))
                    .mx(px(menu_model::STATUS_SEPARATOR_INSET - EDGE))
                    .my(px((height - 1.0) / 2.0))
                    .bg(rgba(palette.status_separator))
                    .into_any_element(),
                StatusRow::Header(label) => div()
                    .id(id)
                    .h(px(height))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pt(px(1.0))
                    .pl(px(menu_model::STATUS_TEXT_INSET - EDGE))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgba(palette.secondary))
                    .child(label)
                    .into_any_element(),
                StatusRow::Disclosure { label, expanded } => {
                    interactive(div().id(id).role(Role::MenuItem).aria_label(label.clone()))
                        .pl(px(row_text_pad))
                        .pr(px(5.35))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(palette.secondary))
                        .child(div().flex_1().child(label))
                        .child(
                            svg()
                                .flex_none()
                                .w(px(menu_model::MENU_ICON_BOX))
                                .h(px(menu_model::MENU_ICON_BOX))
                                .path(menu_icon_path(if expanded {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                }))
                                .text_color(rgba(palette.secondary)),
                        )
                        .into_any_element()
                }
                StatusRow::Badge {
                    label,
                    glyph,
                    on,
                    locked,
                    action,
                } => {
                    let glyph_size = if glyph == BadgeGlyph::LowPower {
                        20.0
                    } else {
                        16.0
                    };
                    let badge = div()
                        .flex_none()
                        .w(px(menu_model::STATUS_BADGE))
                        .h(px(menu_model::STATUS_BADGE))
                        .mr(px(menu_model::STATUS_BADGE_TEXT
                            - menu_model::STATUS_TEXT_INSET
                            - menu_model::STATUS_BADGE))
                        .rounded(px(menu_model::STATUS_BADGE / 2.0))
                        .bg(rgba(if on { palette.badge_on } else { palette.badge }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .w(px(glyph_size))
                                .h(px(glyph_size))
                                .path(menu_icon_path(glyph.icon()))
                                .text_color(rgba(0xFFFFFFFF)),
                        );
                    let base = div().id(id).role(Role::MenuItem).aria_label(label.clone());
                    let row = if action.is_some() {
                        interactive(base)
                    } else {
                        base.h(px(height))
                            .mx(px(menu_model::ROW_INSET - EDGE))
                            .flex()
                            .flex_none()
                            .items_center()
                    };
                    row.pl(px(row_text_pad))
                        .pr(px(7.5))
                        .child(badge)
                        .child(div().flex_1().whitespace_nowrap().child(label))
                        .when(locked, |row| {
                            row.child(
                                svg()
                                    .flex_none()
                                    .w(px(menu_model::MENU_ICON_BOX))
                                    .h(px(menu_model::MENU_ICON_BOX))
                                    .path(menu_icon_path("lock-fill"))
                                    .text_color(rgba(palette.secondary)),
                            )
                        })
                        .into_any_element()
                }
                StatusRow::Detail(label) => div()
                    .id(id)
                    .h(px(height))
                    .flex()
                    .flex_none()
                    .items_center()
                    .pl(px(menu_model::STATUS_BADGE_TEXT - EDGE))
                    .text_size(px(menu_model::STATUS_DETAIL_SIZE))
                    .text_color(rgba(palette.secondary))
                    .whitespace_nowrap()
                    .child(label)
                    .into_any_element(),
                StatusRow::GroupEnd => div().id(id).flex_none().h(px(height)).into_any_element(),
            }
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
                        && this.status_menu.is_none()
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
            } else if self.open_menu.is_none() && self.status_menu.is_none() {
                self.schedule_fullscreen_hide(cx);
            }
        }

        fn handle_key(
            &mut self,
            event: &KeyDownEvent,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if let Some(kind) = self.status_menu {
                self.handle_status_key(kind, event, window, cx);
                return;
            }
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
                    self.selected_item = if self.selected_item == NO_ITEM {
                        0
                    } else {
                        (self.selected_item + 1) % menu.items.len()
                    };
                    self.recent_submenu_open = false;
                    cx.notify();
                }
                "up" => {
                    self.selected_item = match self.selected_item {
                        NO_ITEM | 0 => menu.items.len() - 1,
                        index => index - 1,
                    };
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
                    self.selected_item = NO_ITEM;
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
            if let Some(index) = self.capture_menu.take() {
                let app_id = if index == 0 {
                    SYSTEM_MENU_ID.to_owned()
                } else {
                    self.status.read(cx).menu_app_id.clone().unwrap_or_default()
                };
                self.open_menu(index, app_id, window, cx);
            }
            if let Some((kind, option)) = self.capture_status.take() {
                self.open_status_menu(kind, option, window, cx);
            }
            record_render_count(window, self.display_id, self.render_count);
            let now = Local::now();
            let status = self.status.read(cx);
            let snapshot = &status.update.snapshot.status;
            // macOS leaves a wider gap between the date and the time.
            let (date_pattern, time_pattern) = top_bar_clock_parts(&snapshot.clock);
            let clock_date = date_pattern.map(|pattern| now.format(pattern).to_string());
            let clock = now.format(time_pattern).to_string();
            let clock_label = match &clock_date {
                Some(date) => format!("{date} {clock}"),
                None => clock.clone(),
            };
            // Opening a popup moves keyboard focus to this layer surface, so the
            // live focused window drops to None. Use the last app the status
            // runtime reported (kept across those blips) for the app menu, and
            // name it rather than letting the desktop identity take over.
            // The same holds while the bar still shows the menus of an app
            // that has no focused window: name that app, not the desktop, so
            // the name, its app menu and the menus beside it agree.
            // Close a menu whose app is gone first, so this frame already
            // names the app that owns the bar now. The system and app menus
            // always sit before the exported ones.
            if self.open_menu.is_some()
                && (self.open_menu >= Some(status.menus.len() + 2)
                    || (self.open_menu != Some(0)
                        && status
                            .menu_app_id
                            .as_deref()
                            .is_some_and(|id| self.open_app_id.as_deref() != Some(id))))
            {
                self.open_menu = None;
                self.open_app_id = None;
                self.selected_item = NO_ITEM;
            }
            let keep_menu_app = self.open_menu.is_some()
                || (snapshot.focused.app_id.is_none() && !status.menus.is_empty());
            let active_app_id = if keep_menu_app {
                status
                    .menu_app_id
                    .clone()
                    .or_else(|| snapshot.focused.app_id.clone())
            } else {
                snapshot.focused.app_id.clone()
            };
            let active_app = if keep_menu_app {
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

            let visible = !self.fullscreen
                || self.revealed
                || self.open_menu.is_some()
                || self.status_menu.is_some();
            let palette = menu_palette();
            let menu_top = BAR_HEIGHT + menu_model::MENU_TOP_GAP;
            // Menus are as wide as their widest item, as on macOS, and stay
            // on screen near the right edge.
            let menu_width = if self.pending_system_action.is_some() {
                self.open_menu.map(|_| CONFIRMATION_MENU_WIDTH)
            } else {
                self.open_menu
                    .and_then(|index| menus.get(index))
                    .map(|menu| menu_panel_width(menu, window))
            };
            let screen_width = f32::from(window.bounds().size.width);
            let menu_left = self
                .open_menu
                .map(|index| menu_anchor_x(&active_app, &menus, index, window))
                .zip(menu_width)
                .map(|(left, width)| left.min(screen_width - width - 4.0).max(4.0));
            let menu_height = if self.pending_system_action.is_some() {
                Some(150.0)
            } else {
                self.open_menu
                    .and_then(|index| menus.get(index))
                    .map(|menu| app_menu_height(&menu.items))
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
                    .unwrap_or(menu_top)
            });
            let recent_height = recent_menu_height(
                self.recent_items.len(),
                self.recent_items_loading,
                self.recent_items_unavailable,
            );
            // The Wi-Fi or Battery menu opens under its own status item.
            let status_panel = self.status_menu.map(|kind| {
                let rows = self.status_rows(kind);
                let width = menu_model::STATUS_MENU_WIDTH;
                let left = self
                    .status_slots
                    .borrow()
                    .get(&kind)
                    .map(|&(slot_left, slot_right)| {
                        status_menu_left(slot_left, slot_right, width, screen_width)
                    })
                    .unwrap_or(screen_width - width - BAR_TRAIL);
                let height = status_menu_height(&rows).min(MENU_SURFACE_HEIGHT - menu_top - 8.0);
                (kind, rows, left, width, height)
            });
            let backdrop_panels = if visible {
                let mut panels = Vec::new();
                if let (Some(left), Some(width), Some(height)) =
                    (menu_left, menu_width, menu_height)
                {
                    panels.push(MenuBackdropPanel {
                        left,
                        top: menu_top,
                        width,
                        height,
                        radius: menu_model::APP_MENU_RADIUS,
                        tint: palette.app_tint,
                    });
                    if let Some(top) = recent_submenu_top {
                        panels.push(MenuBackdropPanel {
                            left: left + width - 4.0,
                            top,
                            width: RECENT_MENU_WIDTH,
                            height: recent_height,
                            radius: menu_model::APP_MENU_RADIUS,
                            tint: palette.app_tint,
                        });
                    }
                }
                if let Some((_, _, left, width, height)) = &status_panel {
                    panels.push(MenuBackdropPanel {
                        left: *left,
                        top: menu_top,
                        width: *width,
                        height: *height,
                        radius: menu_model::STATUS_MENU_RADIUS,
                        tint: palette.status_tint,
                    });
                }
                (!panels.is_empty()).then_some(panels)
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
                if let (Some(left), Some(width), Some(height)) =
                    (menu_left, menu_width, menu_height)
                {
                    input_regions.push(Bounds {
                        origin: point(px(left), px(BAR_HEIGHT)),
                        size: Size::new(px(width), px(height + menu_top - BAR_HEIGHT)),
                    });
                    if let Some(top) = recent_submenu_top {
                        input_regions.push(Bounds {
                            origin: point(px(left + width - 4.0), px(top)),
                            size: Size::new(px(RECENT_MENU_WIDTH), px(recent_height)),
                        });
                    }
                }
                if let Some((_, _, left, width, height)) = &status_panel {
                    input_regions.push(Bounds {
                        origin: point(px(*left), px(BAR_HEIGHT)),
                        size: Size::new(px(*width), px(*height + menu_top - BAR_HEIGHT)),
                    });
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
                        .h(px(SLOT_HEIGHT))
                        .px(px(TITLE_PAD))
                        .font_weight(FontWeight::MEDIUM)
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
                let width = menu_width?;
                let selected = self.selected_item;
                let mut panel = div()
                    .id(format!("app-menu-panel-{}-{menu_index}", self.display_id))
                    .role(Role::Menu)
                    .aria_label(format!("{} menu", menu.label))
                    .absolute()
                    .top(px(menu_top))
                    .left(px(left))
                    .w(px(width))
                    .pt(px(menu_model::APP_MENU_PADDING - EDGE))
                    .pb(px(menu_model::APP_MENU_PADDING - EDGE))
                    .rounded(px(menu_model::APP_MENU_RADIUS))
                    .bg(rgba(tokens::transparent()))
                    .text_size(px(MENU_TEXT_SIZE))
                    .text_color(rgba(palette.text))
                    .border(px(EDGE))
                    .border_color(rgba(palette.edge))
                    .shadow(menu_shadows(palette.hairline))
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
                let (icons, column) = menu_icon_column(&menu);
                for (item_index, item) in menu.items.into_iter().enumerate() {
                    if item.separator_before {
                        panel = panel.child(
                            div()
                                .h(px(1.0))
                                .mx(px(menu_model::APP_SEPARATOR_INSET - EDGE))
                                .my(px((menu_model::APP_SEPARATOR_HEIGHT - 1.0) / 2.0))
                                .bg(rgba(palette.separator)),
                        );
                    }
                    let action = item.action.clone();
                    let item_app_id = app_id.clone();
                    let enabled = item.enabled;
                    let opens_recents =
                        item_app_id == SYSTEM_MENU_ID && action == "system::recents";
                    let highlighted = enabled && selected == item_index;
                    let foreground = if !enabled {
                        palette.disabled
                    } else if highlighted {
                        palette.selected_text
                    } else {
                        palette.text
                    };
                    let shortcut_color = if highlighted {
                        palette.selected_shortcut
                    } else {
                        palette.disabled
                    };
                    let mut row = div()
                        .id(format!(
                            "app-menu-item-{}-{menu_index}-{item_index}",
                            self.display_id
                        ))
                        .role(Role::MenuItem)
                        .aria_label(item.label.clone())
                        .relative()
                        .h(px(menu_model::APP_ROW_HEIGHT))
                        .mx(px(menu_model::ROW_INSET - EDGE))
                        .pl(px(column.text_x() - menu_model::ROW_INSET))
                        .pr(px(menu_model::KEY_RIGHT - menu_model::ROW_INSET))
                        .flex()
                        .items_center()
                        .rounded(px(tokens::menu_item_radius()))
                        .text_color(rgba(foreground))
                        .when(highlighted, |style| style.bg(rgba(tokens::accent())))
                        .children(icons[item_index].map(|icon| {
                            svg()
                                .absolute()
                                .left(px(column.icon_x() - menu_model::ROW_INSET))
                                .top(px(
                                    (menu_model::APP_ROW_HEIGHT - menu_model::MENU_ICON_BOX) / 2.0
                                ))
                                .w(px(menu_model::MENU_ICON_BOX))
                                .h(px(menu_model::MENU_ICON_BOX))
                                .path(menu_icon_path(icon))
                                .text_color(rgba(foreground))
                        }))
                        .child(div().flex_1().whitespace_nowrap().child(item.label));
                    if item.shortcut == menu_model::SUBMENU_MARK {
                        row = row.child(
                            svg()
                                .flex_none()
                                .ml(px(menu_model::SHORTCUT_GAP))
                                .mr(px(menu_model::CHEVRON_RIGHT
                                    - menu_model::KEY_RIGHT
                                    - (menu_model::MENU_ICON_BOX - CHEVRON_GLYPH_RIGHT)))
                                .w(px(menu_model::MENU_ICON_BOX))
                                .h(px(menu_model::MENU_ICON_BOX))
                                .path(menu_icon_path("chevron-right"))
                                .text_color(rgba(foreground)),
                        );
                    } else if !item.shortcut.is_empty() {
                        row = row.child(shortcut_keys(&item.shortcut, shortcut_color));
                    }
                    if enabled {
                        row = row
                            .cursor_pointer()
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered {
                                    let mut changed = this.selected_item != item_index;
                                    this.selected_item = item_index;
                                    if this.recent_submenu_open != opens_recents {
                                        this.recent_submenu_open = opens_recents;
                                        this.recent_selected_item = 0;
                                        changed = true;
                                    }
                                    if changed {
                                        cx.notify();
                                    }
                                } else if this.selected_item == item_index
                                    && !this.recent_submenu_open
                                {
                                    // Leaving the menu clears the highlight.
                                    this.selected_item = NO_ITEM;
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
                    let selected_text = palette.selected_text;
                    let recent_row = move |id: String, label: String, highlighted: bool| {
                        div()
                            .id(id)
                            .role(Role::MenuItem)
                            .aria_label(label)
                            .h(px(menu_model::APP_ROW_HEIGHT))
                            .mx(px(menu_model::ROW_INSET - EDGE))
                            .pl(px(menu_model::APP_TEXT_INSET - menu_model::ROW_INSET))
                            .pr(px(menu_model::APP_TEXT_INSET - menu_model::ROW_INSET))
                            .flex()
                            .items_center()
                            .rounded(px(tokens::menu_item_radius()))
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .when(highlighted, |style| {
                                style
                                    .bg(rgba(tokens::accent()))
                                    .text_color(rgba(selected_text))
                            })
                    };
                    let mut submenu = div()
                        .id(format!("recent-items-panel-{}", self.display_id))
                        .role(Role::Menu)
                        .aria_label("Recent Items")
                        .absolute()
                        .top(px(submenu_top))
                        .left(px(left + width - 4.0))
                        .w(px(RECENT_MENU_WIDTH))
                        .pt(px(menu_model::APP_MENU_PADDING - EDGE))
                        .pb(px(menu_model::APP_MENU_PADDING - EDGE))
                        .rounded(px(menu_model::APP_MENU_RADIUS))
                        .bg(rgba(tokens::transparent()))
                        .text_size(px(MENU_TEXT_SIZE))
                        .text_color(rgba(palette.text))
                        .border(px(EDGE))
                        .border_color(rgba(palette.edge))
                        .shadow(menu_shadows(palette.hairline))
                        .occlude()
                        .child(
                            div()
                                .h(px(RECENT_HEADER_HEIGHT))
                                .pl(px(menu_model::APP_TEXT_INSET - EDGE))
                                .flex()
                                .items_center()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgba(palette.secondary))
                                .child("Documents"),
                        );
                    if self.recent_items_loading {
                        submenu = submenu.child(recent_status_row("Loading…", palette.disabled));
                    } else if self.recent_items_unavailable {
                        submenu = submenu.child(recent_status_row(
                            "Recent Items Unavailable",
                            palette.disabled,
                        ));
                    } else if self.recent_items.is_empty() {
                        submenu = submenu.child(recent_status_row("None", palette.disabled));
                    } else {
                        for (index, path) in self.recent_items.iter().cloned().enumerate() {
                            let label = recent_item_label(&path);
                            submenu = submenu.child(
                                recent_row(
                                    format!("recent-item-{}-{index}", self.display_id),
                                    format!("Open {label}"),
                                    selected == index,
                                )
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
                            .child(
                                div()
                                    .h(px(1.0))
                                    .mx(px(menu_model::APP_SEPARATOR_INSET - EDGE))
                                    .my(px((menu_model::APP_SEPARATOR_HEIGHT - 1.0) / 2.0))
                                    .bg(rgba(palette.separator)),
                            )
                            .child(
                                recent_row(
                                    format!("recent-items-clear-{}", self.display_id),
                                    "Clear Recent Items".into(),
                                    selected == clear_index,
                                )
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

            let status_popup = status_panel.map(|(kind, rows, left, width, height)| {
                let mut panel = div()
                    .id(format!("status-menu-panel-{}", self.display_id))
                    .role(Role::Menu)
                    .aria_label(match kind {
                        StatusMenuKind::Wifi => "Wi-Fi",
                        StatusMenuKind::Battery => "Battery",
                    })
                    .absolute()
                    .top(px(menu_top))
                    .left(px(left))
                    .w(px(width))
                    .h(px(height))
                    .overflow_hidden()
                    .pt(px(menu_model::STATUS_PADDING_TOP - EDGE))
                    .pb(px(menu_model::STATUS_PADDING_BOTTOM - EDGE))
                    .flex()
                    .flex_col()
                    .rounded(px(menu_model::STATUS_MENU_RADIUS))
                    .bg(rgba(tokens::transparent()))
                    .text_size(px(MENU_TEXT_SIZE))
                    .text_color(rgba(palette.status_text))
                    .border(px(EDGE))
                    .border_color(rgba(palette.status_edge))
                    .shadow(menu_shadows(0))
                    .occlude()
                    // Clicks on headings and separators keep the menu open.
                    .on_click(|_, _, cx| cx.stop_propagation());
                for (index, row) in rows.into_iter().enumerate() {
                    let selected = self.status_selected == Some(index);
                    panel = panel.child(self.render_status_row(index, row, selected, &palette, cx));
                }
                panel
            });

            let bar = div()
                .id(format!("top-bar-strip-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("top bar")
                .absolute()
                .top(px(if visible { 0.0 } else { -BAR_HEIGHT }))
                .left_0()
                .right_0()
                .h(px(BAR_HEIGHT))
                .flex()
                .items_center()
                .pl(px(BAR_LEAD))
                .pr(px(BAR_TRAIL))
                .font_family("Inter")
                .text_color(rgba(tokens::menubar_text()))
                .text_size(px(tokens::body_text_size()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .child(
                            div()
                                .id(format!("desktop-mark-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("menu")
                                .w(px(LOGO_SLOT))
                                .h(px(SLOT_HEIGHT))
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
                                        .size(px(LOGO_GLYPH))
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
                                .h(px(SLOT_HEIGHT))
                                .px(px(TITLE_PAD))
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
                                        .font_weight(FontWeight::BOLD)
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
                                    let is_battery = indicator.kind == TopBarIndicatorKind::Battery;
                                    // Wi-Fi and Battery open their own menus
                                    // under the icon; the rest open Control
                                    // Center.
                                    let menu_kind = match indicator.kind {
                                        TopBarIndicatorKind::Network => Some(StatusMenuKind::Wifi),
                                        TopBarIndicatorKind::Battery => {
                                            Some(StatusMenuKind::Battery)
                                        }
                                        _ => None,
                                    };
                                    let slots = self.status_slots.clone();
                                    let mut item = div()
                                        .id(format!("status-{}-{index}", self.display_id))
                                        .role(Role::Button)
                                        .aria_label(indicator.accessible)
                                        .relative()
                                        .h(px(SLOT_HEIGHT))
                                        .flex()
                                        .items_center()
                                        .gap(px(5.0))
                                        .px(px(STATUS_PAD))
                                        .rounded(px(tokens::menu_item_radius()))
                                        .cursor_pointer()
                                        .when(
                                            menu_kind.is_some() && self.status_menu == menu_kind,
                                            |style| style.bg(rgba(tokens::light_selection())),
                                        )
                                        .hover(|style| style.bg(rgba(tokens::light_hover())))
                                        .on_click(cx.listener(
                                            move |this, event: &ClickEvent, window, cx| {
                                                cx.stop_propagation();
                                                let Some(kind) = menu_kind else {
                                                    dispatch_shortcut("quick-settings", cx);
                                                    return;
                                                };
                                                if this.status_menu == Some(kind) {
                                                    this.close_menu(window, cx);
                                                } else {
                                                    let option = event.modifiers().alt
                                                        || window.modifiers().alt;
                                                    this.open_status_menu(kind, option, window, cx);
                                                }
                                            },
                                        ))
                                        .font_weight(FontWeight::MEDIUM)
                                        .children(menu_kind.map(|kind| {
                                            // Record the highlight's edges so
                                            // the menu can open under it.
                                            canvas(
                                                move |bounds, _, _| {
                                                    slots.borrow_mut().insert(
                                                        kind,
                                                        (
                                                            f32::from(bounds.left()),
                                                            f32::from(bounds.right()),
                                                        ),
                                                    );
                                                },
                                                |_, _, _, _| {},
                                            )
                                            .absolute()
                                            .inset_0()
                                        }));
                                    // macOS puts the percentage before the glyph.
                                    if !indicator.visible.is_empty() {
                                        item = item.child(indicator.visible);
                                    }
                                    if is_battery {
                                        item.child(battery_glyph(snapshot.battery.clone()))
                                    } else {
                                        let (width, height) = status_icon_size(indicator.kind);
                                        item.child(
                                            svg()
                                                .path(icon)
                                                .w(px(width))
                                                .h(px(height))
                                                .text_color(rgba(tokens::primary_text())),
                                        )
                                    }
                                }),
                        )
                        .child(
                            div()
                                .id(format!("spotlight-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Spotlight")
                                .h(px(SLOT_HEIGHT))
                                .px(px(STATUS_PAD))
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
                                        .w(px(13.0))
                                        .h(px(13.0))
                                        .text_color(rgba(tokens::primary_text())),
                                ),
                        )
                        .child(
                            div()
                                .id(format!("control-center-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Control Center")
                                .h(px(SLOT_HEIGHT))
                                .px(px(STATUS_PAD))
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
                                        .w(px(13.0))
                                        .h(px(13.0))
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
                                .px(px(TITLE_PAD))
                                .h(px(SLOT_HEIGHT))
                                .flex()
                                .items_center()
                                .gap(px(CLOCK_DATE_TIME_GAP))
                                .rounded(px(tokens::menu_item_radius()))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(tokens::light_hover())))
                                .on_click(|_, _, cx| dispatch_shortcut("notification-center", cx))
                                .font_weight(FontWeight::MEDIUM)
                                .font_features(rmac_shell_ui::tabular_font_features())
                                .children(clock_date)
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
                .font_family("Inter")
                .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                    // Holding Option while the Wi-Fi menu is open reveals its
                    // details, as on macOS.
                    if this.status_menu.is_some() && event.modifiers.alt && !this.status_option {
                        this.status_option = true;
                        cx.notify();
                    }
                }))
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.open_menu.is_some() || this.status_menu.is_some() {
                        cx.stop_propagation();
                        this.handle_key(event, window, cx);
                    }
                }))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_menu(window, cx);
                }))
                .child(bar)
                .children(popup)
                .children(status_popup)
        }
    }

    /// The widest item's title and shortcut plus the measured columns,
    /// never narrower than the macOS minimum menu width.
    fn menu_panel_width(menu: &rmac_app_menu::Menu, window: &Window) -> f32 {
        let (_, column) = menu_icon_column(menu);
        app_menu_width(
            &menu.items,
            column,
            tokens::current().metrics.menu_min_width,
            |label| rmac_shell_ui::text_width(window, label, FontWeight::NORMAL),
        )
    }

    /// Left edge of a menu, from the same slot geometry the bar is laid out
    /// with: macOS opens it 4 pt left of the title's item frame.
    fn menu_anchor_x(
        active_app: &str,
        menus: &[rmac_app_menu::Menu],
        index: usize,
        window: &Window,
    ) -> f32 {
        if index == 0 {
            return BAR_LEAD + menu_model::LOGO_MENU_OFFSET;
        }
        let title_slot = |label: &str, weight| {
            rmac_shell_ui::text_width(window, label, weight) + 2.0 * TITLE_PAD
        };
        let app_slot = title_slot(active_app, FontWeight::BOLD);
        let preceding = menus
            .iter()
            .skip(2)
            .take(index.saturating_sub(2))
            .map(|menu| title_slot(&menu.label, FontWeight::MEDIUM))
            .sum::<f32>();
        BAR_LEAD
            + LOGO_SLOT
            + menu_model::TITLE_MENU_OFFSET
            + if index == 1 {
                0.0
            } else {
                app_slot + preceding
            }
    }

    /// The glyph each item shows and the text column they set.
    fn menu_icon_column(menu: &rmac_app_menu::Menu) -> (Vec<Option<&'static str>>, IconColumn) {
        let icons = menu
            .items
            .iter()
            .map(|item| menu_item_icon(&item.action, &item.label))
            .collect::<Vec<_>>();
        let column = IconColumn::for_icons(icons.iter().copied());
        (icons, column)
    }

    /// Where the chevron glyph ends inside its 16 pt icon box.
    const CHEVRON_GLYPH_RIGHT: f32 = 11.35;

    /// A shortcut drawn the macOS way: each modifier centred in its own
    /// cell, the key left-aligned in the last one.
    fn shortcut_keys(shortcut: &str, color: u32) -> impl IntoElement {
        let parts = split_shortcut(shortcut);
        div()
            .flex()
            .flex_none()
            .ml(px(menu_model::SHORTCUT_GAP))
            .text_color(rgba(color))
            .children(parts.modifiers.iter().map(|modifier| {
                div()
                    .w(px(menu_model::KEY_CELL))
                    .flex()
                    .justify_center()
                    .child(modifier.to_string())
            }))
            .when(!parts.key.is_empty(), |keys| {
                keys.child(
                    div()
                        .ml(px(menu_model::KEY_LETTER_GAP))
                        .min_w(px(menu_model::KEY_LETTER_WIDTH))
                        .child(parts.key.clone()),
                )
            })
    }

    fn load_wifi_menu(rescan: bool) -> Option<WifiMenuData> {
        let wifi = rmac_network::snapshot().ok()?;
        if rescan && wifi.enabled {
            // Best effort: the menu still lists what NetworkManager knows.
            let _ = rmac_network::request_scan();
        }
        let device = rmac_network::network_snapshot().ok().and_then(|network| {
            network.devices.into_iter().find(|device| {
                device.kind == rmac_network::DeviceKind::WiFi
                    && wifi
                        .interface
                        .as_deref()
                        .is_none_or(|interface| interface == device.interface)
            })
        });
        Some(WifiMenuData { wifi, device })
    }

    fn open_settings_pane(pane: &'static str, cx: &mut App) {
        let args: &'static [&'static str] = match pane {
            "battery" => &["--pane", "battery"],
            _ => &["--pane", "wifi"],
        };
        spawn_command("/usr/bin/rmac-system-settings", args, cx);
    }

    fn status_icon_size(kind: TopBarIndicatorKind) -> (f32, f32) {
        match kind {
            TopBarIndicatorKind::Network => (17.0, 12.3),
            _ => (15.0, 15.0),
        }
    }

    /// The battery is drawn rather than loaded so its fill tracks the level,
    /// turning red at 10% or below while discharging, as on macOS.
    fn battery_glyph(battery: Option<rmac_shell_status::BatteryIndicator>) -> impl IntoElement {
        const WIDTH: f32 = 25.0;
        const HEIGHT: f32 = 12.0;
        const INSET: f32 = 1.4;
        let (percentage, low) = battery
            .map(|battery| {
                (
                    battery.percentage.min(100),
                    battery.on_battery && battery.percentage <= 10,
                )
            })
            .unwrap_or((100, false));
        let fill_width = ((WIDTH - 2.0 * INSET - 2.4) * f32::from(percentage) / 100.0).max(1.5);
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .relative()
                    .w(px(WIDTH))
                    .h(px(HEIGHT))
                    .rounded(px(3.6))
                    .border(px(1.2))
                    .border_color(rgba(0xFFFFFF8C))
                    .child(
                        div()
                            .absolute()
                            .left(px(INSET - 1.2 + 0.2))
                            .top(px(INSET - 1.2 + 0.2))
                            .h(px(HEIGHT - 2.0 * INSET - 0.4))
                            .w(px(fill_width))
                            .rounded(px(1.8))
                            .bg(rgba(if low { 0xFF453AFF } else { 0xFFFFFFFF })),
                    ),
            )
            .child(
                div()
                    .ml(px(1.0))
                    .w(px(1.6))
                    .h(px(3.6))
                    .rounded_r(px(1.0))
                    .bg(rgba(0xFFFFFF8C)),
            )
    }

    fn system_menu() -> rmac_app_menu::Menu {
        use rmac_app_menu::Item;

        let logout_label = account_display_name()
            .map(|name| format!("Log Out {name}…"))
            .unwrap_or_else(|| "Log Out…".into());
        rmac_app_menu::Menu {
            label: "System".into(),
            items: vec![
                Item {
                    label: "About This Lulo OS".into(),
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
                    // The Mac shows ⇧⌘Q here. Nothing binds it yet: the
                    // confirmation lives in this menu, which no key can
                    // open (the ⌃F2 gap), so no hint is shown rather than
                    // one that does nothing.
                    shortcut: String::new(),
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
                "Each app is asked to quit first, so you can save your work.",
                "Restart",
            ),
            "system::shutdown" => (
                "Shut down this computer?",
                "Each app is asked to quit first, so you can save your work.",
                "Shut Down",
            ),
            "system::logout" => (
                "Log out now?",
                "Each app is asked to quit first, so you can save your work.",
                "Log Out",
            ),
            _ => ("Continue?", "Confirm this system action.", "Continue"),
        }
    }

    /// Top of the Recent Items submenu: its first row lines up with the
    /// parent row, as macOS submenus do.
    fn menu_item_top(menu: &rmac_app_menu::Menu, index: usize) -> f32 {
        BAR_HEIGHT + menu_model::MENU_TOP_GAP + app_menu_item_top(&menu.items, index)
            - menu_model::APP_MENU_PADDING
            - RECENT_HEADER_HEIGHT
    }

    fn recent_action_count(items: &[PathBuf], loading: bool, unavailable: bool) -> usize {
        if loading || unavailable || items.is_empty() {
            0
        } else {
            items.len() + 1
        }
    }

    fn recent_menu_height(items: usize, loading: bool, unavailable: bool) -> f32 {
        let listed = items > 0 && !loading && !unavailable;
        let rows = if listed { items + 1 } else { 1 };
        2.0 * menu_model::APP_MENU_PADDING
            + RECENT_HEADER_HEIGHT
            + rows as f32 * menu_model::APP_ROW_HEIGHT
            + if listed {
                menu_model::APP_SEPARATOR_HEIGHT
            } else {
                0.0
            }
    }

    fn recent_status_row(label: &'static str, color: u32) -> impl IntoElement {
        div()
            .id(format!("recent-items-status-{label}"))
            .role(Role::MenuItem)
            .aria_label(label)
            .h(px(menu_model::APP_ROW_HEIGHT))
            .pl(px(menu_model::APP_TEXT_INSET - EDGE))
            .flex()
            .items_center()
            .text_color(rgba(color))
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
                spawn_command("/usr/bin/rmac-system-settings", &["--pane", "about"], cx)
            }
            "system::settings" => open_or_focus_app(
                rmac_apps::identity::SYSTEM_SETTINGS,
                "/usr/bin/rmac-system-settings",
                cx,
            ),
            "system::software-center" => {
                spawn_command("gtk-launch", &["snap-store_snap-store"], cx)
            }
            "system::force-quit" => open_or_focus_app(
                rmac_apps::identity::SYSTEM_MONITOR,
                "/usr/bin/rmac-system-monitor",
                cx,
            ),
            "system::sleep" => spawn_command("systemctl", &["suspend"], cx),
            "system::restart" | "system::shutdown" | "system::logout" => quit_all_then(action, cx),
            "system::lock" => dispatch_shortcut("lock", cx),
            _ => eprintln!("unknown rmac system menu action: {action}"),
        }
    }

    /// Bring `app_id`'s window forward if it has one, or start `program`.
    fn open_or_focus_app(app_id: &'static str, program: &'static str, cx: &mut App) {
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let plan = match rmac_compositor_niri::snapshot().await {
                Ok(snapshot) => menu_model::open_or_focus(&snapshot, app_id),
                Err(error) => {
                    eprintln!("could not read windows before opening {app_id}: {error:?}");
                    menu_model::OpenOrFocus::Launch
                }
            };
            let actions = match plan {
                menu_model::OpenOrFocus::Launch => Vec::new(),
                menu_model::OpenOrFocus::Focus(window) => {
                    vec![rmac_compositor::Action::FocusWindow { window }]
                }
                menu_model::OpenOrFocus::Restore(windows) => {
                    let mut store = rmac_compositor::ParkingStore::load_default();
                    let actions = store.restore_actions(&windows);
                    if let Err(error) = store.save_default() {
                        eprintln!("could not save the parking set: {error}");
                    }
                    actions
                }
            };
            if actions.is_empty() {
                cx.update(|cx| spawn_command(program, &[], cx));
                return;
            }
            for action in &actions {
                if let Err(error) = rmac_compositor_niri::execute_action(action).await {
                    eprintln!("could not bring {app_id} forward: {error:?}");
                }
            }
        })
        .detach();
    }

    /// Log Out, Restart and Shut Down first ask every window to close, as
    /// macOS asks every app to quit, so an edited document gets its Save /
    /// Don't Save / Cancel alert instead of being lost. The session ends
    /// only once every window has gone; an app still open after
    /// `QUIT_ALL_GRACE` cancels the request and a notice names it.
    fn quit_all_then(action: String, cx: &mut App) {
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            let started = Instant::now();
            let mut asked = false;
            loop {
                let snapshot = match rmac_compositor_niri::snapshot().await {
                    Ok(snapshot) => snapshot,
                    Err(error) if !asked => {
                        // No window list to work from: the compositor itself
                        // is failing, and ending the session is the way out.
                        eprintln!("could not read windows before {action}: {error:?}");
                        break;
                    }
                    Err(error) => {
                        eprintln!("could not re-read windows during {action}: {error:?}");
                        cx.background_executor().timer(QUIT_ALL_CHECK).await;
                        continue;
                    }
                };
                if !asked {
                    asked = true;
                    for window in &snapshot.windows {
                        let close = rmac_compositor::Action::CloseWindow { window: window.id };
                        if let Err(error) = rmac_compositor_niri::execute_action(&close).await {
                            eprintln!("could not ask a window to close: {error:?}");
                        }
                    }
                    cx.background_executor().timer(QUIT_ALL_CHECK).await;
                    continue;
                }
                let remaining = snapshot
                    .windows
                    .iter()
                    .map(|window| window.app_id.as_deref().map(app_display_name))
                    .collect::<Vec<_>>();
                match quit_all_progress(&remaining, started.elapsed()) {
                    QuitAllProgress::Proceed => break,
                    QuitAllProgress::Wait => {
                        cx.background_executor().timer(QUIT_ALL_CHECK).await;
                    }
                    QuitAllProgress::Interrupted(apps) => {
                        let (summary, body) = quit_all_interrupted_copy(&action, &apps);
                        cx.update(|cx| post_system_notice(summary, body, cx));
                        return;
                    }
                }
            }
            cx.update(|cx| match action.as_str() {
                "system::restart" => spawn_command("systemctl", &["reboot"], cx),
                "system::shutdown" => spawn_command("systemctl", &["poweroff"], cx),
                _ => spawn_command(
                    "niri",
                    &["msg", "action", "quit", "--skip-confirmation"],
                    cx,
                ),
            });
        })
        .detach();
    }

    /// A transient notice through the session's notification server.
    fn post_system_notice(summary: String, body: String, cx: &mut App) {
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
                            "Lulo OS",
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

    struct MenuBackdrop {
        radius: f32,
        tint: u32,
    }

    impl Render for MenuBackdrop {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .rounded(px(self.radius))
                .bg(rgba(self.tint))
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
        let (radius, tint) = (panel.radius, panel.tint);
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
                    // -1 ignores the bar's reserved zone, so the margin is
                    // measured from the screen top exactly like the menu.
                    exclusive_zone: Some(px(-1.0)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(move |_| MenuBackdrop { radius, tint }),
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
