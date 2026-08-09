#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgba, App, Bounds, Context, DisplayId,
        Entity, FontWeight, Role, Size, Window, WindowBackgroundAppearance, WindowBounds,
        WindowKind, WindowOptions,
    };
    use gpui_platform::application;

    const SURFACE_HEIGHT: f32 = 88.0;
    const READY_FILE_ENV: &str = "RMAC_DOCK_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_DOCK_RENDER_COUNT_DIR";
    static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

    enum SourceEvent {
        Settings(Result<rmac_shell_settings::ShellSettings, String>),
        Catalog(Result<Vec<rmac_apps::Application>, String>),
    }

    struct DockStatus {
        settings: rmac_shell_settings::ShellSettings,
        catalog: Vec<rmac_apps::Application>,
        compositor: rmac_compositor::State,
    }

    impl DockStatus {
        fn new(
            compositor: async_channel::Receiver<rmac_compositor::Event>,
            sources: async_channel::Receiver<SourceEvent>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| {
                while let Ok(event) = compositor.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            if this.compositor.apply(event).visible {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            cx.spawn(async move |this, cx| {
                while let Ok(event) = sources.recv().await {
                    if this
                        .update(cx, |this, cx| match event {
                            SourceEvent::Settings(Ok(settings)) if this.settings != settings => {
                                this.settings = settings;
                                cx.notify();
                            }
                            SourceEvent::Catalog(Ok(catalog)) if this.catalog != catalog => {
                                this.catalog = catalog;
                                cx.notify();
                            }
                            SourceEvent::Settings(Err(detail)) => {
                                eprintln!("Dock settings unavailable: {detail}");
                            }
                            SourceEvent::Catalog(Err(detail)) => {
                                eprintln!("Dock catalog unavailable: {detail}");
                            }
                            SourceEvent::Settings(Ok(_)) | SourceEvent::Catalog(Ok(_)) => {}
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            Self {
                settings: rmac_shell_settings::ShellSettings::default(),
                catalog: Vec::new(),
                compositor: rmac_compositor::State::default(),
            }
        }

        fn model(&self) -> rmac_dock::Model {
            rmac_dock::Model::build(
                &self.settings.pinned_apps,
                &self.settings.dock,
                &self.catalog,
                &self.compositor.snapshot(),
            )
        }
    }

    struct Dock {
        display_id: u64,
        render_count: u64,
        status: Entity<DockStatus>,
    }

    impl Dock {
        fn new(display_id: DisplayId, status: Entity<DockStatus>, cx: &mut Context<Self>) -> Self {
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            Self {
                display_id: u64::from(display_id),
                render_count: 0,
                status,
            }
        }
    }

    impl Render for Dock {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let model = self.status.read(cx).model();
            let entries = rmac_dock::presentation::ShelfContent::project(&model).applications;
            div()
                .id(format!("dock-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("rmac Dock")
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .pb_2()
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .p_2()
                        .rounded(px(22.0))
                        .bg(rgba(0xe7ecf18c))
                        .border_1()
                        .border_color(rgba(0xffffffb8))
                        .shadow_lg()
                        .children(entries.into_iter().enumerate().map(|(index, entry)| {
                            let app_id = match &entry.id {
                                rmac_dock::presentation::EntryId::Application(app_id) => {
                                    app_id.clone()
                                }
                                _ => unreachable!("the application group contains only apps"),
                            };
                            let activation = model.activate(&app_id);
                            let available = entry.enabled;
                            let actionable = matches!(
                                activation,
                                rmac_dock::Activation::Launch { .. }
                                    | rmac_dock::Activation::FocusWindow(_)
                            );
                            let active = entry.activity
                                == rmac_dock::presentation::ActivityIndicator::Active;
                            let running =
                                entry.activity != rmac_dock::presentation::ActivityIndicator::None;
                            let icon_path = item_icon_path(&entry.icon, &app_id);
                            let mut item = div()
                                .id(format!("dock-item-{}-{index}", self.display_id))
                                .role(Role::Button)
                                .aria_label(entry.accessible_label)
                                .relative()
                                .w(px(56.0))
                                .h(px(56.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(13.0))
                                .bg(rgba(if icon_path.is_some() {
                                    0x00000000
                                } else {
                                    item_color(&app_id, available)
                                }))
                                .text_color(rgba(0xffffffff))
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .opacity(if available { 1.0 } else { 0.58 });
                            if let Some(path) = icon_path {
                                item =
                                    item.child(img(path).w(px(54.0)).h(px(54.0)).rounded(px(14.0)));
                            } else {
                                item = item.child(item_mark(&entry.label));
                            }
                            if actionable {
                                item = item
                                    .cursor_pointer()
                                    .hover(|style| style.opacity(0.88))
                                    .on_click(move |_, _, cx| {
                                        dispatch_activation(activation.clone(), cx);
                                    });
                            }
                            if running {
                                item = item.child(
                                    div()
                                        .absolute()
                                        .bottom(px(-7.0))
                                        .w(px(if active { 7.0 } else { 5.0 }))
                                        .h(px(if active { 7.0 } else { 5.0 }))
                                        .rounded_full()
                                        .bg(rgba(if active { 0x2563ebff } else { 0x60656dff })),
                                );
                            }
                            if entry.urgent {
                                item = item.child(
                                    div()
                                        .absolute()
                                        .top(px(-3.0))
                                        .right(px(-3.0))
                                        .w(px(10.0))
                                        .h(px(10.0))
                                        .rounded_full()
                                        .bg(rgba(0xff3b30ff)),
                                );
                            }
                            item
                        })),
                )
        }
    }

    fn item_mark(label: &str) -> String {
        let mark = label
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect::<String>();
        if mark.is_empty() {
            "•".into()
        } else {
            mark
        }
    }

    fn item_icon_path(icon: &rmac_dock::presentation::Icon, app_id: &str) -> Option<PathBuf> {
        match icon {
            rmac_dock::presentation::Icon::File(path) if path.is_file() => Some(path.clone()),
            rmac_dock::presentation::Icon::File(_) | rmac_dock::presentation::Icon::Builtin(_) => {
                first_party_icon_path(app_id)
            }
        }
    }

    fn first_party_icon_path(app_id: &str) -> Option<PathBuf> {
        let identity = app_id.trim_end_matches(".desktop");
        let file = match identity {
            rmac_apps::identity::FILES => "org.rmac.Files.svg",
            rmac_apps::identity::TERMINAL => "org.rmac.Terminal.svg",
            rmac_apps::identity::NOTES => "org.rmac.Notes.svg",
            rmac_apps::identity::TEXT_EDITOR => "org.rmac.TextEditor.svg",
            rmac_apps::identity::SYSTEM_MONITOR => "org.rmac.SystemMonitor.svg",
            rmac_apps::identity::SYSTEM_SETTINGS => "org.rmac.SystemSettings.svg",
            _ => return None,
        };
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../packaging/rmac-apps/icons")
                .join(file),
        )
    }

    fn item_color(app_id: &str, enabled: bool) -> u32 {
        if !enabled {
            return 0x8d929aff;
        }
        match app_id.trim_end_matches(".desktop") {
            rmac_apps::identity::FILES => 0x4a90e2ff,
            rmac_apps::identity::TERMINAL => 0x25282eff,
            rmac_apps::identity::NOTES => 0xf5c94cff,
            rmac_apps::identity::TEXT_EDITOR => 0x5e72e4ff,
            rmac_apps::identity::SYSTEM_MONITOR => 0x34a875ff,
            rmac_apps::identity::SYSTEM_SETTINGS => 0x8d929aff,
            _ => {
                let hash = app_id.bytes().fold(0x811c9dc5u32, |hash, byte| {
                    hash.wrapping_mul(0x01000193) ^ u32::from(byte)
                });
                ((hash & 0xbfbfbf) | 0x303030) << 8 | 0xff
            }
        }
    }

    fn dispatch_activation(activation: rmac_dock::Activation, cx: &mut App) {
        match activation {
            rmac_dock::Activation::Launch { spec, .. } => {
                cx.background_executor()
                    .spawn(async move {
                        if blocking::unblock(move || rmac_apps::launch(&spec))
                            .await
                            .is_err()
                        {
                            eprintln!("could not launch the selected Dock application");
                        }
                    })
                    .detach();
            }
            rmac_dock::Activation::FocusWindow(window) => {
                let Ok(previous) =
                    NEXT_ACTIVATION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                        current.checked_add(1)
                    })
                else {
                    eprintln!("could not focus Dock application: activation IDs exhausted");
                    return;
                };
                let request = rmac_compositor::ActionRequest {
                    id: rmac_compositor::ActivationId(previous + 1),
                    action: rmac_compositor::Action::FocusWindow { window },
                };
                cx.background_executor()
                    .spawn(async move {
                        if rmac_compositor_niri::execute(request).await.result.is_err() {
                            eprintln!("could not focus the selected Dock application");
                        }
                    })
                    .detach();
            }
            rmac_dock::Activation::NoAction | rmac_dock::Activation::Unavailable { .. } => {}
        }
    }

    async fn watch_settings(sender: async_channel::Sender<SourceEvent>) {
        let setup = blocking::unblock(|| {
            let store = rmac_shell_settings::ShellSettingsStore::from_environment()
                .map_err(|_| "the shell settings authority could not start".to_owned())?;
            let settings = store
                .load()
                .map_err(|_| "the shell settings could not be loaded".to_owned())?
                .settings;
            let watcher = store
                .watch()
                .map_err(|_| "the shell settings watcher could not start".to_owned())?;
            Ok::<_, String>((store, watcher, settings))
        })
        .await;
        let (mut store, watcher, settings) = match setup {
            Ok(setup) => setup,
            Err(detail) => {
                let _ = sender.send(SourceEvent::Settings(Err(detail))).await;
                return;
            }
        };
        if sender
            .send(SourceEvent::Settings(Ok(settings)))
            .await
            .is_err()
        {
            return;
        }
        while watcher.recv().await.is_ok() {
            let (returned, result) = blocking::unblock(move || {
                let result = store
                    .load()
                    .map(|snapshot| snapshot.settings)
                    .map_err(|_| "the changed shell settings could not be loaded".to_owned());
                (store, result)
            })
            .await;
            store = returned;
            if sender.send(SourceEvent::Settings(result)).await.is_err() {
                return;
            }
        }
    }

    async fn watch_catalog(sender: async_channel::Sender<SourceEvent>) {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let setup = blocking::unblock(move || {
            let catalog = rmac_apps::discover()
                .map_err(|_| "the application catalog could not be loaded".to_owned())?;
            let watcher = rmac_apps::watch_catalog(move || {
                let _ = changed_tx.try_send(());
            })
            .map_err(|_| "the application catalog watcher could not start".to_owned())?;
            Ok::<_, String>((watcher, catalog))
        })
        .await;
        let (_watcher, catalog) = match setup {
            Ok(setup) => setup,
            Err(detail) => {
                let _ = sender.send(SourceEvent::Catalog(Err(detail))).await;
                return;
            }
        };
        if sender
            .send(SourceEvent::Catalog(Ok(catalog)))
            .await
            .is_err()
        {
            return;
        }
        while changed_rx.recv().await.is_ok() {
            let result = blocking::unblock(|| {
                rmac_apps::discover()
                    .map_err(|_| "the changed application catalog could not be loaded".to_owned())
            })
            .await;
            if sender.send(SourceEvent::Catalog(result)).await.is_err() {
                return;
            }
        }
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
            .unwrap_or_else(|error| panic!("open Dock evidence {path:?}: {error}"));
        writeln!(file, "display={display_id} scale={scale}")
            .unwrap_or_else(|error| panic!("write Dock evidence {path:?}: {error}"));
    }

    fn record_render_count(window: &Window, display_id: u64, render_count: u64) {
        let Some(directory) = env::var_os(RENDER_COUNT_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        window.on_next_frame(move |_, _| {
            fs::create_dir_all(&directory)
                .unwrap_or_else(|error| panic!("create Dock evidence {directory:?}: {error}"));
            let path = directory.join(format!("{display_id}.count"));
            fs::write(&path, format!("{render_count}\n"))
                .unwrap_or_else(|error| panic!("write Dock render count {path:?}: {error}"));
        });
    }

    fn open_docks(cx: &mut App, status: Entity<DockStatus>) {
        for display in cx.displays() {
            let display_id = display.id();
            let width = display.bounds().size.width;
            let handle = cx
                .open_window(
                    WindowOptions {
                        titlebar: None,
                        focus: false,
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(0.0), px(0.0)),
                            size: Size::new(width, px(SURFACE_HEIGHT)),
                        })),
                        display_id: Some(display_id),
                        app_id: Some("dev.rmac.Dock".to_owned()),
                        window_background: WindowBackgroundAppearance::Blurred,
                        kind: WindowKind::LayerShell(LayerShellOptions {
                            namespace: format!("rmac-dock-{}", u64::from(display_id)),
                            layer: Layer::Top,
                            anchor: Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
                            keyboard_interactivity: KeyboardInteractivity::None,
                            exclusive_zone: Some(px(SURFACE_HEIGHT)),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    {
                        let status = status.clone();
                        move |_, cx| cx.new(|cx| Dock::new(display_id, status, cx))
                    },
                )
                .expect("open Dock layer surface");
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                handle
                    .update(cx, |_, window, _| {
                        record_configured_surface(window, u64::from(display_id));
                    })
                    .expect("sample configured Dock surface");
            })
            .detach();
        }
    }

    pub fn run() {
        application().run(|cx: &mut App| {
            let (compositor_tx, compositor_rx) = async_channel::bounded(64);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                        eprintln!("Dock compositor watcher stopped: {error}");
                    }
                })
                .detach();
            let (source_tx, source_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(watch_settings(source_tx.clone()))
                .detach();
            cx.background_executor()
                .spawn(watch_catalog(source_tx))
                .detach();
            let status = cx.new(|cx| DockStatus::new(compositor_rx, source_rx, cx));
            cx.spawn(async move |cx| loop {
                if !cx.update(|cx| cx.displays().is_empty()) {
                    cx.update(|cx| open_docks(cx, status));
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            })
            .detach();
        });
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("Dock requires Linux and: cargo run --features wayland --bin dock");
    std::process::exit(2);
}
