//! Supervised, on-demand, service-backed Notification Center panel.

mod model;
mod render;
mod view;

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, Application, BorrowAppContext as _,
    Bounds, Global, Pixels, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowKind, WindowOptions,
};
use gpui_component::Root;

use crate::view::NotificationCenterView;

#[cfg(not(target_os = "linux"))]
const WIDTH: f32 = 420.0;
#[cfg(not(target_os = "linux"))]
const HEIGHT: f32 = 720.0;
#[cfg(not(target_os = "linux"))]
const EDGE_GAP: f32 = 12.0;
#[cfg(not(target_os = "linux"))]
const TOP_GAP: f32 = 44.0;

#[derive(Clone)]
pub(crate) struct ActivePanel {
    token: u64,
    view: WeakEntity<NotificationCenterView>,
    window: AnyWindowHandle,
}

pub(crate) struct NotificationCenterService {
    active: Option<ActivePanel>,
    next_token: u64,
}

impl Global for NotificationCenterService {}

fn panel_options(bounds: Bounds<Pixels>) -> WindowOptions {
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
        app_id: Some("org.rmac.NotificationCenter".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn fallback_bounds(cx: &App) -> Bounds<Pixels> {
    let panel_size = size(px(WIDTH), px(HEIGHT));
    cx.primary_display()
        .map(|display| display.bounds())
        .map(|display| {
            Bounds::new(
                point(
                    display.origin.x + display.size.width - px(WIDTH + EDGE_GAP),
                    display.origin.y + px(TOP_GAP),
                ),
                panel_size,
            )
        })
        .unwrap_or_else(|| Bounds::new(point(px(200.0), px(80.0)), panel_size))
}

fn notify_ready() -> Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = std::process::Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=Notification Center live invocation endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected Notification Center readiness".to_owned())
}

fn dismiss_active(cx: &mut App) -> bool {
    let active =
        cx.read_global::<NotificationCenterService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let dismissed = cx
                .update_window(active.window, |_, window, cx| {
                    view.update(cx, |view, cx| view.dismiss(window, cx));
                })
                .is_ok();
            cx.update_global::<NotificationCenterService, _>(|service, _| service.active = None);
            if dismissed {
                return true;
            }
        }
        cx.update_global::<NotificationCenterService, _>(|service, _| service.active = None);
    }
    false
}

fn open_panel(bounds: Bounds<Pixels>, cx: &mut App) {
    let token = cx.update_global::<NotificationCenterService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    let mut panel = None;
    let handle = cx.open_window(panel_options(bounds), |window, cx| {
        window.set_window_title("Notification Center");
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| NotificationCenterView::new(token, window, cx));
        panel = Some(view.downgrade());
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, panel) {
        cx.update_global::<NotificationCenterService, _>(|service, _| {
            service.active = Some(ActivePanel {
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
    open_panel(fallback_bounds(cx), cx);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn route_activation(activation: rmac_shell_activation_runtime::Activation, cx: &mut App) {
    if dismiss_active(cx) {
        return;
    }
    let context = match activation.context() {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Notification Center activation rejected: {error}");
            return;
        }
    };
    let description = match rmac_notifications_linux::center_surface::plan_invocation(
        context.invocation(),
        context.compositor(),
    ) {
        Ok(description) => description,
        Err(error) => {
            eprintln!("Notification Center surface plan rejected: {error}");
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
            eprintln!("Notification Center surface bounds rejected: {error}");
            return;
        }
    };
    open_panel(bounds, cx);
}

fn main() {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.set_global(NotificationCenterService {
                active: None,
                next_token: 0,
            });

            #[cfg(target_os = "linux")]
            let (activation_tx, activation_rx) = async_channel::bounded(8);
            #[cfg(target_os = "linux")]
            let activation_done = cx.spawn(async move |_: &mut gpui::AsyncApp| {
                rmac_shell_activation_runtime::watch(
                    rmac_shortcuts::ShortcutId("notification-center".into()),
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
                                    return Err("Notification Center application context stopped"
                                        .to_owned());
                                }
                            }
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async {
                    activation_done.await.map_err(|error| {
                        format!(
                            "Notification Center shell activation {:?} failed: {}",
                            error.operation(),
                            error.detail()
                        )
                    })
                };
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
                        rmac_shortcuts::ShortcutId("notification-center".into()),
                        shortcut_tx,
                        ready_tx,
                    )
                    .await
                });
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    let consume = async {
                        while shortcut_rx.recv().await.is_ok() {
                            if cx.update(route_shortcut).is_err() {
                                return Err(
                                    "Notification Center application context stopped".to_owned()
                                );
                            }
                        }
                        Ok::<(), String>(())
                    };
                    let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                    let readiness = async {
                        ready_rx.recv().await.map_err(|_| {
                            "Notification Center endpoint stopped before readiness".to_owned()
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
