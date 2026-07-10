#[cfg(all(target_os = "linux", feature = "wayland"))]
mod linux_wayland {
    use gpui::{
        div, layer_shell::*, point, prelude::*, px, rgb, App, Bounds, Context, Size, Window,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };
    use gpui_platform::application;

    struct LayerShellLab;

    impl Render for LayerShellLab {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x171923))
                .text_color(rgb(0xf7fafc))
                .child("rmac layer-shell gate")
        }
    }

    pub fn run() {
        application().run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: Size::new(px(640.), px(40.)),
                    })),
                    app_id: Some("dev.rmac.LayerShellLab".to_owned()),
                    window_background: WindowBackgroundAppearance::Opaque,
                    kind: WindowKind::LayerShell(LayerShellOptions {
                        namespace: "rmac-layer-shell-lab".to_owned(),
                        layer: Layer::Top,
                        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                        keyboard_interactivity: KeyboardInteractivity::None,
                        exclusive_zone: Some(px(40.)),
                        exclusive_edge: Some(Anchor::TOP),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| LayerShellLab),
            )
            .expect("open the layer-shell surface");
        });
    }
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
fn main() {
    linux_wayland::run();
}

#[cfg(not(all(target_os = "linux", feature = "wayland")))]
fn main() {
    eprintln!("layer-shell requires Linux and: cargo run --features wayland --bin layer-shell");
    std::process::exit(2);
}
