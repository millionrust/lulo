//! Launcher overlay geometry, invocation routing, creation, and token-checked release.

use super::*;
#[cfg(target_os = "linux")]
use gpui::ParentElement as _;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn malloc_trim(pad: usize) -> i32;
}

#[cfg(target_os = "linux")]
struct RendererWarmup;

#[cfg(target_os = "linux")]
impl gpui::Render for RendererWarmup {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div().child("Search")
    }
}

/// GPUI initializes the Wayland renderer when it opens its first window.
/// Do that once at login, before the shortcut endpoint becomes ready. This
/// tiny, pointer-transparent surface never asks the compositor for focus and
/// is destroyed after its first frame. The process-shared GPU context stays
/// initialized for the real launcher window.
#[cfg(target_os = "linux")]
pub(super) fn warm_renderer(cx: &mut App) {
    use gpui::layer_shell::KeyboardInteractivity;

    let mut options = overlay_options(
        WindowBounds::Windowed(Bounds::new(point(px(0.0), px(0.0)), size(px(1.0), px(1.0)))),
        0.0,
    );
    options.focus = false;
    if let WindowKind::LayerShell(layer) = &mut options.kind {
        layer.keyboard_interactivity = KeyboardInteractivity::None;
    }
    if let Err(error) = cx.open_window(options, |window, cx| {
        window.set_input_region(Some(&[]));
        window.on_next_frame(|window, cx| {
            window.remove_window();
            #[cfg(target_env = "gnu")]
            cx.spawn(async move |_: &mut gpui::AsyncApp| {
                async_io::Timer::after(std::time::Duration::from_millis(100)).await;
                // glibc documents malloc_trim as thread-safe. The warmup
                // window is gone before this asks glibc to return free pages.
                blocking::unblock(|| unsafe { malloc_trim(0) }).await;
            })
            .detach();
        });
        cx.new(|_| RendererWarmup)
    }) {
        eprintln!("Launcher renderer warmup failed: {error}");
    }
}

pub(crate) fn release(token: u64, cx: &mut App) {
    if cx.has_global::<LauncherService>() {
        let catcher: Option<AnyWindowHandle> =
            cx.update_global::<LauncherService, _>(|service, _| {
                let matches = service
                    .active
                    .as_ref()
                    .is_some_and(|active| active.token == token);
                if matches {
                    service.active = None;
                }
                #[cfg(target_os = "linux")]
                {
                    if service.pending_dismiss == Some(token) {
                        service.pending_dismiss = None;
                    }
                    if matches {
                        service.catcher.take()
                    } else {
                        None
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    None
                }
            });
        if let Some(catcher) = catcher {
            let _ = catcher.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Spotlight `token` just became compact (the bar only) or expanded (bar
/// plus results): move the catcher's hole to match. On Linux the surface
/// itself opens at its expanded size and never resizes (`view::set_compact`
/// instead widens or narrows the *window's own* input region to the bar or
/// the whole surface), and Spotlight's own window maps after the catcher
/// (so real presses inside Spotlight's current input region already reach
/// it first, unlike Control Centre's and Notification Center's popovers,
/// where the catcher maps last): real-click checks on a result row below
/// the bar, and on the gap between the bar and a vertically centred 88 pt
/// hole (`centered_bounds`, `route_activation`'s bug before this), found
/// no case where a stale hole here caused a wrong dismiss. This keeps the
/// hole correct anyway (UIA catcher audit) rather than leaving it
/// pointing at the wrong 88 pt rectangle, which a future change to the
/// window order could turn into the same bug Control Centre had.
#[cfg(target_os = "linux")]
pub(crate) fn follow_compact(token: u64, compact: bool, cx: &mut App) {
    let update = cx.update_global::<LauncherService, _>(|service, _| {
        if service
            .active
            .as_ref()
            .is_none_or(|active| active.token != token)
        {
            return None;
        }
        let hole = service.compact_hole?;
        let target = if compact {
            hole
        } else {
            Bounds::new(
                hole.origin,
                size(
                    hole.size.width,
                    px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
                ),
            )
        };
        Some((service.catcher?, target))
    });
    if let Some((catcher, target)) = update {
        rmac_ui::set_outside_click_catcher_hole(catcher, target, cx);
    }
}

#[cfg(target_os = "linux")]
fn overlay_options(bounds: WindowBounds, margin_top: f64) -> WindowOptions {
    use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};

    let size = bounds.get_bounds().size;
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.0), px(0.0)),
            size,
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "rmac-launcher".into(),
            layer: Layer::Overlay,
            // Top edge only: centred horizontally, bar top at the planned
            // margin, results growing downwards without moving the bar.
            anchor: Anchor::TOP,
            margin: Some((px(margin_top as f32), px(0.0), px(0.0), px(0.0))),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        }),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // Several separate glass shapes share this surface, and compositor
        // blur covers the whole surface as one square rectangle.
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.rmac.Launcher".into()),
        ..Default::default()
    }
}

/// On Windows the same layer surface, as a Win32 window that
/// `rmac-shell-layer` places (ADR 0023, "Phase 3 revised: shared shell
/// views"): its size here, its place from [`windows_layer`].
#[cfg(windows)]
fn overlay_options(bounds: WindowBounds, _margin_top: f64) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.0), px(0.0)),
            bounds.get_bounds().size,
        ))),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("org.rmac.Launcher".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

/// Spotlight's layer on Windows: the Linux surface's anchor and margin,
/// the bar's top where macOS draws it on `display`.
#[cfg(windows)]
fn windows_layer(cx: &App) -> rmac_shell_layer::layer::LayerShellOptions {
    use rmac_shell_layer::layer::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};

    let height = cx
        .primary_display()
        .map_or(768.0, |display| display.bounds().size.height.as_f32());
    LayerShellOptions {
        namespace: rmac_launcher::surface::NAMESPACE.into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP,
        margin: Some((
            px(rmac_launcher::surface::top_margin(f64::from(height)) as f32),
            px(0.0),
            px(0.0),
            px(0.0),
        )),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        ..Default::default()
    }
}

/// Development hosts place the window from `bounds`; only the Linux layer
/// surface takes a top margin.
#[cfg(not(any(target_os = "linux", windows)))]
fn overlay_options(bounds: WindowBounds, _margin_top: f64) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(bounds),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some("org.rmac.Launcher".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn fallback_options(cx: &App) -> WindowOptions {
    overlay_options(WindowBounds::centered(size(px(WIDTH), px(HEIGHT)), cx), 0.0)
}

/// Windows opens the surface at its expanded size and keeps the input
/// region to the bar while it is compact, as Linux does.
#[cfg(windows)]
fn fallback_options(cx: &App) -> WindowOptions {
    overlay_options(
        WindowBounds::centered(
            size(
                px(rmac_launcher::surface::EXPANDED_LOGICAL_WIDTH as f32),
                px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
            ),
            cx,
        ),
        0.0,
    )
}

fn route_existing(event: &rmac_shortcuts::Event, cx: &mut App) -> bool {
    let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
    if let Some(active) = active {
        if let Some(view) = active.view.upgrade() {
            let _ = cx.update_window(active.window, |_, window, cx| {
                view.update(cx, |view, cx| view.handle_shortcut(event, window, cx));
            });
            return true;
        }
        cx.update_global::<LauncherService, _>(|service, _| service.active = None);
    }
    false
}

fn open_launcher(
    event: rmac_shortcuts::Event,
    options: WindowOptions,
    excluded: Option<Bounds<gpui::Pixels>>,
    previous_window: Option<rmac_compositor::WindowId>,
    cx: &mut App,
) {
    #[cfg(not(target_os = "linux"))]
    let _ = excluded;
    let (token, registry, settings, error, clipboard, applications, learning) = cx
        .update_global::<LauncherService, _>(|service, _| {
            service.next_overlay = service.next_overlay.wrapping_add(1).max(1);
            #[cfg(target_os = "linux")]
            {
                service.pending_dismiss = None;
            }
            (
                service.next_overlay,
                service.registry.clone(),
                service.settings.clone(),
                service.settings_error.clone(),
                service.clipboard.clone(),
                service.application_provider.clone(),
                service.learning.clone(),
            )
        });
    #[cfg(target_os = "linux")]
    {
        // The catcher is lightweight; request it before the visible overlay
        // so the first outside click after Spotlight appears is not lost
        // while a second GPUI layer surface maps.
        let display = cx
            .displays()
            .into_iter()
            .find(|display| {
                excluded.is_some_and(|excluded| {
                    let screen = display.bounds();
                    excluded.origin.x >= screen.origin.x
                        && excluded.origin.x < screen.origin.x + screen.size.width
                        && excluded.origin.y >= screen.origin.y
                        && excluded.origin.y < screen.origin.y + screen.size.height
                })
            })
            .or_else(|| cx.primary_display());
        let catcher = display.and_then(|display| {
            rmac_ui::open_outside_click_catcher_around(
                "rmac-launcher-click-catcher",
                display,
                px(29.0),
                excluded,
                move |cx| {
                    let active = cx.read_global::<LauncherService, _>(|service, _| {
                        service
                            .active
                            .clone()
                            .filter(|active| active.token == token)
                    });
                    if let Some(active) = active {
                        if let Some(view) = active.view.upgrade() {
                            let _ = cx.update_window(active.window, |_, window, cx| {
                                view.update(cx, |view, cx| view.dismiss(window, cx));
                            });
                        }
                        release(token, cx);
                    } else {
                        cx.update_global::<LauncherService, _>(|service, _| {
                            service.pending_dismiss = Some(token);
                        });
                    }
                },
                cx,
            )
        });
        // Always the *compact* rectangle, whichever size the catcher's
        // hole actually opened with (`excluded`): `follow_compact` widens
        // it to the full surface and must be able to narrow it back.
        let compact_hole = excluded.map(|excluded| {
            Bounds::new(
                excluded.origin,
                size(
                    excluded.size.width,
                    px(rmac_launcher::surface::LOGICAL_HEIGHT as f32),
                ),
            )
        });
        cx.update_global::<LauncherService, _>(|service, _| {
            service.catcher = catcher;
            service.compact_hole = compact_hole;
        });
    }
    let mut launcher = None;
    #[cfg(windows)]
    let layer = windows_layer(cx);
    let build = |window: &mut gpui::Window, cx: &mut App| {
        if let Some(directory) = std::env::var_os("RMAC_SPOTLIGHT_FRAME_DIR") {
            window.on_next_frame(move |_, _| {
                let path = std::path::PathBuf::from(directory).join(format!("show-{token}.ready"));
                std::fs::write(&path, b"ready\n").unwrap_or_else(|error| {
                    panic!("write Spotlight frame marker {path:?}: {error}")
                });
            });
        }
        window.set_window_title("Spotlight");
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| {
            LauncherView::new(
                OverlayEnvironment {
                    token,
                    event,
                    registry,
                    settings,
                    settings_error: error,
                    clipboard,
                    applications,
                    learning,
                    previous_window,
                },
                window,
                cx,
            )
        });
        launcher = Some(view.downgrade());
        cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
    };
    #[cfg(windows)]
    let handle = rmac_shell_layer::open_layer_window(cx, options, layer, build);
    #[cfg(not(windows))]
    let handle = cx.open_window(options, build);
    if let (Ok(handle), Some(view)) = (handle, launcher) {
        #[cfg(target_os = "linux")]
        let cancel = cx.update_global::<LauncherService, _>(|service, _| {
            service.active = Some(ActiveOverlay {
                token,
                view,
                window: handle.into(),
            });
            service.pending_dismiss.take() == Some(token)
        });
        #[cfg(not(target_os = "linux"))]
        cx.update_global::<LauncherService, _>(|service, _| {
            service.active = Some(ActiveOverlay {
                token,
                view,
                window: handle.into(),
            });
        });
        #[cfg(target_os = "linux")]
        if cancel {
            let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
            if let Some(active) = active {
                if let Some(view) = active.view.upgrade() {
                    let _ = cx.update_window(active.window, |_, window, cx| {
                        view.update(cx, |view, cx| view.dismiss(window, cx));
                    });
                }
            }
            release(token, cx);
            return;
        }
        cx.activate(true);
        // A layer-shell popup opened from the shortcut endpoint does not
        // receive a pointer press. Request compositor keyboard focus so the
        // first Escape closes Spotlight even after other shell popovers.
        // (On Windows the layer takes the foreground once it is placed.)
        #[cfg(not(windows))]
        {
            let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
            if let Some(active) = active {
                let _ = active
                    .window
                    .update(cx, |_, window, _| window.activate_window());
            }
        }
    } else {
        #[cfg(target_os = "linux")]
        {
            let catcher = cx.update_global::<LauncherService, _>(|service, _| {
                service.pending_dismiss = None;
                service.catcher.take()
            });
            if let Some(catcher) = catcher {
                let _ = catcher.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn route_shortcut(event: rmac_shortcuts::Event, cx: &mut App) {
    if route_existing(&event, cx) {
        return;
    }
    // The dev-host shortcut path has no compositor snapshot to restore
    // focus from; ACC's Escape-focus-restore applies to the Linux shell
    // activation path below.
    open_launcher(event, fallback_options(cx), None, None, cx);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn route_activation(
    activation: rmac_shell_activation_runtime::Activation,
    cx: &mut App,
) {
    let (event, context) = activation.into_parts();
    if route_existing(&event, cx) {
        return;
    }
    let context = match context {
        Ok(context) => context,
        Err(error) => {
            eprintln!("Launcher activation rejected: {error}");
            return;
        }
    };
    let description =
        match rmac_launcher::surface::plan_invocation(context.invocation(), context.compositor()) {
            Ok(description) => description,
            Err(error) => {
                eprintln!("Launcher surface plan rejected: {error}");
                return;
            }
        };
    // Horizontally centred, `description.margin_top` below the output's
    // top edge: where the bar (and the window around it) actually render,
    // never full-screen-centred -- the mismatch an outside-click catcher
    // placed by `centered_bounds` left (SPOT catcher audit).
    let bounds = match context.top_centered_bounds(
        description.logical_width,
        description.logical_height,
        description.margin_top,
    ) {
        Ok(bounds) => Bounds::new(
            point(px(bounds.x), px(bounds.y)),
            size(px(bounds.width), px(bounds.height)),
        ),
        Err(error) => {
            eprintln!("Launcher surface bounds rejected: {error}");
            return;
        }
    };
    // The surface opens at its expanded height and never resizes while
    // typing (`view::set_compact`); the catcher's hole starts at the same
    // size `set_compact` gives the window's own input region, widening or
    // narrowing with it (`follow_compact`) -- an app-drawer-style
    // activation opens straight into the expanded view, not the compact
    // bar.
    let surface_bounds = Bounds::new(
        bounds.origin,
        size(
            bounds.size.width,
            px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32).max(bounds.size.height),
        ),
    );
    let excluded = if requested_browse_mode(&event).is_some() {
        surface_bounds
    } else {
        bounds
    };
    // Captured before Spotlight's own window opens and takes focus, so
    // Escape (or an outside click) can hand focus back to it, as on the
    // Mac. `None` whenever nothing was focused (e.g. an empty desktop).
    let previous_window = context.compositor().focus.window;
    open_launcher(
        event,
        overlay_options(
            WindowBounds::Windowed(surface_bounds),
            description.margin_top,
        ),
        Some(excluded),
        previous_window,
        cx,
    );
}

/// Type `query` into the open Spotlight (the shell scene).
pub(super) fn set_scene_query(query: String, cx: &mut App) {
    let active = cx.read_global::<LauncherService, _>(|service, _| service.active.clone());
    let Some(active) = active else {
        return;
    };
    if let Some(view) = active.view.upgrade() {
        let _ = cx.update_window(active.window, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.set_query_from_assistive_technology(query, window, cx)
            });
        });
    }
}

/// Open Spotlight on the first output without a shortcut activation, as
/// the shell scene shows it, with `query` typed.
#[cfg(target_os = "linux")]
pub(super) fn open_scene(query: String, cx: &mut App) {
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let snapshot = match rmac_compositor_system::snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("Spotlight scene: no compositor snapshot: {error}");
                return;
            }
        };
        cx.update(|cx| {
            let Some(output) = snapshot.outputs.iter().find(|output| output.enabled()) else {
                eprintln!("Spotlight scene: no output");
                return;
            };
            let description = match rmac_launcher::surface::SeatId::new("seat0")
                .and_then(|seat| rmac_launcher::surface::plan(&output.id, seat, &snapshot))
            {
                Ok(description) => description,
                Err(error) => {
                    eprintln!("Spotlight scene: {error}");
                    return;
                }
            };
            let bar = Bounds::new(
                point(px(0.0), px(0.0)),
                size(
                    px(description.logical_width as f32),
                    px(description.logical_height as f32),
                ),
            );
            let surface = Bounds::new(
                bar.origin,
                size(
                    bar.size.width,
                    px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
                ),
            );
            let event = rmac_shortcuts::Event::Activated {
                id: rmac_shortcuts::ShortcutId("launcher".into()),
                timestamp_ms: 1,
            };
            open_launcher(
                event,
                overlay_options(WindowBounds::Windowed(surface), description.margin_top),
                Some(bar),
                None,
                cx,
            );
            set_scene_query(query, cx);
        });
    })
    .detach();
}
