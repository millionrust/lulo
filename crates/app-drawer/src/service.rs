#[cfg(not(target_os = "linux"))]
use gpui::WindowDecorations;
use gpui::{
    point, px, size, AnyWindowHandle, App as GpuiApp, AppContext as _, BorrowAppContext as _,
    Bounds, Global, KeyBinding, Pixels, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions,
};

use crate::view::{AppDrawer, DRAWER_HEIGHT, DRAWER_WIDTH};

#[cfg(target_os = "linux")]
const LINUX_SHORTCUT_ENDPOINT: &str = "app-drawer";
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
    #[cfg(target_os = "linux")]
    catcher: Option<AnyWindowHandle>,
    #[cfg(target_os = "linux")]
    pending_dismiss: Option<u64>,
}

impl Global for AppDrawerService {}

pub(crate) fn release(token: u64, cx: &mut GpuiApp) {
    if cx.has_global::<AppDrawerService>() {
        let catcher: Option<AnyWindowHandle> =
            cx.update_global::<AppDrawerService, _>(|service, _| {
                let matches = service
                    .active
                    .as_ref()
                    .is_some_and(|active| active.token == token);
                if matches {
                    service.active = None;
                }
                #[cfg(target_os = "linux")]
                {
                    if service.pending_dismiss == Some(token) {
                        service.pending_dismiss = None;
                    }
                    if matches {
                        service.catcher.take()
                    } else {
                        None
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    None
                }
            });
        if let Some(catcher) = catcher {
            let _ = catcher.update(cx, |_, window, _| window.remove_window());
        }
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
        .arg("--status=Apps shortcut endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected Apps readiness".to_owned())
}

fn dismiss_active(cx: &mut GpuiApp) -> bool {
    let active = cx.read_global::<AppDrawerService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let dismissed = cx
                .update_window(active.window, |_, window, cx| {
                    view.update(cx, |view, cx| view.dismiss(window, cx));
                })
                .is_ok();
            release(active.token, cx);
            if dismissed {
                return true;
            }
        }
        release(active.token, cx);
    }
    false
}

#[cfg(target_os = "linux")]
fn drawer_options(bounds: Bounds<Pixels>) -> WindowOptions {
    use gpui::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};

    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.0), px(0.0)),
            bounds.size,
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "rmac-app-drawer".into(),
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some(rmac_ui::app_id::APP_DRAWER.to_owned()),
        ..Default::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn drawer_options(bounds: Bounds<Pixels>) -> WindowOptions {
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
        app_id: Some(rmac_ui::app_id::APP_DRAWER.to_owned()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

fn fallback_bounds(cx: &GpuiApp) -> Bounds<Pixels> {
    WindowBounds::centered(size(px(DRAWER_WIDTH), px(DRAWER_HEIGHT)), cx).get_bounds()
}

fn open_drawer(bounds: Bounds<Pixels>, cx: &mut GpuiApp) {
    let token = cx.update_global::<AppDrawerService, _>(|service, _| {
        service.next_token = service.next_token.wrapping_add(1).max(1);
        service.next_token
    });
    #[cfg(target_os = "linux")]
    {
        // Map the outside catcher before the visible panel so a quick first
        // click cannot arrive while only the panel is ready for input.
        let display = cx
            .displays()
            .into_iter()
            .find(|display| {
                let screen = display.bounds();
                bounds.origin.x >= screen.origin.x
                    && bounds.origin.x < screen.origin.x + screen.size.width
                    && bounds.origin.y >= screen.origin.y
                    && bounds.origin.y < screen.origin.y + screen.size.height
            })
            .or_else(|| cx.primary_display());
        let catcher = display.and_then(|display| {
            rmac_ui::open_outside_click_catcher_around(
                "rmac-app-drawer-click-catcher",
                display,
                px(29.0),
                Some(bounds),
                move |cx| {
                    let active = cx.read_global::<AppDrawerService, _>(|service, _| {
                        service.active.clone().filter(|active| active.token == token)
                    });
                    if active.is_some() {
                        dismiss_active(cx);
                    } else {
                        cx.update_global::<AppDrawerService, _>(|service, _| {
                            service.pending_dismiss = Some(token);
                        });
                    }
                },
                cx,
            )
        });
        cx.update_global::<AppDrawerService, _>(|service, _| service.catcher = catcher);
    }
    let mut drawer = None;
    let handle = cx.open_window(drawer_options(bounds), |window, cx| {
        window.set_window_title("Apps");
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| AppDrawer::new(Some(token), window, cx));
        drawer = Some(view.downgrade());
        cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, drawer) {
        #[cfg(target_os = "linux")]
        let cancel = cx.update_global::<AppDrawerService, _>(|service, _| {
            service.active = Some(ActiveDrawer {
                token,
                view,
                window: handle.into(),
            });
            service.pending_dismiss.take() == Some(token)
        });
        #[cfg(not(target_os = "linux"))]
        cx.update_global::<AppDrawerService, _>(|service, _| {
            service.active = Some(ActiveDrawer {
                token,
                view,
                window: handle.into(),
            });
        });
        #[cfg(target_os = "linux")]
        if cancel {
            dismiss_active(cx);
            return;
        }
        cx.activate(true);
    } else {
        #[cfg(target_os = "linux")]
        {
            let catcher = cx.update_global::<AppDrawerService, _>(|service, _| {
                service.pending_dismiss = None;
                service.catcher.take()
            });
            if let Some(catcher) = catcher {
                let _ = catcher.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn route_shortcut(cx: &mut GpuiApp) {
    if dismiss_active(cx) {
        return;
    }
    open_drawer(fallback_bounds(cx), cx);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn route_activation(activation: rmac_shell_activation_runtime::Activation, cx: &mut GpuiApp) {
    if dismiss_active(cx) {
        return;
    }
    let context = match activation.context() {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Apps activation rejected: {error}");
            return;
        }
    };
    let bounds = match context.centered_bounds(DRAWER_WIDTH as f64, DRAWER_HEIGHT as f64) {
        Ok(bounds) => Bounds::new(
            point(px(bounds.x), px(bounds.y)),
            size(px(bounds.width), px(bounds.height)),
        ),
        Err(error) => {
            eprintln!("Apps surface bounds rejected: {error}");
            return;
        }
    };
    open_drawer(bounds, cx);
}

pub(crate) fn run(show_on_start: bool) {
    rmac_ui::application()
        .with_assets(gpui_component_assets::Assets)
        .with_quit_mode(gpui::QuitMode::Explicit)
        .run(move |cx: &mut GpuiApp| {
            rmac_ui::init_application(cx);
            rmac_ui::install_surface_idle_exit(cx);
            cx.bind_keys(key_bindings());
            cx.set_global(AppDrawerService {
                active: None,
                next_token: 0,
                #[cfg(target_os = "linux")]
                catcher: None,
                #[cfg(target_os = "linux")]
                pending_dismiss: None,
            });

            #[cfg(target_os = "linux")]
            let (activation_tx, activation_rx) = async_channel::bounded(16);
            #[cfg(target_os = "linux")]
            let activation_done = cx.spawn(async move |_: &mut gpui::AsyncApp| {
                rmac_shell_activation_runtime::watch(
                    rmac_shortcuts::ShortcutId(LINUX_SHORTCUT_ENDPOINT.into()),
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
                                cx.update(|cx| route_activation(*activation, cx));
                            }
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async {
                    activation_done.await.map_err(|error| {
                        format!(
                            "Apps shell activation {:?} failed: {}",
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
                        rmac_shortcuts::ShortcutId("app-drawer".into()),
                        shortcut_tx,
                        ready_tx,
                    )
                    .await
                });
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    let consume = async {
                        while shortcut_rx.recv().await.is_ok() {
                            cx.update(route_shortcut);
                        }
                        Ok::<(), String>(())
                    };
                    let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                    let readiness = async {
                        ready_rx
                            .recv()
                            .await
                            .map_err(|_| "Apps endpoint stopped before readiness".to_owned())?;
                        blocking::unblock(notify_ready).await
                    };
                    if let Err(error) = futures_util::try_join!(watcher, consume, readiness) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                })
                .detach();
            }

            if show_on_start {
                open_drawer(fallback_bounds(cx), cx);
            }
        });
}
