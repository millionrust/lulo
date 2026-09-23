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
    use rmac_shell_ui::tokens;
    use uuid::Uuid;

    // macOS 26 draws the volume/brightness OSD as the Control Center Sound or
    // Display module (design-lab/switcher-osd.html, measured on the owner's
    // Mac): 292 × 64, radius 22, 14 from the right edge, 6 below the bar.
    const SURFACE_WIDTH: f32 = 292.0;
    const SURFACE_HEIGHT: f32 = 64.0;
    const RADIUS: f32 = 22.0;
    const RIGHT_MARGIN: f32 = 14.0;
    const GAP_BELOW_BAR: f32 = 6.0;
    const HIDDEN_SIZE: f32 = 1.0;
    const HIDE_DELAY: Duration = Duration::from_millis(1_600);
    /// Title: 13 pt bold at x 16.5, baseline 23 (a 16 pt line box from 10.5).
    const TITLE_LEFT: f32 = 16.5;
    const TITLE_TOP: f32 = 10.5;
    /// Slider: 4 thick on the centre line y = 41.75, no knob.
    const ROW_CENTER: f32 = 41.75;
    const ROW_HEIGHT: f32 = 17.0;
    const TRACK_HEIGHT: f32 = 4.0;
    /// The OSD has no trailing AirPlay/display button, so the row mirrors
    /// the module's left inset on the right.
    const ROW_RIGHT: f32 = 16.0;
    /// Dark glass measured over near-black content: #2E3034 → 52,54,58 @ 85 %.
    const DARK_TINT: u32 = 0x3436_3AD9;
    const DARK_RIM: u32 = 0xFFFF_FF66;
    const DARK_TRACK: u32 = 0x0000_0066;
    const DARK_FILL: u32 = 0xFAFA_FAFF;
    const LIGHT_TINT: u32 = 0xF6F6_F8D9;
    const LIGHT_RIM: u32 = 0xFFFF_FFB3;
    const LIGHT_TRACK: u32 = 0x0000_001A;
    const LIGHT_FILL: u32 = 0x1D1D_1FFF;

    /// One glyph with its measured size in points (the SVGs are cropped to
    /// the glyph, so the box is the drawn size).
    struct Glyph {
        file: &'static str,
        width: f32,
        height: f32,
    }

    /// Leading glyph, gap to the track, trailing gap, trailing glyph.
    struct RowGlyphs {
        low: Glyph,
        low_left: f32,
        low_gap: f32,
        high_gap: f32,
        high: Glyph,
    }

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
                                .map(rmac_shell_layer::stable_output_uuid);
                            this.fallback_output = snapshot
                                .outputs
                                .iter()
                                .filter(|output| output.enabled())
                                .map(|output| rmac_shell_layer::stable_output_uuid(&output.id))
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
            let glyphs = row_glyphs(&presentation);
            let fraction = f32::from(presentation.visible_level()) / 100.0;
            let label = presentation.accessible_label();
            let dark = tokens::is_dark();
            let (tint, rim, track, fill) = if dark {
                (DARK_TINT, DARK_RIM, DARK_TRACK, DARK_FILL)
            } else {
                (LIGHT_TINT, LIGHT_RIM, LIGHT_TRACK, LIGHT_FILL)
            };
            let text = if dark { 0xFFFF_FFFF } else { 0x0000_00E6 };
            root.child(
                div()
                    .id(format!("system-osd-{}", self.output))
                    .relative()
                    .size_full()
                    .rounded(px(RADIUS))
                    .bg(rgba(tint))
                    .border(px(0.5))
                    .border_color(rgba(rim))
                    .role(Role::Status)
                    .aria_label(label)
                    .child(
                        div()
                            .absolute()
                            .left(px(TITLE_LEFT))
                            .right(px(ROW_RIGHT))
                            .top(px(TITLE_TOP))
                            .h(px(16.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(13.0))
                            .line_height(px(16.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgba(text))
                            .child(title(&presentation)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right(px(ROW_RIGHT))
                            .top(px(ROW_CENTER - ROW_HEIGHT / 2.0))
                            .h(px(ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .child(
                                img(glyph_path(glyphs.low.file))
                                    .flex_none()
                                    .ml(px(glyphs.low_left))
                                    .mr(px(glyphs.low_gap))
                                    .w(px(glyphs.low.width))
                                    .h(px(glyphs.low.height)),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .h(px(TRACK_HEIGHT))
                                    .rounded_full()
                                    .bg(rgba(track))
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .h_full()
                                            .w(relative(fraction))
                                            .rounded_full()
                                            .bg(rgba(fill)),
                                    ),
                            )
                            .child(
                                img(glyph_path(glyphs.high.file))
                                    .flex_none()
                                    .ml(px(glyphs.high_gap))
                                    .w(px(glyphs.high.width))
                                    .h(px(glyphs.high.height)),
                            ),
                    ),
            )
        }
    }

    /// Control Center's module names: the OSD reads "Sound" or "Display" on
    /// the Mac; the device name stays in the accessible label.
    fn title(presentation: &Presentation) -> String {
        match presentation.kind {
            Kind::Output => "Sound".to_owned(),
            Kind::Display => "Display".to_owned(),
            Kind::Input => "Microphone".to_owned(),
        }
    }

    fn glyph_path(file: &str) -> PathBuf {
        let installed = PathBuf::from("/usr/share/rmac/osd").join(file);
        if installed.is_file() {
            return installed;
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/osd")
            .join(file)
    }

    fn row_glyphs(presentation: &Presentation) -> RowGlyphs {
        // Measured (pt): speaker.fill 9.25 × 13.5 at x 16.5, track from 34,
        // speaker.wave.3.fill 20.5 × 14.75 seven after the track; sun.min.fill
        // 15 at x 15.5, track from 36.5, sun.max.fill 16.5 six after it.
        const SPEAKER: Glyph = Glyph {
            file: "speaker-low.svg",
            width: 9.25,
            height: 13.5,
        };
        const SPEAKER_WAVES: Glyph = Glyph {
            file: "speaker-high.svg",
            width: 20.5,
            height: 14.75,
        };
        const MUTED: Glyph = Glyph {
            file: "muted.svg",
            width: 11.0,
            height: 13.5,
        };
        match (presentation.kind, presentation.muted) {
            (Kind::Output, muted) => RowGlyphs {
                low: if muted { MUTED } else { SPEAKER },
                low_left: 16.5,
                low_gap: if muted { 6.5 } else { 8.25 },
                high_gap: 7.0,
                high: SPEAKER_WAVES,
            },
            (Kind::Input, muted) => RowGlyphs {
                low: if muted {
                    MUTED
                } else {
                    Glyph {
                        file: "microphone-low.svg",
                        width: 15.0,
                        height: 15.0,
                    }
                },
                low_left: 15.5,
                low_gap: 6.0,
                high_gap: 6.0,
                high: Glyph {
                    file: "microphone-high.svg",
                    width: 17.0,
                    height: 17.0,
                },
            },
            (Kind::Display, _) => RowGlyphs {
                low: Glyph {
                    file: "brightness-low.svg",
                    width: 15.0,
                    height: 15.0,
                },
                low_left: 15.5,
                low_gap: 6.0,
                high_gap: 6.0,
                high: Glyph {
                    file: "brightness-high.svg",
                    width: 16.5,
                    height: 16.5,
                },
            },
        }
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
            let displays = rmac_shell_layer::output_surfaces::newest_displays(cx);
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
                    margin: Some((
                        px(tokens::menubar_height() + GAP_BELOW_BAR),
                        px(RIGHT_MARGIN),
                        px(0.0),
                        px(0.0),
                    )),
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
            rmac_shell_ui::tokens::install_appearance_watch(cx);
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
                        rmac_shell_layer::output_surfaces::watch_enabled(output_tx).await
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
