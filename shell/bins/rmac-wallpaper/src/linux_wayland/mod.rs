//! The wallpaper layer and, on it, the macOS 26 desktop: Desktop items on
//! Finder's icon grid, Stacks, the desktop menu, desktop widgets and the
//! Edit Widgets gallery. Numbers come from design-lab/desktop.html.

mod desktop;
mod gallery;
mod menu;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use gpui::{
    div, img, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px, rgba, svg,
    AnyElement, AnyWindowHandle, App, AssetSource, Bounds, Context, DisplayId, Entity, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    PlatformDisplay, Point, QuitMode, RenderImage, Role, SharedString, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use gpui_platform::application;
use rmac_desktop::settings::{Arrangement, DesktopSettings, GalleryTarget};
use rmac_desktop::widgets::{WidgetKind, WidgetLocation, WidgetSize};
use rmac_desktop_widgets::WidgetData;
use rmac_shell_ui::tokens;
use uuid::Uuid;

const READY_FILE_ENV: &str = "RMAC_WALLPAPER_READY_FILE";
const RENDER_COUNT_DIR_ENV: &str = "RMAC_WALLPAPER_RENDER_COUNT_DIR";
/// How often the Weather widget's forecast is refreshed.
const WEATHER_REFRESH: Duration = Duration::from_secs(15 * 60);
static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

/// Menu glyphs (shell/assets/menu), the desktop's folder and document
/// artwork (assets/icons) and the widget glyphs.
macro_rules! menu_icons {
    ($($name:literal),* $(,)?) => {
        const MENU_ICON_NAMES: &[&str] = &[$(concat!("menu/", $name, ".svg")),*];

        fn menu_icon_bytes(path: &str) -> Option<&'static [u8]> {
            $(
                if path == concat!("menu/", $name, ".svg") {
                    return Some(include_bytes!(concat!(
                        "../../../../assets/menu/",
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
    "checkmark",
    "chevron-right",
    "duplicate",
    "gear",
    "info",
    "new-folder",
    "open",
    "rename",
    "sort",
    "stacks",
    "trash",
);

pub(crate) fn menu_icon_path(name: &str) -> SharedString {
    SharedString::from(format!("menu/{name}.svg"))
}

pub(crate) const FOLDER_ICON: &str = "desktop/folder.svg";
pub(crate) const DOCUMENT_ICON: &str = "desktop/document.svg";

struct WallpaperAssets;

impl AssetSource for WallpaperAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = menu_icon_bytes(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = rmac_desktop_widgets::asset(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        let bytes: Option<&'static [u8]> = match path {
            FOLDER_ICON => Some(include_bytes!("../../../../../assets/icons/folder.svg")),
            DOCUMENT_ICON => Some(include_bytes!("../../../../../assets/icons/document.svg")),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(MENU_ICON_NAMES
            .iter()
            .chain([FOLDER_ICON, DOCUMENT_ICON].iter())
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(*asset))
            .collect())
    }
}

#[derive(Clone)]
struct PreparedSurface {
    image: Arc<RenderImage>,
    layout: rmac_wallpaper::Layout,
}

enum PreparedUpdate {
    Render(BTreeMap<Uuid, PreparedSurface>),
    Health(rmac_wallpaper_runtime::HealthSnapshot),
    Desktop {
        snapshot: Option<rmac_desktop::Snapshot>,
        error: Option<SharedString>,
    },
    Battery(Option<rmac_desktop_widgets::Battery>, bool),
    Weather(Result<rmac_weather::widget::WidgetWeather, rmac_weather::widget::Unavailable>),
    Gallery(GalleryTarget),
}

pub(crate) struct WallpaperStatus {
    surfaces: BTreeMap<Uuid, PreparedSurface>,
    health: rmac_wallpaper_runtime::HealthSnapshot,
    compositor: rmac_compositor::State,
    pub(crate) desktop: Option<rmac_desktop::Snapshot>,
    pub(crate) desktop_error: Option<SharedString>,
    desktop_requests: async_channel::Sender<rmac_desktop::SortOrder>,
    pub(crate) settings: DesktopSettings,
    saves: async_channel::Sender<DesktopSettings>,
    pub(crate) widgets: WidgetData,
    weather_wanted: Arc<AtomicBool>,
    weather_kick: async_channel::Sender<()>,
    /// The open Edit Widgets gallery.
    pub(crate) gallery: Option<AnyWindowHandle>,
    /// The display the most recent desktop interaction happened on.
    pub(crate) last_display: Option<DisplayId>,
}

pub(crate) struct StatusChannels {
    receiver: async_channel::Receiver<PreparedUpdate>,
    desktop_requests: async_channel::Sender<rmac_desktop::SortOrder>,
    saves: async_channel::Sender<DesktopSettings>,
    weather_wanted: Arc<AtomicBool>,
    weather_kick: async_channel::Sender<()>,
}

impl WallpaperStatus {
    fn new(settings: DesktopSettings, channels: StatusChannels, cx: &mut Context<Self>) -> Self {
        let receiver = channels.receiver;
        cx.spawn(async move |this, cx| {
            while let Ok(update) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        match update {
                            PreparedUpdate::Render(surfaces) => this.surfaces = surfaces,
                            PreparedUpdate::Health(health) => this.health = health,
                            PreparedUpdate::Desktop { snapshot, error } => {
                                this.desktop = snapshot;
                                this.desktop_error = error;
                            }
                            PreparedUpdate::Battery(battery, none) => {
                                this.widgets.battery = battery;
                                this.widgets.no_battery = none;
                            }
                            PreparedUpdate::Weather(weather) => {
                                this.widgets.weather = Some(weather);
                            }
                            PreparedUpdate::Gallery(target) => {
                                // Opened after this update, since the
                                // gallery reads the status as it opens.
                                let display = this.last_display;
                                let status = cx.entity();
                                cx.defer(move |cx| gallery::open(status, target, display, cx));
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
        // The Clock widget's second hand.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let alive = this.update(cx, |this, cx| {
                if this.has_widget(WidgetKind::Clock) {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        let status = Self {
            surfaces: BTreeMap::new(),
            health: rmac_wallpaper_runtime::HealthSnapshot::default(),
            compositor: rmac_compositor::State::default(),
            desktop: None,
            desktop_error: None,
            desktop_requests: channels.desktop_requests,
            settings,
            saves: channels.saves,
            widgets: WidgetData::default(),
            weather_wanted: channels.weather_wanted,
            weather_kick: channels.weather_kick,
            gallery: None,
            last_display: None,
        };
        status
            .weather_wanted
            .store(status.has_widget(WidgetKind::Weather), Ordering::Relaxed);
        status
    }

    pub(crate) fn has_widget(&self, kind: WidgetKind) -> bool {
        self.settings
            .widgets
            .iter()
            .any(|widget| widget.kind == kind)
    }

    /// Changes the saved desktop state, rescans in the new order when Sort
    /// By changed, and saves in the background (newest state wins).
    pub(crate) fn update_settings(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut DesktopSettings),
    ) {
        let before = self.settings.arrangement.sort_order();
        let had_weather = self.has_widget(WidgetKind::Weather);
        change(&mut self.settings);
        self.settings = std::mem::take(&mut self.settings).normalized();
        let after = self.settings.arrangement.sort_order();
        if before != after {
            let _ = self.desktop_requests.try_send(after);
        }
        let has_weather = self.has_widget(WidgetKind::Weather);
        self.weather_wanted.store(has_weather, Ordering::Relaxed);
        if has_weather && !had_weather {
            let _ = self.weather_kick.try_send(());
        }
        let _ = self.saves.try_send(self.settings.clone());
        cx.notify();
    }

    pub(crate) fn add_widget(
        &mut self,
        kind: WidgetKind,
        location: WidgetLocation,
        cx: &mut Context<Self>,
    ) {
        self.update_settings(cx, |settings| {
            let _ = settings.add_widget(kind, WidgetSize::Small, location);
        });
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

pub(crate) struct Wallpaper {
    display_id: u64,
    pub(crate) display: DisplayId,
    display_uuid: Uuid,
    render_count: u64,
    pub(crate) status: Entity<WallpaperStatus>,
    pub(crate) focus: FocusHandle,
    pub(crate) desk: desktop::DeskState,
    pub(crate) action_error: Option<SharedString>,
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
            display: display_id,
            display_uuid,
            render_count: 0,
            status,
            focus: cx.focus_handle(),
            desk: desktop::DeskState::default(),
            action_error: None,
        }
    }

    pub(crate) fn dismiss_app_drawer(&self, cx: &mut App) {
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

    /// Records this display as the one in use (for the gallery).
    pub(crate) fn note_display(&self, cx: &mut App) {
        let display = self.display;
        self.status.update(cx, |status, _| {
            status.last_display = Some(display);
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ItemAction {
    Open,
    Reveal,
}

pub(crate) fn spawn_item_action(path: PathBuf, action: ItemAction, cx: &mut App) {
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

pub(crate) fn spawn_settings(pane: &'static str, cx: &mut App) {
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
        let mut root = div()
            .id(format!("wallpaper-{}", self.display_id))
            .role(Role::Image)
            .aria_label("Desktop wallpaper")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.background_mouse_down(event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.background_context_menu(event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.pointer_moved(event, window, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.pointer_released(event, window, cx);
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
        root = root.children(self.render_desktop(window, cx));
        let desktop_error = self.status.read(cx).desktop_error.clone();
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

fn render_surface(surface: PreparedSurface) -> Vec<AnyElement> {
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
    let (saves_tx, saves_rx) = async_channel::unbounded::<DesktopSettings>();
    let (weather_kick, weather_kick_rx) = async_channel::bounded(1);
    let weather_wanted = Arc::new(AtomicBool::new(false));
    // A small file, read before the first frame so icons never jump.
    let settings = rmac_desktop::settings::load().unwrap_or_else(|error| {
        eprintln!("the desktop settings could not be read: {error}");
        DesktopSettings::default()
    });
    let initial_sort = settings.arrangement.sort_order();
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
                        let (surfaces, colors) = blocking::unblock(move || {
                            let mut prepared = BTreeMap::new();
                            let mut outputs = BTreeMap::new();
                            for surface in rasterized.surfaces {
                                if let Some(summary) =
                                    rmac_wallpaper_image::summarize_color(&surface.image)
                                {
                                    outputs.insert(
                                        surface.output.0.clone(),
                                        rmac_theme::WallpaperColor {
                                            dominant: summary.dominant,
                                            luminance: summary.luminance,
                                        },
                                    );
                                }
                                if let Some((output, surface)) = prepare_surface(surface) {
                                    prepared.insert(output, surface);
                                }
                            }
                            (prepared, rmac_theme::WallpaperColors { outputs })
                        })
                        .await;
                        match rmac_theme::WallpaperColorStore::from_environment()
                            .and_then(|store| store.publish(&colors))
                        {
                            Ok(_) => {}
                            Err(error) => {
                                eprintln!("wallpaper colour publication failed: {error}")
                            }
                        }
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
        .spawn(watch_desktop(
            prepared_tx.clone(),
            desktop_request_rx,
            initial_sort,
        ))
        .detach();
    cx.background_executor()
        .spawn(save_settings(saves_rx))
        .detach();
    cx.background_executor()
        .spawn(watch_battery(prepared_tx.clone()))
        .detach();
    cx.background_executor()
        .spawn(refresh_weather(
            prepared_tx.clone(),
            weather_wanted.clone(),
            weather_kick_rx,
        ))
        .detach();
    cx.background_executor()
        .spawn(watch_gallery_requests(prepared_tx))
        .detach();
    let channels = StatusChannels {
        receiver: prepared_rx,
        desktop_requests,
        saves: saves_tx,
        weather_wanted,
        weather_kick,
    };
    let status = cx.new(|cx| WallpaperStatus::new(settings, channels, cx));
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

/// Writes the newest desktop settings, skipping states a later change
/// already replaced.
async fn save_settings(saves: async_channel::Receiver<DesktopSettings>) {
    while let Ok(mut settings) = saves.recv().await {
        while let Ok(newer) = saves.try_recv() {
            settings = newer;
        }
        if blocking::unblock(move || rmac_desktop::settings::save(&settings))
            .await
            .is_err()
        {
            eprintln!("the desktop settings could not be saved");
        }
    }
}

async fn watch_battery(updates: async_channel::Sender<PreparedUpdate>) {
    let publish = |updates: async_channel::Sender<PreparedUpdate>| async move {
        let (battery, none) = blocking::unblock(rmac_desktop_widgets::read_battery).await;
        updates
            .send(PreparedUpdate::Battery(battery, none))
            .await
            .is_ok()
    };
    if !publish(updates.clone()).await {
        return;
    }
    let (events_tx, events_rx) = async_channel::bounded(4);
    let watcher = async move {
        if let Err(error) = rmac_power::watch(events_tx).await {
            eprintln!("the battery watcher stopped: {error}");
        }
    };
    let forward = async move {
        while let Ok(event) = events_rx.recv().await {
            if event == rmac_power::WatchEvent::Changed && !publish(updates.clone()).await {
                break;
            }
        }
    };
    futures_util::join!(watcher, forward);
}

async fn refresh_weather(
    updates: async_channel::Sender<PreparedUpdate>,
    wanted: Arc<AtomicBool>,
    kick: async_channel::Receiver<()>,
) {
    loop {
        if wanted.load(Ordering::Relaxed) {
            let weather = blocking::unblock(|| rmac_desktop_widgets::read_weather(true)).await;
            if updates
                .send(PreparedUpdate::Weather(weather))
                .await
                .is_err()
            {
                break;
            }
        }
        let timer = async_io::Timer::after(WEATHER_REFRESH).fuse();
        let kicked = kick.recv().fuse();
        futures_util::pin_mut!(timer, kicked);
        futures_util::select! {
            _ = timer => {}
            kicked = kicked => if kicked.is_err() {
                break;
            },
        }
    }
}

async fn watch_gallery_requests(updates: async_channel::Sender<PreparedUpdate>) {
    let Ok(directory) = blocking::unblock(rmac_desktop::settings::prepare_runtime_directory).await
    else {
        eprintln!("widget gallery requests are unavailable");
        return;
    };
    let (events_tx, events_rx) = async_channel::bounded(1);
    let watcher = blocking::unblock(move || {
        rmac_desktop::watch_file(&directory, "desktop-widget-gallery", move || {
            let _ = events_tx.try_send(());
        })
    })
    .await;
    let Ok(_watcher) = watcher else {
        eprintln!("widget gallery requests are unavailable");
        return;
    };
    loop {
        if let Some(target) = blocking::unblock(rmac_desktop::settings::take_gallery_request).await
        {
            if updates.send(PreparedUpdate::Gallery(target)).await.is_err() {
                break;
            }
        }
        if events_rx.recv().await.is_err() {
            break;
        }
    }
}

async fn watch_desktop(
    updates: async_channel::Sender<PreparedUpdate>,
    requests: async_channel::Receiver<rmac_desktop::SortOrder>,
    initial_sort: rmac_desktop::SortOrder,
) {
    let directory = match blocking::unblock(rmac_desktop::directory_from_environment).await {
        Ok(directory) => directory,
        Err(_) => {
            let _ = updates
                .send(PreparedUpdate::Desktop {
                    snapshot: None,
                    error: Some("The Desktop directory is unavailable".into()),
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
            })
            .await;
        return;
    };
    let mut sort = initial_sort;
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
        .send(PreparedUpdate::Desktop { snapshot, error })
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
                    // The background takes keyboard focus when clicked, so
                    // the desktop's selection shortcuts and menus work.
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
    let app = application()
        .with_assets(WallpaperAssets)
        .with_quit_mode(QuitMode::Explicit);
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
            let mut removed_outputs = BTreeSet::new();
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
    previous: &BTreeSet<Uuid>,
    current: &BTreeSet<Uuid>,
    removed: &mut BTreeSet<Uuid>,
) {
    if rmac_shell_layer::output_reappeared(previous, current, removed) {
        std::process::exit(rmac_shell_layer::WAYLAND_OUTPUT_RESTART_EXIT_CODE);
    }
}
