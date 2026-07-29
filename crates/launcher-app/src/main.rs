//! Session-owned, Spotlight-style launcher surface.

mod view_render;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use gpui::{point, Bounds};
use gpui::{
    px, size, AnyWindowHandle, App, AppContext as _, Application, BorrowAppContext as _,
    ClipboardItem, Context, Entity, Focusable as _, Global, SharedString, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions,
};
use gpui_component::Root;
use rmac_launcher::{ActivationMode, ResultId};
use rmac_launcher_runtime::{
    CatalogUpdate, Coordinator, KeyCommand, KeyEffect, Registry, SettingsUpdate, ShortcutEffect,
};
use rmac_launcher_system::{BackendError, FailureKind, Surface, SystemBackend};
use rmac_ui::{InputEvent, InputState};

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

#[derive(Clone)]
struct SurfaceBridge {
    clipboard: async_channel::Sender<String>,
}

impl Surface for SurfaceBridge {
    fn open_setting(&self, pane_id: &str) -> Result<(), BackendError> {
        if !rmac_launcher_providers::system_settings_entries()
            .iter()
            .any(|entry| entry.pane_id == pane_id)
        {
            return Err(BackendError::new(
                FailureKind::InvalidAction,
                "unknown Settings destination",
            ));
        }
        let executable = std::env::current_exe()
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))?
            .with_file_name("rmac-system-settings");
        Command::new(executable)
            .arg("--pane")
            .arg(pane_id)
            .spawn()
            .map(|_| ())
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))
    }

    fn copy_text(&self, text: &str) -> Result<(), BackendError> {
        self.clipboard.try_send(text.to_owned()).map_err(|_| {
            BackendError::new(FailureKind::Unavailable, "clipboard surface is unavailable")
        })
    }
}

struct LauncherView {
    token: u64,
    query: Entity<InputState>,
    coordinator: Coordinator,
    registry: Arc<Registry>,
    backend: Arc<SystemBackend<SurfaceBridge>>,
    settings_error: Option<SharedString>,
    was_active: bool,
}

struct OverlayEnvironment {
    token: u64,
    event: rmac_shortcuts::Event,
    registry: Arc<Registry>,
    settings: rmac_shell_settings::ShellSettings,
    settings_error: Option<SharedString>,
    clipboard: async_channel::Sender<String>,
}

impl LauncherView {
    fn new(environment: OverlayEnvironment, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let OverlayEnvironment {
            token,
            event,
            registry,
            settings,
            settings_error,
            clipboard,
        } = environment;
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Spotlight Search"));
        cx.subscribe(&query, |this, query, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = query.read(cx).value().to_string();
                if let Some(request) = this.coordinator.set_query(value) {
                    this.dispatch(request, cx);
                }
                cx.notify();
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();
        cx.on_release(move |_, cx| {
            if cx.has_global::<LauncherService>() {
                cx.update_global::<LauncherService, _>(|service, _| {
                    if service
                        .active
                        .as_ref()
                        .is_some_and(|active| active.token == token)
                    {
                        service.active = None;
                    }
                });
            }
        })
        .detach();

        let mut coordinator = Coordinator::new(registry.descriptors(), settings.providers.clone());
        let ShortcutEffect::Open(opened) = coordinator.handle_shortcut(&event) else {
            unreachable!("a fresh launcher surface starts from one launcher activation")
        };
        query.read(cx).focus_handle(cx).focus(window);
        let view = Self {
            token,
            query,
            coordinator,
            registry,
            backend: Arc::new(SystemBackend::new(SurfaceBridge { clipboard })),
            settings_error,
            was_active: false,
        };
        Self::spawn_dispatch(view.registry.clone(), opened.request, cx);
        view
    }

    fn spawn_dispatch(
        registry: Arc<Registry>,
        request: rmac_launcher::Request,
        cx: &mut Context<Self>,
    ) {
        let capacity = request.providers.len().max(1);
        let (sender, receiver) = async_channel::bounded(capacity);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let dispatch = registry.dispatch(request, sender);
            let consume = async {
                while let Ok(batch) = receiver.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            if this.coordinator.apply(batch) {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            };
            futures_util::join!(dispatch, consume);
        })
        .detach();
    }

    fn dispatch(&self, request: rmac_launcher::Request, cx: &mut Context<Self>) {
        Self::spawn_dispatch(self.registry.clone(), request, cx);
    }

    fn apply_catalog(&mut self, update: CatalogUpdate, cx: &mut Context<Self>) {
        let effect = self.coordinator.apply_catalog(update);
        if let Some(request) = effect.request {
            self.dispatch(request, cx);
        }
        if effect.visible {
            cx.notify();
        }
    }

    fn apply_environment(
        &mut self,
        registry: Arc<Registry>,
        settings: rmac_shell_settings::ShellSettings,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.registry = registry;
        self.settings_error = error;
        if let Some(request) = self
            .coordinator
            .set_environment(self.registry.descriptors(), settings.providers)
        {
            self.dispatch(request, cx);
        }
        cx.notify();
    }

    fn handle_shortcut(
        &mut self,
        event: &rmac_shortcuts::Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.coordinator.handle_shortcut(event),
            ShortcutEffect::Dismissed
        ) {
            self.dismiss(window, cx);
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.coordinator.handle_key(KeyCommand::Escape);
        if cx.has_global::<LauncherService>() {
            let token = self.token;
            cx.update_global::<LauncherService, _>(|service, _| {
                if service
                    .active
                    .as_ref()
                    .is_some_and(|active| active.token == token)
                {
                    service.active = None;
                }
            });
        }
        window.remove_window();
    }

    fn handle_key(&mut self, command: KeyCommand, window: &mut Window, cx: &mut Context<Self>) {
        let effect = self.coordinator.handle_key(command);
        self.apply_key_effect(effect, window, cx);
    }

    fn select_and_activate(
        &mut self,
        id: ResultId,
        mode: ActivationMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.coordinator.select(&id);
        let effect = self.coordinator.activate_selected(mode);
        self.apply_key_effect(effect, window, cx);
    }

    fn apply_key_effect(&mut self, effect: KeyEffect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            KeyEffect::None => {}
            KeyEffect::SelectionChanged => cx.notify(),
            KeyEffect::Dismissed => self.dismiss(window, cx),
            KeyEffect::Activate(activation) => {
                let backend = self.backend.clone();
                let window_handle = window.window_handle();
                cx.notify();
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    let result = rmac_launcher_system::execute(
                        activation.id,
                        &activation.action,
                        backend.as_ref(),
                    )
                    .await;
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            if this.coordinator.finish_activation(result) {
                                if !this.coordinator.snapshot().open {
                                    if cx.has_global::<LauncherService>() {
                                        let token = this.token;
                                        cx.update_global::<LauncherService, _>(|service, _| {
                                            if service
                                                .active
                                                .as_ref()
                                                .is_some_and(|active| active.token == token)
                                            {
                                                service.active = None;
                                            }
                                        });
                                    }
                                    window.remove_window();
                                } else {
                                    cx.notify();
                                }
                            }
                        });
                    });
                })
                .detach();
            }
        }
    }
}

fn build_registry(
    application_provider: &rmac_launcher_providers::ApplicationProvider,
    settings: &rmac_shell_settings::ShellSettings,
) -> Result<Arc<Registry>, SharedString> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    build_registry_with_home(application_provider, settings, home)
}

fn build_registry_with_home(
    application_provider: &rmac_launcher_providers::ApplicationProvider,
    settings: &rmac_shell_settings::ShellSettings,
    home: Option<PathBuf>,
) -> Result<Arc<Registry>, SharedString> {
    let mut providers: Vec<Arc<dyn rmac_launcher_providers::Provider>> = vec![
        Arc::new(application_provider.clone()),
        Arc::new(rmac_launcher_providers::SettingsProvider::system_settings()),
        Arc::new(rmac_launcher_providers::CalculatorProvider),
    ];
    if let Some(home) = home {
        let files = rmac_launcher_providers::FileProvider::scoped(
            home,
            &settings.spotlight,
            rmac_launcher_providers::SystemFileSearch,
        )
        .map_err(|_| SharedString::from("File-search scope is unavailable"))?;
        providers.push(Arc::new(files));
    }
    Registry::new(providers)
        .map(Arc::new)
        .map_err(|_| "Search provider registry is unavailable".into())
}

fn overlay_options(bounds: WindowBounds) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(bounds),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some("org.rmac.Launcher".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

#[cfg(not(target_os = "linux"))]
fn fallback_options(cx: &App) -> WindowOptions {
    overlay_options(WindowBounds::centered(size(px(WIDTH), px(HEIGHT)), cx))
}

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

fn route_existing(event: &rmac_shortcuts::Event, cx: &mut App) -> bool {
    let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let _ = cx.update_window(active.window, |_, window, cx| {
                view.update(cx, |view, cx| view.handle_shortcut(event, window, cx));
            });
            return true;
        }
        cx.update_global::<LauncherService, _>(|service, _| service.active = None);
    }
    false
}

fn open_launcher(event: rmac_shortcuts::Event, options: WindowOptions, cx: &mut App) {
    let (token, registry, settings, error, clipboard) =
        cx.update_global::<LauncherService, _>(|service, _| {
            service.next_overlay = service.next_overlay.wrapping_add(1).max(1);
            (
                service.next_overlay,
                service.registry.clone(),
                service.settings.clone(),
                service.settings_error.clone(),
                service.clipboard.clone(),
            )
        });
    let mut launcher = None;
    let handle = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            LauncherView::new(
                OverlayEnvironment {
                    token,
                    event,
                    registry,
                    settings,
                    settings_error: error,
                    clipboard,
                },
                window,
                cx,
            )
        });
        launcher = Some(view.downgrade());
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, launcher) {
        cx.update_global::<LauncherService, _>(|service, _| {
            service.active = Some(ActiveOverlay {
                token,
                view,
                window: handle.into(),
            });
        });
        cx.activate(true);
    }
}

#[cfg(not(target_os = "linux"))]
fn route_shortcut(event: rmac_shortcuts::Event, cx: &mut App) {
    if route_existing(&event, cx) {
        return;
    }
    open_launcher(event, fallback_options(cx), cx);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn route_activation(activation: rmac_shell_activation_runtime::Activation, cx: &mut App) {
    let (event, context) = activation.into_parts();
    if route_existing(&event, cx) {
        return;
    }
    let context = match context {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Launcher activation rejected: {error}");
            return;
        }
    };
    let description =
        match rmac_launcher::surface::plan_invocation(context.invocation(), context.compositor()) {
            Ok(description) => description,
            Err(error) => {
                eprintln!("Launcher surface plan rejected: {error}");
                return;
            }
        };
    let bounds =
        match context.centered_bounds(description.logical_width, description.logical_height) {
            Ok(bounds) => Bounds::new(
                point(px(bounds.x), px(bounds.y)),
                size(px(bounds.width), px(bounds.height)),
            ),
            Err(error) => {
                eprintln!("Launcher surface bounds rejected: {error}");
                return;
            }
        };
    open_launcher(event, overlay_options(WindowBounds::Windowed(bounds)), cx);
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
                    view.settings_error = Some(message);
                    cx.notify();
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

fn main() {
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
            let activation_done = cx.background_executor().spawn(async move {
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
                let watcher = async { activation_done.await.map_err(|error| error.to_string()) };
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

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_launcher::Category;

    #[test]
    fn built_in_registry_exposes_all_four_provider_categories() {
        let registry = build_registry_with_home(
            &rmac_launcher_providers::ApplicationProvider::default(),
            &rmac_shell_settings::ShellSettings::default(),
            Some("/home/test".into()),
        )
        .unwrap();
        let categories = registry
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.category)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            categories,
            std::collections::BTreeSet::from([
                Category::Applications,
                Category::Settings,
                Category::Calculator,
                Category::Files,
            ])
        );
    }

    #[test]
    fn settings_surface_rejects_hidden_destinations() {
        let (sender, _receiver) = async_channel::bounded(1);
        let surface = SurfaceBridge { clipboard: sender };
        assert_eq!(
            surface.open_setting("assistant").unwrap_err().kind,
            FailureKind::InvalidAction
        );
        assert_eq!(
            surface.open_setting("screen-time").unwrap_err().kind,
            FailureKind::InvalidAction
        );
    }
}
