use gpui::{
    div, prelude::FluentBuilder as _, px, rgba, App, ElementId, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, Styled as _, Window,
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
    fill: Hsla,
    border: Hsla,
    glyph: &'static str,
    tooltip: &'static str,
    active: bool,
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
                .bg(fill)
                .border_1()
                .border_color(border)
                .flex()
                .items_center()
                .justify_center()
                .text_size(text_px(9.0))
                .font_weight(mac::BOLD)
                .text_color(rgba(0x00000000))
                .when(active, |circle| {
                    circle.hover(|c| c.text_color(mac::black().opacity(0.55)))
                })
                .child(glyph),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// The rmac traffic-light cluster (close / minimize / zoom), wired to the GPUI
/// window controls. Reusable so unified-toolbar apps can place it themselves.
pub fn traffic_lights() -> impl IntoElement {
    traffic_lights_active(true)
}

/// [`traffic_lights`] for an inactive window: the three buttons share the
/// inactive gray pair and do not reveal glyphs on hover.
pub fn traffic_lights_active(active: bool) -> impl IntoElement {
    let (close_fill, close_border) = if active {
        mac::traffic_close()
    } else {
        mac::traffic_inactive()
    };
    let (min_fill, min_border) = if active {
        mac::traffic_minimize()
    } else {
        mac::traffic_inactive()
    };
    let (zoom_fill, zoom_border) = if active {
        mac::traffic_zoom()
    } else {
        mac::traffic_inactive()
    };
    div()
        .flex()
        .items_center()
        .child(traffic_light(
            "tl-close",
            close_fill,
            close_border,
            "✕",
            "Close",
            active,
            // Route through the app's close guard (e.g. unsaved-changes prompt)
            // rather than closing the window directly. Apps bind `RequestClose`.
            |window, cx| window.dispatch_action(Box::new(components::RequestClose), cx),
        ))
        .child(traffic_light(
            "tl-min",
            min_fill,
            min_border,
            "—",
            "Minimize",
            active,
            |window, _| window.minimize_window(),
        ))
        .child(traffic_light(
            "tl-zoom",
            zoom_fill,
            zoom_border,
            "+",
            "Zoom",
            active,
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
