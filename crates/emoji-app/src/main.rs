//! `rmac-emoji`: Emoji & Symbols, opened by ⌃⌘Space and by Edit ▸ Emoji &
//! Symbols. Pressing the shortcut again closes the panel. Choosing an emoji
//! puts it into the field that had the keyboard.

#[cfg(target_os = "linux")]
mod inject;
#[cfg(target_os = "linux")]
mod instance;
#[cfg(target_os = "linux")]
mod view;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("rmac-emoji runs on Lulo OS; Windows has its own emoji panel (Win+.)");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
mod linux {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::process::ExitCode;
    use std::rc::Rc;

    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
    use gpui::{
        point, px, size, App, AppContext as _, AssetSource, Bounds, Result, SharedString,
        WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    };

    use crate::view::{EmojiView, Outcome, HEIGHT, WIDTH};

    /// The layer-shell namespace niri's layer-rule matches (blur, radius).
    const NAMESPACE: &str = "rmac-emoji";

    struct Assets;

    impl AssetSource for Assets {
        fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
            gpui_component_assets::Assets.load(path)
        }

        fn list(&self, path: &str) -> Result<Vec<SharedString>> {
            gpui_component_assets::Assets.list(path)
        }
    }

    /// The window and the app it came from: the panel's layer surface takes
    /// the keyboard, and closing it leaves the compositor with nothing to
    /// hand focus to, so the previous window is refocused by hand.
    #[derive(Clone)]
    struct Previous {
        window: Option<rmac_compositor::WindowId>,
        app_id: Option<String>,
    }

    fn previous_window() -> Previous {
        let snapshot = futures_lite::future::block_on(rmac_compositor_system::snapshot());
        let Ok(snapshot) = snapshot else {
            return Previous {
                window: None,
                app_id: None,
            };
        };
        let window = snapshot.focus.window;
        let app_id = window.and_then(|id| {
            snapshot
                .windows
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.app_id.clone())
        });
        Previous { window, app_id }
    }

    fn refocus(window: Option<rmac_compositor::WindowId>) {
        let Some(window) = window else {
            return;
        };
        let action = rmac_compositor::Action::FocusWindow { window };
        if let Err(error) =
            futures_lite::future::block_on(rmac_compositor_system::execute_action(&action))
        {
            eprintln!("rmac-emoji: could not refocus the previous window: {error:?}");
        }
    }

    fn options() -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(WIDTH), px(HEIGHT)),
            ))),
            titlebar: None,
            focus: true,
            show: true,
            // No anchor: the compositor centres the panel on the output
            // that has the keyboard.
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: NAMESPACE.into(),
                layer: Layer::Overlay,
                anchor: Anchor::empty(),
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                ..Default::default()
            }),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("org.rmac.Emoji".into()),
            ..Default::default()
        }
    }

    pub fn run() -> ExitCode {
        let previous = previous_window();
        // A second press of the shortcut closes the panel that is open.
        let _guard = match crate::instance::claim(previous.window.map(|window| window.0)) {
            crate::instance::Launch::Open(guard) => guard,
            crate::instance::Launch::Closed { restore } => {
                refocus(restore.map(rmac_compositor::WindowId));
                return ExitCode::SUCCESS;
            }
        };
        let outcome = Rc::new(RefCell::new(Outcome::default()));
        let shown = outcome.clone();
        rmac_ui::application()
            .with_assets(Assets)
            .run(move |cx: &mut App| {
                rmac_ui::init_application(cx);
                let opened = cx.open_window(options(), |window, cx| {
                    rmac_ui::prepare_surface_window(window, cx);
                    let view = cx.new(|cx| EmojiView::new(shown.clone(), window, cx));
                    cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
                });
                if let Err(error) = opened {
                    eprintln!("rmac-emoji: could not open the panel: {error}");
                    cx.quit();
                }
                cx.on_window_closed(|cx, _| {
                    if cx.windows().is_empty() {
                        cx.quit();
                    }
                })
                .detach();
                cx.activate(true);
            });

        // The panel is gone: give the keyboard back, then paste.
        refocus(previous.window);
        let glyph = outcome.borrow_mut().glyph.take();
        if let Some(glyph) = glyph {
            // Focus follows the compositor's own pace.
            std::thread::sleep(std::time::Duration::from_millis(120));
            let chord = crate::inject::Paste::for_app(previous.app_id.as_deref());
            if let Err(error) = crate::inject::paste(&glyph, chord) {
                eprintln!("rmac-emoji: could not insert {glyph}: {error}");
                return ExitCode::FAILURE;
            }
        }
        ExitCode::SUCCESS
    }
}
