//! Supervised, on-demand Quick Settings surface.

mod render;
mod view;

use std::borrow::Cow;

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, Application, AssetSource,
    BorrowAppContext as _, Bounds, Global, Pixels, Result, SharedString, WeakEntity,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions,
};
use gpui_component::Root;

use crate::view::QuickSettingsView;

#[cfg(not(target_os = "linux"))]
const WIDTH: f32 = 380.0;
#[cfg(not(target_os = "linux"))]
const HEIGHT: f32 = 548.0;
#[cfg(not(target_os = "linux"))]
const EDGE_GAP: f32 = 12.0;
#[cfg(not(target_os = "linux"))]
const TOP_GAP: f32 = 44.0;

#[derive(rust_embed::RustEmbed)]
#[folder = "../system-settings/assets"]
#[include = "icons/**/*.svg"]
struct QuickSettingsAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = QuickSettingsAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = QuickSettingsAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

#[derive(Clone)]
pub(crate) struct ActivePopover {
    token: u64,
    view: WeakEntity<QuickSettingsView>,
    window: AnyWindowHandle,
}

pub(crate) struct QuickSettingsService {
    active: Option<ActivePopover>,
    next_token: u64,
}

impl Global for QuickSettingsService {}

fn popover_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some("org.rmac.QuickSettings".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn fallback_bounds(cx: &App) -> Bounds<Pixels> {
    let popover_size = size(px(WIDTH), px(HEIGHT));
    cx.primary_display()
        .map(|display| display.bounds())
        .map(|display| {
            Bounds::new(
                point(
                    display.origin.x + display.size.width - px(WIDTH + EDGE_GAP),
                    display.origin.y + px(TOP_GAP),
                ),
                popover_size,
            )
        })
        .unwrap_or_else(|| Bounds::new(point(px(200.0), px(80.0)), popover_size))
}

fn dismiss_active(cx: &mut App) -> bool {
    let active = cx.read_global::<QuickSettingsService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let dismissed = cx
                .update_window(active.window, |_, window, cx| {
                    view.update(cx, |view, cx| view.dismiss(window, cx));
                })
                .is_ok();
            cx.update_global::<QuickSettingsService, _>(|service, _| service.active = None);
            if dismissed {
                return true;
            }
        }
        cx.update_global::<QuickSettingsService, _>(|service, _| service.active = None);
    }
    false
}

fn open_popover(bounds: Bounds<Pixels>, cx: &mut App) {
    let token = cx.update_global::<QuickSettingsService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    let mut popover = None;
    let handle = cx.open_window(popover_options(bounds), |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| QuickSettingsView::new(token, window, cx));
        popover = Some(view.downgrade());
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, popover) {
        cx.update_global::<QuickSettingsService, _>(|service, _| {
            service.active = Some(ActivePopover {
                token,
                view,
                window: handle.into(),
            });
        });
        cx.activate(true);
    }
}

#[cfg(not(target_os = "linux"))]
fn route_shortcut(cx: &mut App) {
    if dismiss_active(cx) {
        return;
    }
    open_popover(fallback_bounds(cx), cx);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn route_activation(activation: rmac_shell_activation_runtime::Activation, cx: &mut App) {
    if dismiss_active(cx) {
        return;
    }
    let context = match activation.context() {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Quick Settings activation rejected: {error}");
            return;
        }
    };
    let description = match rmac_quick_settings::surface::plan_invocation(
        context.invocation(),
        context.compositor(),
    ) {
        Ok(description) => description,
        Err(error) => {
            eprintln!("Quick Settings surface plan rejected: {error}");
            return;
        }
    };
    let bounds = match context.top_right_bounds(
        description.logical_width,
        description.logical_height,
        description.top_margin,
        description.right_margin,
    ) {
        Ok(bounds) => Bounds::new(
            point(px(bounds.x), px(bounds.y)),
            size(px(bounds.width), px(bounds.height)),
        ),
        Err(error) => {
            eprintln!("Quick Settings surface bounds rejected: {error}");
            return;
        }
    };
    open_popover(bounds, cx);
}

fn notify_ready() -> std::result::Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = std::process::Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=Quick Settings live invocation endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected Quick Settings readiness".to_owned())
}

fn main() {
    Application::new()
        .with_assets(CombinedAssets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.set_global(QuickSettingsService {
                active: None,
                next_token: 0,
            });

            #[cfg(target_os = "linux")]
            let (activation_tx, activation_rx) = async_channel::bounded(8);
            #[cfg(target_os = "linux")]
            let activation_done = cx.background_executor().spawn(async move {
                rmac_shell_activation_runtime::watch(
                    rmac_shortcuts::ShortcutId("quick-settings".into()),
                    activation_tx,
                )
                .await
            });
            #[cfg(target_os = "linux")]
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                let consume = async {
                    while let Ok(update) = activation_rx.recv().await {
                        match update {
                            rmac_shell_activation_runtime::Update::Ready => {
                                blocking::unblock(notify_ready).await?;
                            }
                            rmac_shell_activation_runtime::Update::Activated(activation) => {
                                if cx.update(|cx| route_activation(*activation, cx)).is_err() {
                                    return Err(
                                        "Quick Settings application context stopped".to_owned()
                                    );
                                }
                            }
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async { activation_done.await.map_err(|error| error.to_string()) };
                if let Err(error) = futures_util::try_join!(watcher, consume) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            })
            .detach();

            #[cfg(not(target_os = "linux"))]
            {
                let (shortcut_tx, shortcut_rx) = async_channel::bounded(8);
                let (ready_tx, ready_rx) = async_channel::bounded(1);
                let shortcut_done = cx.background_executor().spawn(async move {
                    rmac_shortcuts::watch_dispatches_ready(
                        rmac_shortcuts::ShortcutId("quick-settings".into()),
                        shortcut_tx,
                        ready_tx,
                    )
                    .await
                });
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    let consume = async {
                        while shortcut_rx.recv().await.is_ok() {
                            if cx.update(route_shortcut).is_err() {
                                return Err("Quick Settings application context stopped".to_owned());
                            }
                        }
                        Ok::<(), String>(())
                    };
                    let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                    let readiness = async {
                        ready_rx.recv().await.map_err(|_| {
                            "Quick Settings endpoint stopped before readiness".to_owned()
                        })?;
                        blocking::unblock(notify_ready).await
                    };
                    if let Err(error) = futures_util::try_join!(watcher, consume, readiness) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                })
                .detach();
            }

            #[cfg(not(target_os = "linux"))]
            if std::env::args().any(|argument| argument == "--show") {
                route_shortcut(cx);
            }
        });
}
