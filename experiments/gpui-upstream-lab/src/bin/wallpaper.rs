#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{
        div, layer_shell::*, linear_color_stop, linear_gradient, point, prelude::*, px, rgba,
        AnyWindowHandle, App, Bounds, Context, DisplayId, PlatformDisplay, Role, Size, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;

    const READY_FILE_ENV: &str = "RMAC_WALLPAPER_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_WALLPAPER_RENDER_COUNT_DIR";

    struct Wallpaper {
        display_id: u64,
        render_count: u64,
    }

    impl Wallpaper {
        fn new(display_id: DisplayId) -> Self {
            Self {
                display_id: u64::from(display_id),
                render_count: 0,
            }
        }
    }

    impl Render for Wallpaper {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let palette = rmac_wallpaper::DEFAULT_BUILT_IN.metadata().palette;
            div()
                .id(format!("wallpaper-{}", self.display_id))
                .role(Role::Image)
                .aria_label("Aurora wallpaper, original artwork by rmac")
                .relative()
                .size_full()
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
                )
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

    fn open_wallpaper(display: Rc<dyn PlatformDisplay>, cx: &mut App) -> AnyWindowHandle {
        let display_id = display.id();
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
                move |_, cx| cx.new(|_| Wallpaper::new(display_id)),
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
        application().run(|cx: &mut App| {
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
                match output_rx.recv().await {
                    Ok(mut desired) => loop {
                        for _ in 0..20 {
                            let complete = cx.update(|cx| {
                                tracker.reconcile(Some(&desired), cx, open_wallpaper);
                                tracker.len() == desired.len()
                            });
                            if complete {
                                break;
                            }
                            cx.background_executor()
                                .timer(Duration::from_millis(50))
                                .await;
                        }
                        let Ok(next) = output_rx.recv().await else {
                            break;
                        };
                        desired = next;
                    },
                    Err(_) => loop {
                        cx.update(|cx| tracker.reconcile(None, cx, open_wallpaper));
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                    },
                }
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
    eprintln!("wallpaper requires Linux and: cargo run --features wayland --bin wallpaper");
    std::process::exit(2);
}
