#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px, rgba,
        AnyWindowHandle, App, Bounds, Context, DisplayId, Entity, MouseButton, PlatformDisplay,
        QuitMode, RenderImage, Role, Size, Window, WindowBackgroundAppearance, WindowBounds,
        WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use uuid::Uuid;

    const READY_FILE_ENV: &str = "RMAC_WALLPAPER_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_WALLPAPER_RENDER_COUNT_DIR";
    static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone)]
    struct PreparedSurface {
        image: Arc<RenderImage>,
        layout: rmac_wallpaper::Layout,
    }

    enum PreparedUpdate {
        Render(std::collections::BTreeMap<Uuid, PreparedSurface>),
        Health(rmac_wallpaper_runtime::HealthSnapshot),
    }

    struct WallpaperStatus {
        surfaces: std::collections::BTreeMap<Uuid, PreparedSurface>,
        health: rmac_wallpaper_runtime::HealthSnapshot,
        compositor: rmac_compositor::State,
    }

    impl WallpaperStatus {
        fn new(receiver: async_channel::Receiver<PreparedUpdate>, cx: &mut Context<Self>) -> Self {
            cx.spawn(async move |this, cx| {
                while let Ok(update) = receiver.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            match update {
                                PreparedUpdate::Render(surfaces) => this.surfaces = surfaces,
                                PreparedUpdate::Health(health) => this.health = health,
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
            Self {
                surfaces: std::collections::BTreeMap::new(),
                health: rmac_wallpaper_runtime::HealthSnapshot::default(),
                compositor: rmac_compositor::State::default(),
            }
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

    struct Wallpaper {
        display_id: u64,
        display_uuid: Uuid,
        render_count: u64,
        status: Entity<WallpaperStatus>,
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
                display_uuid,
                render_count: 0,
                status,
            }
        }

        fn dismiss_app_drawer(&self, cx: &mut App) {
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.dismiss_app_drawer(cx)),
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
                        linear_color_stop(rgba((palette[2] << 8) | 0x00), 0.72),
                    )
                    .color_space(gpui::ColorSpace::Oklab)),
                )
                .child(
                    div().absolute().inset_0().bg(linear_gradient(
                        315.0,
                        linear_color_stop(rgba((palette[3] << 8) | 0x00), 0.28),
                        linear_color_stop(rgba((palette[3] << 8) | 0xb8), 1.0),
                    )
                    .color_space(gpui::ColorSpace::Oklab)),
                );
            if let Some(surface) = surface {
                root = root
                    .child(div().absolute().inset_0().bg(rgba(0x1e1e20ff)))
                    .children(render_surface(surface));
            }
            root
        }
    }

    fn render_surface(surface: PreparedSurface) -> Vec<gpui::AnyElement> {
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
        let (prepared_tx, prepared_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_wallpaper_runtime::watch(runtime_tx).await {
                    eprintln!("wallpaper runtime stopped: {error}");
                }
            })
            .detach();
        cx.background_executor()
            .spawn(async move {
                while let Ok(update) = runtime_rx.recv().await {
                    let prepared = match update {
                        rmac_wallpaper_runtime::Update::Render {
                            rasterized, health, ..
                        } => {
                            let surfaces = blocking::unblock(move || {
                                rasterized
                                    .surfaces
                                    .into_iter()
                                    .filter_map(prepare_surface)
                                    .collect()
                            })
                            .await;
                            let _ = prepared_tx.send(PreparedUpdate::Health(health)).await;
                            PreparedUpdate::Render(surfaces)
                        }
                        rmac_wallpaper_runtime::Update::Health(health) => {
                            PreparedUpdate::Health(health)
                        }
                    };
                    if prepared_tx.send(prepared).await.is_err() {
                        break;
                    }
                }
            })
            .detach();
        let status = cx.new(|cx| WallpaperStatus::new(prepared_rx, cx));
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
                if compositor_status
                    .update(cx, |status, _| {
                        status.compositor.apply(event);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        status
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
                        keyboard_interactivity: KeyboardInteractivity::None,
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
        let app = application().with_quit_mode(QuitMode::Explicit);
        app.run(|cx: &mut App| {
            let status = start_status(cx);
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_gpui_upstream_lab::output_surfaces::watch_enabled(output_tx).await
                    {
                        eprintln!("wallpaper output watcher unavailable: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx| {
                let mut tracker = rmac_gpui_upstream_lab::output_surfaces::Tracker::default();
                let mut removed_outputs = std::collections::BTreeSet::new();
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
        previous: &std::collections::BTreeSet<Uuid>,
        current: &std::collections::BTreeSet<Uuid>,
        removed: &mut std::collections::BTreeSet<Uuid>,
    ) {
        if rmac_gpui_upstream_lab::output_reappeared(previous, current, removed) {
            std::process::exit(rmac_gpui_upstream_lab::WAYLAND_OUTPUT_RESTART_EXIT_CODE);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("wallpaper requires Linux and: cargo run --features wayland --bin wallpaper");
    std::process::exit(2);
}
