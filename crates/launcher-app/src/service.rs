//! Launcher service, environment, provider registry, and overlay authority.

mod overlay;
mod registry;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use gpui::{point, Bounds};
use gpui::{
    px, size, AnyWindowHandle, App, AppContext as _, Application, BorrowAppContext as _,
    ClipboardItem, Global, SharedString, WeakEntity, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowKind, WindowOptions,
};
use gpui_component::Root;
use rmac_launcher_runtime::{CatalogUpdate, Registry, SettingsUpdate};

use crate::view::{LauncherView, OverlayEnvironment};
pub(crate) use overlay::release;
#[cfg(target_os = "linux")]
use overlay::route_activation;
#[cfg(not(target_os = "linux"))]
use overlay::route_shortcut;
use registry::build_registry;

#[cfg(not(target_os = "linux"))]
const WIDTH: f32 = 720.0;
#[cfg(not(target_os = "linux"))]
const HEIGHT: f32 = 540.0;

#[derive(Clone)]
struct ActiveOverlay {
    token: u64,
    view: WeakEntity<LauncherView>,
    window: AnyWindowHandle,
}

struct LauncherService {
    application_provider: rmac_launcher_providers::ApplicationProvider,
    settings: rmac_shell_settings::ShellSettings,
    registry: Arc<Registry>,
    settings_error: Option<SharedString>,
    active: Option<ActiveOverlay>,
    next_overlay: u64,
    clipboard: async_channel::Sender<String>,
}

impl Global for LauncherService {}

fn notify_ready() -> Result<(), String> {
    if std::env::var_os("NOTIFY_SOCKET").is_none() {
        return Ok(());
    }
    let status = Command::new("/usr/bin/systemd-notify")
        .arg("--ready")
        .arg("--status=Launcher live invocation endpoint ready")
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "systemd rejected Launcher readiness".to_owned())
}

fn update_active_catalog(update: CatalogUpdate, cx: &mut App) {
    let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            view.update(cx, |view, cx| view.apply_catalog(update, cx));
        }
    }
}

fn apply_settings_update(update: SettingsUpdate, cx: &mut App) {
    match update {
        SettingsUpdate::Unavailable(_) => {
            let message: SharedString =
                "Search preferences are unavailable; last-known-good privacy remains active".into();
            let active = cx.update_global::<LauncherService, _>(|service, _| {
                service.settings_error = Some(message.clone());
                service.active.clone()
            });
            if let Some(view) = active.and_then(|active| active.view.upgrade()) {
                view.update(cx, |view, cx| {
                    view.set_settings_error(message, cx);
                });
            }
        }
        SettingsUpdate::Snapshot(settings) => {
            let settings = *settings;
            let application_provider = cx.read_global::<LauncherService, _>(|service, _| {
                service.application_provider.clone()
            });
            let (registry, error) = match build_registry(&application_provider, &settings) {
                Ok(registry) => (registry, None),
                Err(error) => {
                    let current =
                        cx.read_global::<LauncherService, _>(|service, _| service.registry.clone());
                    (current, Some(error))
                }
            };
            let active = cx.update_global::<LauncherService, _>(|service, _| {
                service.registry = registry.clone();
                service.settings = settings.clone();
                service.settings_error = error.clone();
                service.active.clone()
            });
            if let Some(view) = active.and_then(|active| active.view.upgrade()) {
                view.update(cx, |view, cx| {
                    view.apply_environment(registry, settings, error, cx)
                });
            }
        }
    }
}

pub(crate) fn run() {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);

            let application_provider = rmac_launcher_providers::ApplicationProvider::default();
            let settings = rmac_shell_settings::ShellSettings::default();
            let registry = build_registry(&application_provider, &settings)
                .expect("built-in launcher providers must have distinct stable identities");
            let (clipboard_tx, clipboard_rx) = async_channel::bounded(8);
            cx.set_global(LauncherService {
                application_provider: application_provider.clone(),
                settings,
                registry,
                settings_error: None,
                active: None,
                next_overlay: 0,
                clipboard: clipboard_tx,
            });

            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                while let Ok(text) = clipboard_rx.recv().await {
                    if cx
                        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();

            #[cfg(target_os = "linux")]
            let (activation_tx, activation_rx) = async_channel::bounded(16);
            #[cfg(target_os = "linux")]
            let activation_done = cx.spawn(async move |_: &mut gpui::AsyncApp| {
                rmac_shell_activation_runtime::watch(
                    rmac_shortcuts::ShortcutId("launcher".into()),
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
                                    return Err("Launcher application context stopped".to_owned());
                                }
                            }
                        }
                    }
                    Ok::<(), String>(())
                };
                let watcher = async {
                    activation_done.await.map_err(|error| {
                        format!(
                            "Launcher shell activation {:?} failed: {}",
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
                let (shortcut_tx, shortcut_rx) = async_channel::bounded(16);
                let (ready_tx, ready_rx) = async_channel::bounded(1);
                let shortcut_done = cx.background_executor().spawn(async move {
                    rmac_shortcuts::watch_dispatches_ready(
                        rmac_shortcuts::ShortcutId("launcher".into()),
                        shortcut_tx,
                        ready_tx,
                    )
                    .await
                });
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    let consume = async {
                        while let Ok(event) = shortcut_rx.recv().await {
                            if cx.update(|cx| route_shortcut(event, cx)).is_err() {
                                return Err("Launcher application context stopped".to_owned());
                            }
                        }
                        Ok::<(), String>(())
                    };
                    let watcher = async { shortcut_done.await.map_err(|error| error.to_string()) };
                    let readiness = async {
                        ready_rx
                            .recv()
                            .await
                            .map_err(|_| "Launcher endpoint stopped before readiness".to_owned())?;
                        blocking::unblock(notify_ready).await
                    };
                    if let Err(error) = futures_util::try_join!(watcher, consume, readiness) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                })
                .detach();
            }

            let (catalog_tx, catalog_rx) = async_channel::bounded(8);
            cx.background_executor()
                .spawn(rmac_launcher_runtime::watch_application_catalog(
                    application_provider,
                    catalog_tx,
                ))
                .detach();
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                while let Ok(update) = catalog_rx.recv().await {
                    if cx.update(|cx| update_active_catalog(update, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();

            let (settings_tx, settings_rx) = async_channel::bounded(8);
            cx.background_executor()
                .spawn(rmac_launcher_runtime::watch_shell_settings(settings_tx))
                .detach();
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                while let Ok(update) = settings_rx.recv().await {
                    if cx.update(|cx| apply_settings_update(update, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();

            #[cfg(not(target_os = "linux"))]
            if std::env::args().any(|argument| argument == "--show") {
                route_shortcut(
                    rmac_shortcuts::Event::Activated {
                        id: rmac_shortcuts::ShortcutId("launcher".into()),
                        timestamp_ms: 1,
                    },
                    cx,
                );
            }
        });
}
