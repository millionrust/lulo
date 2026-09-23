//! Launcher overlay geometry, invocation routing, creation, and token-checked release.

use super::*;

pub(crate) fn release(token: u64, cx: &mut App) {
    if cx.has_global::<LauncherService>() {
        cx.update_global::<LauncherService, _>(|service, _| {
            if service
                .active
                .as_ref()
                .is_some_and(|active| active.token == token)
            {
                service.active = None;
            }
        });
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
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
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

/// Development hosts place the window from `bounds`; only the Linux layer
/// surface takes a top margin.
#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
fn fallback_options(cx: &App) -> WindowOptions {
    overlay_options(WindowBounds::centered(size(px(WIDTH), px(HEIGHT)), cx), 0.0)
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

fn open_launcher(event: rmac_shortcuts::Event, options: WindowOptions, cx: &mut App) {
    let (token, registry, settings, error, clipboard) =
        cx.update_global::<LauncherService, _>(|service, _| {
            service.next_overlay = service.next_overlay.wrapping_add(1).max(1);
            (
                service.next_overlay,
                service.registry.clone(),
                service.settings.clone(),
                service.settings_error.clone(),
                service.clipboard.clone(),
            )
        });
    let mut launcher = None;
    let handle = cx.open_window(options, |window, cx| {
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
                },
                window,
                cx,
            )
        });
        launcher = Some(view.downgrade());
        cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
    });
    if let (Ok(handle), Some(view)) = (handle, launcher) {
        cx.update_global::<LauncherService, _>(|service, _| {
            service.active = Some(ActiveOverlay {
                token,
                view,
                window: handle.into(),
            });
        });
        cx.activate(true);
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn route_shortcut(event: rmac_shortcuts::Event, cx: &mut App) {
    if route_existing(&event, cx) {
        return;
    }
    open_launcher(event, fallback_options(cx), cx);
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
    let bounds =
        match context.centered_bounds(description.logical_width, description.logical_height) {
            Ok(bounds) => Bounds::new(
                point(px(bounds.x), px(bounds.y)),
                size(px(bounds.width), px(bounds.height)),
            ),
            Err(error) => {
                eprintln!("Launcher surface bounds rejected: {error}");
                return;
            }
        };
    open_launcher(
        event,
        overlay_options(WindowBounds::Windowed(bounds), description.margin_top),
        cx,
    );
}
