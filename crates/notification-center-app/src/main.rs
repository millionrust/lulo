//! On-demand, service-backed Notification Center panel.

mod model;
mod render;
mod view;

use gpui::{
    point, px, size, App, AppContext as _, Application, Bounds, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowKind, WindowOptions,
};
use gpui_component::Root;

use crate::view::NotificationCenterView;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 720.0;
const EDGE_GAP: f32 = 12.0;
const TOP_GAP: f32 = 44.0;

fn panel_options(cx: &App) -> WindowOptions {
    let panel_size = size(px(WIDTH), px(HEIGHT));
    let bounds = cx
        .primary_display()
        .map(|display| display.bounds())
        .map(|display| {
            Bounds::new(
                point(
                    display.origin.x + display.size.width - px(WIDTH + EDGE_GAP),
                    display.origin.y + px(TOP_GAP),
                ),
                panel_size,
            )
        })
        .unwrap_or_else(|| Bounds::new(point(px(200.0), px(80.0)), panel_size));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Blurred,
        app_id: Some("org.rmac.NotificationCenter".into()),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    }
}

fn main() {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.open_window(panel_options(cx), |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| NotificationCenterView::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open Notification Center");
            cx.activate(true);
        });
}
