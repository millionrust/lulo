use gpui::{
    AnyWindowHandle, App as GpuiApp, AppContext as _, Application, BorrowAppContext as _, Global,
    KeyBinding, WeakEntity,
};
use gpui_component::Root;

use crate::view::AppDrawer;
use crate::{ClearSearch, Launch, MoveDown, MoveLeft, MoveRight, MoveUp};

#[derive(Clone)]
struct ActiveDrawer {
    token: u64,
    view: WeakEntity<AppDrawer>,
    window: AnyWindowHandle,
}

struct AppDrawerService {
    active: Option<ActiveDrawer>,
    next_token: u64,
}

impl Global for AppDrawerService {}

pub(crate) fn release(token: u64, cx: &mut GpuiApp) {
    if cx.has_global::<AppDrawerService>() {
        cx.update_global::<AppDrawerService, _>(|service, _| {
            if service
                .active
                .as_ref()
                .is_some_and(|active| active.token == token)
            {
                service.active = None;
            }
        });
    }
}

pub(crate) fn key_bindings() -> [KeyBinding; 6] {
    [
        KeyBinding::new(
            rmac_ui::shortcuts::LEFT.keystroke,
            MoveLeft,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::RIGHT.keystroke,
            MoveRight,
            Some("AppDrawer"),
        ),
        KeyBinding::new(rmac_ui::shortcuts::UP.keystroke, MoveUp, Some("AppDrawer")),
        KeyBinding::new(
            rmac_ui::shortcuts::DOWN.keystroke,
            MoveDown,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::ENTER.keystroke,
            Launch,
            Some("AppDrawer"),
        ),
        KeyBinding::new(
            rmac_ui::shortcuts::ESCAPE.keystroke,
            ClearSearch,
            Some("AppDrawer"),
        ),
    ]
}

fn notify_ready() -> Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = std::process::Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=App Drawer shortcut endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected App Drawer readiness".to_owned())
}

fn route_shortcut(cx: &mut GpuiApp) {
    let active = cx.read_global::<AppDrawerService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let dismissed = cx
                .update_window(active.window, |_, window, cx| {
                    view.update(cx, |view, cx| view.dismiss(window, cx));
                })
                .is_ok();
            cx.update_global::<AppDrawerService, _>(|service, _| service.active = None);
            if dismissed {
                return;
            }
        }
        cx.update_global::<AppDrawerService, _>(|service, _| service.active = None);
    }

    let token = cx.update_global::<AppDrawerService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    let mut drawer = None;
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::APP_DRAWER, 1080.0, 720.0, cx);
    let handle = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| AppDrawer::new(Some(token), window, cx));
        drawer = Some(view.downgrade());
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, drawer) {
        cx.update_global::<AppDrawerService, _>(|service, _| {
            service.active = Some(ActiveDrawer {
                token,
                view,
                window: handle.into(),
            });
        });
        cx.activate(true);
    }
}

pub(crate) fn run(show_on_start: bool) {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut GpuiApp| {
            rmac_ui::init_application(cx);
            cx.bind_keys(key_bindings());
            cx.set_global(AppDrawerService {
                active: None,
                next_token: 0,
            });

            let (shortcut_tx, shortcut_rx) = async_channel::bounded(8);
            let (ready_tx, ready_rx) = async_channel::bounded(1);
            let shortcut_done = cx.background_executor().spawn(async move {
                rmac_shortcuts::watch_dispatches_ready(
                    rmac_shortcuts::ShortcutId("app-drawer".into()),
                    shortcut_tx,
                    ready_tx,
                )
                .await
            });
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                let consume = async {
                    while shortcut_rx.recv().await.is_ok() {
                        if cx.update(route_shortcut).is_err() {
                            return Err("App Drawer application context stopped".to_owned());
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                let readiness = async {
                    ready_rx
                        .recv()
                        .await
                        .map_err(|_| "App Drawer endpoint stopped before readiness".to_owned())?;
                    blocking::unblock(notify_ready).await
                };
                if let Err(error) = futures_util::try_join!(watcher, consume, readiness) {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            })
            .detach();

            if show_on_start {
                route_shortcut(cx);
            }
        });
}
