use gpui::{
    div, px, rgb, rgba, App, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Window,
};
use gpui_component::{
    button::{Button as ComponentButton, ButtonVariants as _},
    ActiveTheme as _, StyledExt as _, TitleBar,
};

use crate::{components, mac, text_px};

/// One traffic-light button: a colored circle that reveals its glyph on hover
/// and runs `on_click` (a window-control action). The glyph is always present
/// but transparent until hover, giving the macOS reveal-on-hover effect.
fn traffic_light(
    id: impl Into<ElementId>,
    color: Hsla,
    glyph: &'static str,
    tooltip: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    ComponentButton::new(id)
        .ghost()
        .tooltip(tooltip)
        .w(px(mac::traffic_light_hit_width()))
        .h(px(mac::traffic_light_hit_height()))
        .p_0()
        .rounded_full()
        .bg(rgba(0x00000000))
        .border_color(rgba(0x00000000))
        .shadow_none()
        .child(
            div()
                .size(px(mac::traffic_light_diameter()))
                .rounded_full()
                .bg(color)
                .flex()
                .items_center()
                .justify_center()
                .text_size(text_px(9.0))
                .font_weight(mac::BOLD)
                .text_color(rgba(0x00000000))
                .hover(|circle| circle.text_color(rgba(0x00000088)))
                .child(glyph),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// The rmac traffic-light cluster (close / minimize / zoom), wired to the GPUI
/// window controls. Reusable so unified-toolbar apps can place it themselves.
pub fn traffic_lights() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .child(traffic_light(
            "tl-close",
            rgb(0xff5f57).into(),
            "✕",
            "Close",
            // Route through the app's close guard (e.g. unsaved-changes prompt)
            // rather than closing the window directly. Apps bind `RequestClose`.
            |window, cx| window.dispatch_action(Box::new(components::RequestClose), cx),
        ))
        .child(traffic_light(
            "tl-min",
            rgb(0xfebc2e).into(),
            "—",
            "Minimize",
            |window, _| window.minimize_window(),
        ))
        .child(traffic_light(
            "tl-zoom",
            rgb(0x28c840).into(),
            "+",
            "Zoom",
            |window, _| window.zoom_window(),
        ))
}

/// Overlay our traffic lights in the left gutter (x=13) of a `TitleBar`. The
/// `TitleBar` forces its own children into an 80px left-padded zone, so the
/// lights are layered as a sibling anchored to the bar's true left edge.
fn with_traffic_lights(bar: impl IntoElement) -> impl IntoElement {
    div().relative().w_full().flex_shrink_0().child(bar).child(
        div()
            .absolute()
            .left(px(13.0))
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .child(traffic_lights()),
    )
}

/// The shared title bar: our own traffic lights on the left, centered title.
/// Apps put this at the top of their root `div`. The bar stays draggable via
/// gpui-component's `TitleBar` container.
pub fn title_bar(title: impl Into<SharedString>) -> impl IntoElement {
    let title: SharedString = title.into();
    with_traffic_lights(
        TitleBar::new().child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .child(title),
        ),
    )
}

/// A full-bleed page background using the active theme — the base every app
/// content sits on, below the title bar.
pub fn page() -> gpui::Div {
    div().size_full().v_flex()
}

/// Convenience: themed background color for the app body.
pub fn body_bg(cx: &App) -> gpui::Hsla {
    cx.theme().background
}

/// A unified macOS toolbar/title bar with the chrome color and a hairline base.
pub fn toolbar(children: impl IntoElement) -> impl IntoElement {
    with_traffic_lights(
        TitleBar::new()
            .bg(mac::chrome())
            .border_color(mac::separator())
            .child(children),
    )
}
