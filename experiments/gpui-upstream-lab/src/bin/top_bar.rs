#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::Local;
    use gpui::{
        div, layer_shell::*, point, prelude::*, px, rgba, App, Bounds, Context, DisplayId,
        FontWeight, Role, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
    };
    use gpui_platform::application;
    use rmac_gpui_upstream_lab::delay_until_next_minute;

    const BAR_HEIGHT: f32 = 32.0;
    const READY_FILE_ENV: &str = "RMAC_TOP_BAR_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_TOP_BAR_RENDER_COUNT_DIR";

    struct TopBar {
        display_id: u64,
        render_count: u64,
    }

    impl TopBar {
        fn new(display_id: DisplayId, cx: &mut Context<Self>) -> Self {
            let display_id = u64::from(display_id);
            cx.spawn(async move |this, cx| loop {
                let epoch_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                cx.background_executor()
                    .timer(delay_until_next_minute(epoch_millis))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            })
            .detach();
            Self {
                display_id,
                render_count: 0,
            }
        }
    }

    impl Render for TopBar {
        fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let now = Local::now();
            let clock = now.format("%a %b %-d  %-I:%M %p").to_string();
            let clock_label = now.format("%A, %B %-d, %-I:%M %p").to_string();

            div()
                .id(format!("top-bar-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("rmac top bar")
                .size_full()
                .flex()
                .items_center()
                .px_3()
                .bg(rgba(0xf4f5f6ed))
                .text_color(rgba(0x15171aff))
                .text_sm()
                .border_b_1()
                .border_color(rgba(0x00000024))
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("rmac"),
                )
                .child(
                    div()
                        .id(format!("clock-{}", self.display_id))
                        .role(Role::Time)
                        .aria_label(clock_label)
                        .flex_1()
                        .flex()
                        .justify_center()
                        .font_weight(FontWeight::MEDIUM)
                        .child(clock),
                )
                .child(div().flex_1())
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
            .unwrap_or_else(|error| panic!("open top-bar evidence {path:?}: {error}"));
        writeln!(file, "display={display_id} scale={scale}")
            .unwrap_or_else(|error| panic!("write top-bar evidence {path:?}: {error}"));
    }

    fn record_render_count(window: &Window, display_id: u64, render_count: u64) {
        let Some(directory) = env::var_os(RENDER_COUNT_DIR_ENV).map(PathBuf::from) else {
            return;
        };
        window.on_next_frame(move |_, _| {
            fs::create_dir_all(&directory).unwrap_or_else(|error| {
                panic!("create top-bar render evidence {directory:?}: {error}")
            });
            let path = directory.join(format!("{display_id}.count"));
            fs::write(&path, format!("{render_count}\n"))
                .unwrap_or_else(|error| panic!("write top-bar render count {path:?}: {error}"));
        });
    }

    fn open_top_bars(cx: &mut App) {
        for display in cx.displays() {
            let display_id = display.id();
            let width = display.bounds().size.width;
            let handle = cx
                .open_window(
                    WindowOptions {
                        titlebar: None,
                        focus: false,
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(0.), px(0.)),
                            size: Size::new(width, px(BAR_HEIGHT)),
                        })),
                        display_id: Some(display_id),
                        app_id: Some("dev.rmac.TopBar".to_owned()),
                        window_background: WindowBackgroundAppearance::Transparent,
                        kind: WindowKind::LayerShell(LayerShellOptions {
                            namespace: format!("rmac-top-bar-{}", u64::from(display_id)),
                            layer: Layer::Top,
                            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                            keyboard_interactivity: KeyboardInteractivity::None,
                            exclusive_zone: Some(px(BAR_HEIGHT)),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |_, cx| cx.new(|cx| TopBar::new(display_id, cx)),
                )
                .expect("open top-bar layer surface");
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                handle
                    .update(cx, |_, window, _| {
                        record_configured_surface(window, u64::from(display_id));
                    })
                    .expect("sample configured top-bar surface");
            })
            .detach();
        }
    }

    pub fn run() {
        application().run(|cx: &mut App| {
            cx.spawn(async move |cx| {
                // Wayland outputs arrive through the registry after the application callback
                // begins. Wait for that first round-trip before creating output-bound surfaces.
                loop {
                    if !cx.update(|cx| cx.displays().is_empty()) {
                        cx.update(open_top_bars);
                        break;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(10))
                        .await;
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
    eprintln!("top-bar requires Linux and: cargo run --features wayland --bin top-bar");
    std::process::exit(2);
}
