#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;

    use gpui::{
        div, layer_shell::*, point, prelude::*, px, rgba, App, Bounds, Context, DisplayId,
        FontWeight, Role, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
    };
    use gpui_platform::application;

    const SURFACE_HEIGHT: f32 = 84.0;
    const READY_FILE_ENV: &str = "RMAC_DOCK_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_DOCK_RENDER_COUNT_DIR";
    const ITEMS: [DockItem; 6] = [
        DockItem::new("Files", "Fi", "rmac-files", 0x4a90e2ff),
        DockItem::new("Terminal", ">_", "rmac-terminal", 0x25282eff),
        DockItem::new("Notes", "N", "rmac-notes", 0xf5c94cff),
        DockItem::new("Text Editor", "Tx", "rmac-text-editor", 0x5e72e4ff),
        DockItem::new("System Monitor", "◫", "rmac-system-monitor", 0x34a875ff),
        DockItem::new("Settings", "⚙", "rmac-system-settings", 0x8d929aff),
    ];

    #[derive(Clone, Copy)]
    struct DockItem {
        name: &'static str,
        mark: &'static str,
        program: &'static str,
        color: u32,
    }

    impl DockItem {
        const fn new(
            name: &'static str,
            mark: &'static str,
            program: &'static str,
            color: u32,
        ) -> Self {
            Self {
                name,
                mark,
                program,
                color,
            }
        }
    }

    struct Dock {
        display_id: u64,
        render_count: u64,
    }

    impl Dock {
        fn new(display_id: DisplayId) -> Self {
            Self {
                display_id: u64::from(display_id),
                render_count: 0,
            }
        }
    }

    impl Render for Dock {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
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
                        .rounded(px(20.0))
                        .bg(rgba(0xf4f5f6d8))
                        .border_1()
                        .border_color(rgba(0xffffff80))
                        .shadow_lg()
                        .children(ITEMS.into_iter().enumerate().map(|(index, item)| {
                            div()
                                .id(format!("dock-item-{}-{index}", self.display_id))
                                .role(Role::Button)
                                .aria_label(format!("Open {}", item.name))
                                .w(px(52.0))
                                .h(px(52.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(13.0))
                                .bg(rgba(item.color))
                                .text_color(rgba(0xffffffff))
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.88))
                                .on_click(move |_, _, _| {
                                    if let Err(error) = Command::new(item.program).spawn() {
                                        eprintln!("could not open {}: {error}", item.name);
                                    }
                                })
                                .child(item.mark)
                        })),
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

    fn open_docks(cx: &mut App) {
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
                        window_background: WindowBackgroundAppearance::Transparent,
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
                    move |_, cx| cx.new(|_| Dock::new(display_id)),
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
            cx.spawn(async move |cx| loop {
                if !cx.update(|cx| cx.displays().is_empty()) {
                    cx.update(open_docks);
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
