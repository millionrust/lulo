//! Probe: an idle, inactive layer surface paints an `img()` whose asset
//! finishes loading after the frame that requested it.
//!
//! GPUI loads image assets on the background executor and repaints the
//! requesting view from a next-frame callback. The vendored Wayland backend
//! parks an idle window's frame loop (docs/decisions/0013-gpui-linux-patch.md),
//! and GPUI throttles an inactive window's next-frame callbacks to 30 fps, so
//! a load that finished just after the loop parked used to wait for the next
//! input event: the desktop showed a folder's name without its icon.
//!
//! Each line on stdin adds one magenta swatch whose (embedded SVG) asset takes
//! the next delay from `--delays` to load. The surface never takes keyboard
//! focus, like the Dock and the menu bar. scripts/behavior/
//! run_desktop_first_paint.py captures the output after each line, with no
//! other input, and expects every swatch to be painted.

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use std::borrow::Cow;
    use std::io::BufRead as _;
    use std::time::Duration;

    use gpui::{
        div, img, layer_shell::*, point, prelude::*, px, rgb, App, AssetSource, Bounds, Context,
        SharedString, Size, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
        WindowOptions,
    };
    use gpui_platform::application;

    const SWATCH: f32 = 48.0;
    const SWATCH_SVG: &[u8] =
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48\" height=\"48\">\
<rect width=\"48\" height=\"48\" fill=\"#ff00ff\"/></svg>";

    struct DelayedAssets {
        delays: Vec<u64>,
    }

    impl AssetSource for DelayedAssets {
        fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
            let Some(index) = path
                .strip_prefix("swatch/")
                .and_then(|rest| rest.strip_suffix(".svg"))
                .and_then(|index| index.parse::<usize>().ok())
            else {
                return Ok(None);
            };
            // Runs on the background executor, like a real decode.
            let delay = self.delays[index % self.delays.len()];
            std::thread::sleep(Duration::from_millis(delay));
            Ok(Some(Cow::Borrowed(SWATCH_SVG)))
        }

        fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }

    struct Probe {
        shown: usize,
    }

    impl Render for Probe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                .bg(rgb(0x171923))
                .text_color(rgb(0xf7fafc))
                .child(format!("{}", self.shown))
                .children((0..self.shown).map(|index| {
                    img(SharedString::from(format!("swatch/{index}.svg"))).size(px(SWATCH))
                }))
        }
    }

    fn delays() -> Vec<u64> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--delays" {
                let parsed: Vec<u64> = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|delay| delay.trim().parse().ok())
                    .collect();
                if !parsed.is_empty() {
                    return parsed;
                }
            }
        }
        vec![0, 10, 25, 40, 50, 60, 80, 120]
    }

    pub fn run() {
        let delays = delays();
        application()
            .with_assets(DelayedAssets { delays })
            .run(|cx: &mut App| {
                let handle = cx
                    .open_window(
                        WindowOptions {
                            titlebar: None,
                            window_bounds: Some(WindowBounds::Windowed(Bounds {
                                origin: point(px(0.), px(0.)),
                                size: Size::new(px(560.), px(64.)),
                            })),
                            app_id: Some("dev.rmac.ImgPaintProbe".to_owned()),
                            window_background: WindowBackgroundAppearance::Opaque,
                            kind: WindowKind::LayerShell(LayerShellOptions {
                                namespace: "rmac-img-paint-probe".to_owned(),
                                layer: Layer::Top,
                                anchor: Anchor::TOP | Anchor::LEFT,
                                keyboard_interactivity: KeyboardInteractivity::None,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        |_, cx| cx.new(|_| Probe { shown: 0 }),
                    )
                    .expect("open the probe layer surface");

                let (sender, receiver) = async_channel::unbounded::<()>();
                std::thread::spawn(move || {
                    for line in std::io::stdin().lock().lines() {
                        if line.is_err() || sender.send_blocking(()).is_err() {
                            break;
                        }
                    }
                });
                cx.spawn(async move |cx| {
                    while receiver.recv().await.is_ok() {
                        let updated = handle.update(cx, |probe, _, cx| {
                            probe.shown += 1;
                            cx.notify();
                        });
                        if updated.is_err() {
                            break;
                        }
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
    eprintln!("img-paint requires Linux and: cargo run --features wayland --bin img-paint");
    std::process::exit(2);
}
