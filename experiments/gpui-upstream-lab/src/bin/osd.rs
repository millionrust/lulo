#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::collections::{BTreeSet, HashMap};
    use std::env;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use futures_util::FutureExt as _;
    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, relative, rgba, AnyWindowHandle, App,
        Bounds, Context, DisplayId, Entity, FontWeight, PlatformDisplay, QuitMode, Role, Size,
        Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;
    use rmac_osd::{Kind, Presentation};
    use uuid::Uuid;

    const SURFACE_WIDTH: f32 = 304.0;
    const SURFACE_HEIGHT: f32 = 74.0;
    const HIDDEN_SIZE: f32 = 1.0;
    const HIDE_DELAY: Duration = Duration::from_millis(1_600);

    struct VisiblePresentation {
        value: Presentation,
        output: Option<Uuid>,
        generation: u64,
    }

    struct OsdStatus {
        compositor: rmac_compositor::State,
        active_output: Option<Uuid>,
        fallback_output: Option<Uuid>,
        visible: Option<VisiblePresentation>,
        generation: u64,
    }

    impl OsdStatus {
        fn new(
            updates: async_channel::Receiver<Presentation>,
            compositor: async_channel::Receiver<rmac_compositor::Event>,
            cx: &mut Context<Self>,
        ) -> Self {
            cx.spawn(async move |this, cx| {
                while let Ok(event) = compositor.recv().await {
                    if this
                        .update(cx, |this, _| {
                            this.compositor.apply(event);
                            let snapshot = this.compositor.snapshot();
                            this.active_output = snapshot
                                .workspaces
                                .iter()
                                .find(|workspace| workspace.focused)
                                .or_else(|| {
                                    snapshot
                                        .workspaces
                                        .iter()
                                        .find(|workspace| workspace.active)
                                })
                                .and_then(|workspace| workspace.output.as_ref())
                                .map(rmac_gpui_upstream_lab::stable_output_uuid);
                            this.fallback_output = snapshot
                                .outputs
                                .iter()
                                .filter(|output| output.enabled())
                                .map(|output| {
                                    rmac_gpui_upstream_lab::stable_output_uuid(&output.id)
                                })
                                .min();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            cx.spawn(async move |this, cx| {
                let Ok(mut value) = updates.recv().await else {
                    return;
                };
                loop {
                    let generation = match this.update(cx, |this, cx| {
                        this.generation = this.generation.saturating_add(1);
                        let generation = this.generation;
                        this.visible = Some(VisiblePresentation {
                            value,
                            output: this.active_output.or(this.fallback_output),
                            generation,
                        });
                        cx.notify();
                        generation
                    }) {
                        Ok(generation) => generation,
                        Err(_) => return,
                    };

                    let next = updates.recv().fuse();
                    let hide = cx.background_executor().timer(HIDE_DELAY).fuse();
                    futures_util::pin_mut!(next, hide);
                    futures_util::select_biased! {
                        next = next => match next {
                            Ok(next) => {
                                value = next;
                                continue;
                            }
                            Err(_) => return,
                        },
                        _ = hide => {
                            if this
                                .update(cx, |this, cx| {
                                    if this
                                        .visible
                                        .as_ref()
                                        .is_some_and(|visible| visible.generation == generation)
                                    {
                                        this.visible = None;
                                        cx.notify();
                                    }
                                })
                                .is_err()
                            {
                                return;
                            }
                            let Ok(next) = updates.recv().await else {
                                return;
                            };
                            value = next;
                        }
                    }
                }
            })
            .detach();
            Self {
                compositor: rmac_compositor::State::default(),
                active_output: None,
                fallback_output: None,
                visible: None,
                generation: 0,
            }
        }
    }

    struct OsdSurface {
        output: Uuid,
        status: Entity<OsdStatus>,
        expanded: bool,
    }

    impl OsdSurface {
        fn new(output: Uuid, status: Entity<OsdStatus>, cx: &mut Context<Self>) -> Self {
            cx.observe(&status, |_, _, cx| cx.notify()).detach();
            Self {
                output,
                status,
                expanded: false,
            }
        }
    }

    impl Render for OsdSurface {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            window.set_input_region(Some(&[]));
            let presentation = self.status.read(cx).visible.as_ref().and_then(|visible| {
                (visible.output == Some(self.output)).then(|| visible.value.clone())
            });
            let expanded = presentation.is_some();
            if expanded != self.expanded {
                self.expanded = expanded;
                let size = if expanded {
                    Size::new(px(SURFACE_WIDTH), px(SURFACE_HEIGHT))
                } else {
                    Size::new(px(HIDDEN_SIZE), px(HIDDEN_SIZE))
                };
                window.resize(size);
            }

            let root = div().size_full();
            let Some(presentation) = presentation else {
                return root;
            };
            let (low_icon, high_icon) = icon_pair(&presentation);
            let visible_level = presentation.visible_level();
            let fraction = f32::from(visible_level) / 100.0;
            let label = presentation.accessible_label();
            let ticks = (0..16).fold(
                div().flex().items_center().justify_between().w_full(),
                |row, _| row.child(div().size(px(2.5)).rounded_full().bg(rgba(0xffffffb5))),
            );
            root.child(
                div()
                    .id(format!("system-osd-{}", self.output))
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap_1()
                    .px_4()
                    .py_2()
                    .rounded(px(28.0))
                    .bg(rgba(0x18202b9c))
                    .border_1()
                    .border_color(rgba(0xffffff26))
                    .role(Role::Status)
                    .aria_label(label)
                    .child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(0xffffffff))
                            .child(presentation.title),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .child(img(low_icon).size(px(18.0)))
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(
                                        div()
                                            .relative()
                                            .w_full()
                                            .h(px(7.0))
                                            .rounded_full()
                                            .bg(rgba(0xffffff36))
                                            .child(
                                                div()
                                                    .absolute()
                                                    .left_0()
                                                    .top_0()
                                                    .h_full()
                                                    .w(relative(fraction))
                                                    .rounded_full()
                                                    .bg(rgba(0xffffffff)),
                                            ),
                                    )
                                    .child(ticks),
                            )
                            .child(img(high_icon).size(px(20.0))),
                    ),
            )
        }
    }

    fn icon_pair(presentation: &Presentation) -> (PathBuf, PathBuf) {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/osd");
        let (low, high) = match (presentation.kind, presentation.muted) {
            (Kind::Output, true) => ("muted.svg", "speaker-high.svg"),
            (Kind::Output, false) => ("speaker-low.svg", "speaker-high.svg"),
            (Kind::Input, true) => ("muted.svg", "microphone-high.svg"),
            (Kind::Input, false) => ("microphone-low.svg", "microphone-high.svg"),
            (Kind::Display, _) => ("brightness-low.svg", "brightness-high.svg"),
        };
        (directory.join(low), directory.join(high))
    }

    #[derive(Default)]
    struct OsdWindows {
        windows: HashMap<Uuid, AnyWindowHandle>,
    }

    impl OsdWindows {
        fn reconcile(
            &mut self,
            desired: Option<&BTreeSet<Uuid>>,
            status: &Entity<OsdStatus>,
            cx: &mut App,
        ) {
            let displays = rmac_gpui_upstream_lab::output_surfaces::newest_displays(cx);
            let available = displays.keys().copied().collect::<BTreeSet<_>>();
            let active = desired
                .map(|desired| desired.intersection(&available).copied().collect())
                .unwrap_or(available);
            let removed = self
                .windows
                .keys()
                .filter(|uuid| !active.contains(uuid))
                .copied()
                .collect::<Vec<_>>();
            for uuid in removed {
                if let Some(handle) = self.windows.remove(&uuid) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }
            for (uuid, display) in displays {
                if active.contains(&uuid) && !self.windows.contains_key(&uuid) {
                    self.windows
                        .insert(uuid, open_osd(display, uuid, status.clone(), cx));
                }
            }
        }
    }

    fn open_osd(
        display: Rc<dyn PlatformDisplay>,
        output: Uuid,
        status: Entity<OsdStatus>,
        cx: &mut App,
    ) -> AnyWindowHandle {
        let display_id: DisplayId = display.id();
        cx.open_window(
            WindowOptions {
                titlebar: None,
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: Size::new(px(HIDDEN_SIZE), px(HIDDEN_SIZE)),
                })),
                display_id: Some(display_id),
                app_id: Some("dev.rmac.Osd".to_owned()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(LayerShellOptions {
                    namespace: format!("rmac-osd-{}", u64::from(display_id)),
                    layer: Layer::Overlay,
                    anchor: Anchor::TOP | Anchor::RIGHT,
                    margin: Some((px(36.0), px(12.0), px(0.0), px(0.0))),
                    keyboard_interactivity: KeyboardInteractivity::None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| OsdSurface::new(output, status, cx)),
        )
        .expect("open OSD layer surface")
        .into()
    }

    fn start_listener() -> Result<async_channel::Receiver<Presentation>, rmac_osd::Error> {
        let listener = rmac_osd::Listener::bind()?;
        let (sender, receiver) = async_channel::bounded(16);
        std::thread::Builder::new()
            .name("rmac-osd-ipc".into())
            .spawn(move || loop {
                match listener.receive() {
                    Ok(presentation) => {
                        if sender.send_blocking(presentation).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        eprintln!("OSD endpoint stopped: {error}");
                        std::process::exit(1);
                    }
                }
            })
            .map_err(|_| rmac_osd::Error {
                operation: rmac_osd::Operation::BindTransport,
            })?;
        Ok(receiver)
    }

    fn run_service() -> Result<(), rmac_osd::Error> {
        let updates = start_listener()?;
        let app = application().with_quit_mode(QuitMode::Explicit);
        app.run(move |cx: &mut App| {
            let (compositor_tx, compositor_rx) = async_channel::bounded(64);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = rmac_compositor_niri::watch(compositor_tx).await {
                        eprintln!("OSD compositor watcher stopped: {error}");
                    }
                })
                .detach();
            let status = cx.new(|cx| OsdStatus::new(updates, compositor_rx, cx));
            let (output_tx, output_rx) = async_channel::bounded(4);
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) =
                        rmac_gpui_upstream_lab::output_surfaces::watch_enabled(output_tx).await
                    {
                        eprintln!("OSD output watcher stopped: {error}");
                    }
                })
                .detach();
            cx.spawn(async move |cx| {
                let mut windows = OsdWindows::default();
                match output_rx.recv().await {
                    Ok(mut desired) => loop {
                        cx.update(|cx| windows.reconcile(Some(&desired), &status, cx));
                        let Ok(next) = output_rx.recv().await else {
                            break;
                        };
                        desired = next;
                    },
                    Err(_) => loop {
                        cx.update(|cx| windows.reconcile(None, &status, cx));
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                    },
                }
            })
            .detach();
        });
        Ok(())
    }

    fn run_command(value: &str) -> Result<(), rmac_osd::Error> {
        let command = rmac_osd::Command::parse(value).ok_or(rmac_osd::Error {
            operation: rmac_osd::Operation::ParseCommand,
        })?;
        let presentation = rmac_osd::execute(command)?;
        let _ = rmac_osd::send(&presentation);
        Ok(())
    }

    pub fn run() -> Result<(), rmac_osd::Error> {
        let arguments = env::args().skip(1).collect::<Vec<_>>();
        match arguments.as_slice() {
            [service] if service == "--service" => run_service(),
            [command] => run_command(command),
            _ => Err(rmac_osd::Error {
                operation: rmac_osd::Operation::ParseCommand,
            }),
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    if let Err(error) = linux_wayland::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("OSD requires Linux and: cargo run --features wayland --bin osd -- --service");
    std::process::exit(2);
}
