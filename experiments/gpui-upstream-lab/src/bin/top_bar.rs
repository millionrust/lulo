#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;
    use std::process::Command;
    use std::rc::Rc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use chrono::Local;
    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgba, AnyWindowHandle, App, Bounds,
        Context, DisplayId, Entity, FontWeight, PlatformDisplay, Role, Size, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_gpui_upstream_lab::{
        delay_until_next_clock_tick, top_bar_active_app_name, top_bar_clock_pattern,
        top_bar_indicator_labels, top_bar_workspace_label, TopBarIndicatorKind,
    };

    const BAR_HEIGHT: f32 = 28.0;
    const READY_FILE_ENV: &str = "RMAC_TOP_BAR_READY_FILE";
    const RENDER_COUNT_DIR_ENV: &str = "RMAC_TOP_BAR_RENDER_COUNT_DIR";

    struct ShellStatus {
        update: rmac_shell_runtime::Update,
    }

    impl ShellStatus {
        fn new(
            receiver: async_channel::Receiver<rmac_shell_runtime::Update>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| loop {
                let show_seconds = this
                    .read_with(cx, |this, _| this.update.snapshot.status.clock.show_seconds)
                    .unwrap_or(false);
                let epoch_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let update = receiver.recv().fuse();
                let tick = cx
                    .background_executor()
                    .timer(delay_until_next_clock_tick(epoch_millis, show_seconds))
                    .fuse();
                futures_util::pin_mut!(update, tick);
                futures_util::select! {
                    update = update => {
                        let Ok(update) = update else { break };
                        let visible = update.visible;
                        if this.update(cx, |this, cx| {
                            this.update = update;
                            if visible {
                                cx.notify();
                            }
                        }).is_err() {
                            break;
                        }
                    }
                    _ = tick => {
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }
            })
            .detach();
            Self {
                update: rmac_shell_runtime::Update::default(),
            }
        }
    }

    struct TopBar {
        display_id: u64,
        render_count: u64,
        status: Entity<ShellStatus>,
    }

    impl TopBar {
        fn new(display_id: DisplayId, status: Entity<ShellStatus>, cx: &mut Context<Self>) -> Self {
            let display_id = u64::from(display_id);
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            Self {
                display_id,
                render_count: 0,
                status,
            }
        }
    }

    impl Render for TopBar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count = self.render_count.saturating_add(1);
            record_render_count(window, self.display_id, self.render_count);
            let now = Local::now();
            let snapshot = &self.status.read(cx).update.snapshot.status;
            let clock = now
                .format(top_bar_clock_pattern(&snapshot.clock))
                .to_string();
            let clock_label = clock.clone();
            let active_app = top_bar_active_app_name(snapshot);
            let workspace = top_bar_workspace_label(snapshot);
            let indicators = top_bar_indicator_labels(snapshot);

            div()
                .id(format!("top-bar-{}", self.display_id))
                .role(Role::Toolbar)
                .aria_label("rmac top bar")
                .size_full()
                .flex()
                .items_center()
                .px_2()
                .bg(rgba(0x10151d52))
                .text_color(rgba(0xf7f8faff))
                .text_sm()
                .border_b_1()
                .border_color(rgba(0xffffff25))
                .shadow_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .child(
                            div()
                                .id(format!("desktop-mark-{}", self.display_id))
                                .w(px(18.0))
                                .h(px(18.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .font_weight(FontWeight::BOLD)
                                .aria_label("rmac desktop")
                                .child("r"),
                        )
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(active_app))
                        .children(workspace.map(|workspace| {
                            div()
                                .text_color(rgba(0xf7f8faaa))
                                .aria_label(format!("Workspace {workspace}"))
                                .child(workspace)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .justify_end()
                        .children(
                            indicators
                                .into_iter()
                                .filter(|indicator| {
                                    indicator.kind != TopBarIndicatorKind::Notifications
                                })
                                .enumerate()
                                .map(|(index, indicator)| {
                                    let icon = indicator_icon_path(indicator.kind);
                                    let mut item = div()
                                        .id(format!("status-{}-{index}", self.display_id))
                                        .role(Role::Button)
                                        .aria_label(indicator.accessible)
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_1()
                                        .rounded(px(5.0))
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgba(0xffffff22)))
                                        .on_click(|_, _, cx| {
                                            dispatch_shortcut("quick-settings", cx)
                                        })
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(img(icon).w(px(14.0)).h(px(14.0)));
                                    if !indicator.visible.is_empty() {
                                        item = item.child(indicator.visible);
                                    }
                                    item
                                }),
                        )
                        .child(
                            div()
                                .id(format!("spotlight-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Spotlight")
                                .w(px(22.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("launcher", cx))
                                .child(
                                    img(shell_icon_path("spotlight.svg"))
                                        .w(px(14.0))
                                        .h(px(14.0)),
                                ),
                        )
                        .child(
                            div()
                                .id(format!("control-center-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label("Control Center")
                                .w(px(22.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("quick-settings", cx))
                                .child(
                                    img(shell_icon_path("control-center.svg"))
                                        .w(px(15.0))
                                        .h(px(15.0)),
                                ),
                        )
                        .child(
                            div()
                                .id(format!("clock-{}", self.display_id))
                                .role(Role::Button)
                                .aria_label(format!(
                                    "Date and time: {clock_label}. Open Notification Center"
                                ))
                                .px_1()
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(0xffffff22)))
                                .on_click(|_, _, cx| dispatch_shortcut("notification-center", cx))
                                .font_weight(FontWeight::MEDIUM)
                                .child(clock),
                        ),
                )
        }
    }

    fn indicator_icon_path(kind: TopBarIndicatorKind) -> PathBuf {
        let file = match kind {
            TopBarIndicatorKind::Focus => "focus.svg",
            TopBarIndicatorKind::Vpn => "vpn.svg",
            TopBarIndicatorKind::Network => "wifi.svg",
            TopBarIndicatorKind::Bluetooth => "bluetooth.svg",
            TopBarIndicatorKind::Sound => "sound.svg",
            TopBarIndicatorKind::Battery => "battery.svg",
            TopBarIndicatorKind::Notifications => "notifications.svg",
        };
        shell_icon_path(file)
    }

    fn shell_icon_path(file: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/status")
            .join(file)
    }

    fn dispatch_shortcut(shortcut: &'static str, cx: &mut App) {
        let local = env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".local/libexec/rmac/rmac-shortcut-dispatch"));
        let dispatcher = local
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/usr/libexec/rmac/rmac-shortcut-dispatch"));
        cx.background_executor()
            .spawn(async move {
                let result = blocking::unblock(move || {
                    Command::new(dispatcher).arg(shortcut).spawn().map(|_| ())
                })
                .await;
                if result.is_err() {
                    eprintln!("could not open {shortcut}");
                }
            })
            .detach();
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

    fn start_status(cx: &mut App) -> Entity<ShellStatus> {
        let (status_tx, status_rx) = async_channel::bounded(16);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = rmac_shell_runtime::watch(status_tx).await {
                    eprintln!("shell status runtime stopped: {error}");
                }
            })
            .detach();
        cx.new(|cx| ShellStatus::new(status_rx, cx))
    }

    fn open_top_bar(
        display: Rc<dyn PlatformDisplay>,
        status: Entity<ShellStatus>,
        cx: &mut App,
    ) -> AnyWindowHandle {
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
                    window_background: WindowBackgroundAppearance::Blurred,
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
                {
                    let status = status.clone();
                    move |_, cx| cx.new(|cx| TopBar::new(display_id, status, cx))
                },
            )
            .expect("open top-bar layer surface");
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
            let status = start_status(cx);
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_gpui_upstream_lab::output_surfaces::watch_enabled(output_tx).await
                    {
                        eprintln!("top-bar output watcher unavailable: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx| {
                let mut tracker = rmac_gpui_upstream_lab::output_surfaces::Tracker::default();
                match output_rx.recv().await {
                    Ok(mut desired) => loop {
                        for _ in 0..20 {
                            let complete = cx.update(|cx| {
                                tracker.reconcile(Some(&desired), cx, |display, cx| {
                                    open_top_bar(display, status.clone(), cx)
                                });
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
                        cx.update(|cx| {
                            tracker.reconcile(None, cx, |display, cx| {
                                open_top_bar(display, status.clone(), cx)
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
