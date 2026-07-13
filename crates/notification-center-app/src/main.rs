//! Supervised, on-demand, service-backed Notification Center panel.

mod model;
mod render;
mod view;

use gpui::{
    point, px, size, AnyWindowHandle, App, AppContext as _, Application, BorrowAppContext as _,
    Bounds, Global, WeakEntity, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowKind, WindowOptions,
};
use gpui_component::Root;

use crate::view::NotificationCenterView;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 720.0;
const EDGE_GAP: f32 = 12.0;
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

fn panel_options(cx: &App) -> WindowOptions {
    let panel_size = size(px(WIDTH), px(HEIGHT));
    let bounds = cx
        .primary_display()
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
        .unwrap_or_else(|| Bounds::new(point(px(200.0), px(80.0)), panel_size));
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

fn notify_ready() -> Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = std::process::Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=Notification Center shortcut endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected Notification Center readiness".to_owned())
}

fn route_shortcut(cx: &mut App) {
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
                return;
            }
        }
        cx.update_global::<NotificationCenterService, _>(|service, _| service.active = None);
    }

    let token = cx.update_global::<NotificationCenterService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    let mut panel = None;
    let handle = cx.open_window(panel_options(cx), |window, cx| {
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

fn main() {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.set_global(NotificationCenterService {
                active: None,
                next_token: 0,
            });

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

            if std::env::args().any(|argument| argument == "--show") {
                route_shortcut(cx);
            }
        });
}
