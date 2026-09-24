use std::time::Duration;

use gpui::{
    canvas, deferred, div, point, prelude::FluentBuilder as _, px, App, Bounds, Context, Entity,
    FocusHandle, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder,
    Pixels, RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea,
};
use gpui_component::{ActiveTheme as _, InteractiveElementExt as _, StyledExt as _};
use rmac_compositor::TileRegion;

use crate::{components, mac, text_px};

/// A window-management request from the traffic lights or the green
/// button's Move & Resize menu, carried out by the compositor.
#[derive(Clone, Copy)]
enum WindowAction {
    ToggleFullscreen,
    Fill,
    Tile(TileRegion),
    Minimize,
}

/// Route a window control through the niri compositor. GPUI's own window
/// controls are no-ops under niri's floating policy, and niri has no native
/// minimize, so all three are compositor actions on this process's focused
/// window. Minimize parks the window on `rmac-parking` and records where it
/// came from so the app menu's Show All can restore it (§2.2).
fn send_window_action(action: WindowAction, cx: &mut App) {
    let executor = cx.background_executor().clone();
    cx.spawn(async move |_cx: &mut gpui::AsyncApp| {
        let pid = std::process::id() as i32;
        let Ok(snapshot) = rmac_compositor_niri::snapshot().await else {
            return;
        };
        let window = snapshot
            .windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .min_by_key(|window| i32::from(!window.focused))
            .map(|window| window.id);
        let Some(window) = window else { return };
        let action = match action {
            WindowAction::ToggleFullscreen => {
                rmac_compositor::Action::FullscreenWindow { window, on: true }
            }
            WindowAction::Fill => rmac_compositor::Action::FillWindow { window },
            WindowAction::Tile(region) => rmac_compositor::Action::TileWindow { window, region },
            WindowAction::Minimize => {
                let mut store = rmac_compositor::ParkingStore::load_default();
                store.record_from(&snapshot, &[window]);
                // Capture the tile thumbnail while the window is still on
                // screen; the Dock shows it for the parked window (§4.11).
                match (
                    rmac_compositor::window_logical_rect(&snapshot, window),
                    rmac_compositor::ParkingStore::default_thumbnail_path(window),
                ) {
                    (Some(rect), Some(path)) => {
                        if capture_thumbnail(&executor, rect, &path).await {
                            store.set_thumbnail(window, path);
                        }
                    }
                    (rect, path) => {
                        eprintln!("no minimized-tile geometry: rect={rect:?} path={path:?}");
                    }
                }
                if let Err(error) = store.save_default() {
                    eprintln!("could not save the parking set: {error}");
                }
                rmac_compositor::Action::MinimizeWindow { window }
            }
        };
        let _ = rmac_compositor_niri::execute_action(&action).await;
    })
    .detach();
}

/// Minimize this process's focused window, the same way the yellow traffic
/// light does. Exposed so an app can bind ⌘M to it (`todo.md` journey 8):
/// `WindowAction` and `send_window_action` are private to this module, so a
/// caller in another crate has no other way to reach this path.
pub fn minimize_focused_window(cx: &mut App) {
    send_window_action(WindowAction::Minimize, cx);
}

/// Capture a logical rectangle into `path` for a minimized-window thumbnail.
/// `grim` scales the region to the output's physical pixels; if the tool is
/// missing or fails the tile simply falls back to the application icon.
async fn capture_thumbnail(
    executor: &gpui::BackgroundExecutor,
    rect: rmac_compositor::LogicalRect,
    path: &std::path::Path,
) -> bool {
    let geometry = format!(
        "{:.0},{:.0} {:.0}x{:.0}",
        rect.x.round(),
        rect.y.round(),
        rect.width.round(),
        rect.height.round()
    );
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("could not create the thumbnail directory: {error}");
            return false;
        }
    }
    let path = path.to_path_buf();
    let captured = path.clone();
    let status = executor
        .spawn(async move {
            std::process::Command::new("grim")
                .arg("-g")
                .arg(&geometry)
                .arg(&path)
                .status()
        })
        .await;
    match status {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("grim failed for {captured:?}: {status}");
            false
        }
        Err(error) => {
            eprintln!("could not run grim: {error}");
            false
        }
    }
}

/// The glyph a traffic light reveals while the pointer is over the group.
#[derive(Clone, Copy)]
enum Glyph {
    Close,
    Minimize,
    FullScreen,
}

/// Paint `glyph` in a light's 14 pt circle as vector paths, black at 50 %
/// as measured on macOS 26 (design-lab/chrome.html): × with ±3.5 arms, an
/// 8 pt −, and the full-screen pair of right triangles with 4.75 pt legs.
fn paint_glyph(glyph: Glyph, bounds: Bounds<Pixels>, window: &mut Window) {
    let scale = f32::from(bounds.size.width) / 14.0;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let at = |x: f32, y: f32| point(px(left + x * scale), px(top + y * scale));
    let color = mac::black().opacity(0.5);
    let stroke = |width: f32, segments: &[((f32, f32), (f32, f32))], window: &mut Window| {
        let mut path = PathBuilder::stroke(px(width * scale));
        for &((x0, y0), (x1, y1)) in segments {
            path.move_to(at(x0, y0));
            path.line_to(at(x1, y1));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    };
    match glyph {
        Glyph::Close => stroke(
            1.5,
            &[((3.5, 3.5), (10.5, 10.5)), ((10.5, 3.5), (3.5, 10.5))],
            window,
        ),
        Glyph::Minimize => stroke(1.75, &[((3.0, 7.0), (11.0, 7.0))], window),
        Glyph::FullScreen => {
            let mut path = PathBuilder::fill();
            for triangle in [
                [(3.75, 3.75), (8.5, 3.75), (3.75, 8.5)],
                [(10.25, 10.25), (5.5, 10.25), (10.25, 5.5)],
            ] {
                path.move_to(at(triangle[0].0, triangle[0].1));
                path.line_to(at(triangle[1].0, triangle[1].1));
                path.line_to(at(triangle[2].0, triangle[2].1));
                path.close();
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
    }
}

const LIGHTS_GROUP: &str = "rmac-traffic-lights";

/// One light: a 16 pt hit box (the AX button frame) holding the 14 pt circle.
/// Glyphs appear when the pointer is anywhere over the group, and an inactive
/// window's grey lights regain their colours at the same moment.
///
/// `name` is the accessible name AppKit gives the same control ("Close
/// window", "Minimize", "Enter Full Screen"); `focus` makes the light a Tab
/// stop so keyboard and switch-control users can reach it, matching AppKit's
/// AX button semantics rather than requiring a pointer.
#[allow(clippy::too_many_arguments)]
fn traffic_light(
    id: &'static str,
    name: &'static str,
    (fill, border): (Hsla, Hsla),
    glyph: Glyph,
    active: bool,
    enabled: bool,
    focus: Option<(FocusHandle, bool)>,
) -> gpui::Stateful<gpui::Div> {
    let (inactive_fill, inactive_border) = mac::traffic_inactive();
    let lit = active && enabled;
    let is_focused = focus.as_ref().is_some_and(|(_, focused)| *focused);
    let circle = div()
        .relative()
        .size(px(mac::traffic_light_diameter()))
        .rounded_full()
        .border_1()
        .bg(if lit { fill } else { inactive_fill })
        .border_color(if lit { border } else { inactive_border })
        .when(is_focused, |circle| circle.shadow(mac::focus_ring_shadow()))
        .when(enabled && !active, |circle| {
            circle.group_hover(LIGHTS_GROUP, move |style| {
                style.bg(fill).border_color(border)
            })
        })
        .when(enabled, |circle| {
            circle.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(0.0)
                    .group_hover(LIGHTS_GROUP, |style| style.opacity(1.0))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, (), window, _| paint_glyph(glyph, bounds, window),
                        )
                        .size_full(),
                    ),
            )
        });
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(name)
        .size(px(mac::traffic_light_hit_width()))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when_some(focus, |el, (handle, _)| {
            el.track_focus(&handle.tab_stop(true).tab_index(0))
        })
        .child(circle)
}

/// Hover state behind the green button's Move & Resize menu.
#[derive(Default)]
struct ZoomMenu {
    open: bool,
    over_button: bool,
    over_menu: bool,
    generation: u64,
}

/// macOS opens the menu after the pointer rests on the green button.
const ZOOM_MENU_OPEN_DELAY: Duration = Duration::from_millis(500);
/// …and closes it shortly after the pointer leaves both button and menu.
const ZOOM_MENU_CLOSE_DELAY: Duration = Duration::from_millis(250);

impl ZoomMenu {
    fn set_hover(
        &mut self,
        over_button: Option<bool>,
        over_menu: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if let Some(over) = over_button {
            self.over_button = over;
        }
        if let Some(over) = over_menu {
            self.over_menu = over;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let (delay, open) = if self.over_button || self.over_menu {
            if self.open {
                return;
            }
            (ZOOM_MENU_OPEN_DELAY, true)
        } else {
            if !self.open {
                return;
            }
            (ZOOM_MENU_CLOSE_DELAY, false)
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |menu, cx| {
                if menu.generation == generation && menu.open != open {
                    menu.open = open;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.over_button = false;
        self.over_menu = false;
        self.generation = self.generation.wrapping_add(1);
        cx.notify();
    }
}

/// A Move & Resize icon: a 25 × 20 rounded outline holding the filled part
/// of the screen the window will take.
fn tile_icon(fill: (f32, f32, f32, f32)) -> gpui::Div {
    let metrics = rmac_design::Metrics::default();
    let (width, height) = (metrics.tile_icon_width, metrics.tile_icon_height);
    let (x, y, w, h) = fill;
    // The fill sits 3 pt inside the outline, as drawn by macOS.
    let inner_w = width - 6.0;
    let inner_h = height - 6.0;
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(4.0))
        .border(px(1.5))
        .border_color(mac::text())
        .child(
            div()
                .absolute()
                .left(px(1.5 + x * inner_w))
                .top(px(1.5 + y * inner_h))
                .w(px(w * inner_w))
                .h(px(h * inner_h))
                .rounded(px(1.5))
                .bg(mac::text()),
        )
}

/// The popover the green button shows on hover: Move & Resize halves, Fill,
/// and Full Screen, each a compositor action on this window. The Mac's
/// multi-window Arrange layouts and the Full Screen tiling submenu need a
/// window arranger rmac does not have, so they are not drawn.
fn zoom_menu(menu: Entity<ZoomMenu>) -> impl IntoElement {
    let metrics = rmac_design::Metrics::default();
    let pitch = metrics.tile_icon_pitch;
    let item = |id: &'static str, icon: gpui::Div, action: WindowAction, menu: Entity<ZoomMenu>| {
        div().id(id).child(icon).on_click(move |_, _, cx| {
            menu.update(cx, |menu, cx| menu.close(cx));
            send_window_action(action, cx);
        })
    };
    let header = |label: &'static str| {
        div()
            .px(px(17.0))
            .h(px(16.0))
            .text_size(text_px(13.0))
            .font_weight(mac::SEMIBOLD)
            .text_color(mac::text_tertiary())
            .child(label)
    };
    let separator = || div().mx(px(14.0)).h(px(1.0)).bg(mac::separator());
    let row = |children: Vec<gpui::AnyElement>| {
        div()
            .h(px(41.0))
            .pl(px(22.5))
            .flex()
            .items_center()
            .gap(px(pitch - metrics.tile_icon_width))
            .children(children)
    };
    let halves = [
        ("tile-left", (0.0, 0.0, 0.5, 1.0), TileRegion::Left),
        ("tile-right", (0.5, 0.0, 0.5, 1.0), TileRegion::Right),
        ("tile-top", (0.0, 0.0, 1.0, 0.5), TileRegion::Top),
        ("tile-bottom", (0.0, 0.5, 1.0, 0.5), TileRegion::Bottom),
    ]
    .into_iter()
    .map(|(id, fill, region)| {
        item(
            id,
            tile_icon(fill),
            WindowAction::Tile(region),
            menu.clone(),
        )
        .into_any_element()
    })
    .collect();
    let hover_menu = menu.clone();
    let outside_menu = menu.clone();
    div()
        .id("rmac-zoom-menu")
        .occlude()
        .w(px(metrics.tile_popover_width))
        .h(px(metrics.tile_popover_height))
        .pt(px(12.0))
        .flex()
        .flex_col()
        .rounded(px(rmac_design::Radii::default().tile_popover))
        .bg(mac::material_popover())
        .border_1()
        .border_color(mac::separator())
        .shadow_lg()
        .text_size(text_px(13.0))
        .text_color(mac::text())
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            hover_menu.update(cx, |menu, cx| menu.set_hover(None, Some(hovered), cx));
        })
        .on_mouse_down_out(move |_, _, cx| {
            outside_menu.update(cx, |menu, cx| menu.close(cx));
        })
        .child(header("Move & Resize"))
        .child(row(halves))
        .child(div().pt(px(6.0)).child(separator()))
        .child(div().pt(px(11.0)).child(header("Fill & Arrange")))
        .child(row(vec![item(
            "tile-fill",
            tile_icon((0.0, 0.0, 1.0, 1.0)),
            WindowAction::Fill,
            menu.clone(),
        )
        .into_any_element()]))
        .child(div().pt(px(6.0)).child(separator()))
        .child(
            div()
                .id("tile-full-screen")
                .h(px(36.0))
                .px(px(17.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .relative()
                        .w(px(16.0))
                        .h(px(12.0))
                        .rounded(px(2.5))
                        .border(px(1.5))
                        .border_color(mac::text())
                        .child(
                            div()
                                .absolute()
                                .left(px(4.5))
                                .top_0()
                                .bottom_0()
                                .w(px(1.5))
                                .bg(mac::text()),
                        ),
                )
                .child("Full Screen")
                .on_click(move |_, _, cx| {
                    menu.update(cx, |menu, cx| menu.close(cx));
                    send_window_action(WindowAction::ToggleFullscreen, cx);
                }),
        )
}

/// The close / minimise / zoom cluster, laid out as the Mac's AX frames:
/// 16 pt hit boxes 23 apart. Place its top-left corner at the first light's
/// centre minus half a hit box ([`traffic_lights_origin`]).
#[derive(IntoElement)]
pub struct TrafficLights {
    /// `None` follows the window's own key state.
    active: Option<bool>,
    zoom_enabled: bool,
}

impl RenderOnce for TrafficLights {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let active = self.active.unwrap_or_else(|| window.is_window_active());
        let menu = window.use_keyed_state("rmac-zoom-menu-state", cx, |_, _| ZoomMenu::default());
        let open = self.zoom_enabled && menu.read(cx).open;
        let hit = mac::traffic_light_hit_width();
        let metrics = rmac_design::Metrics::default();
        let zoom_enabled = self.zoom_enabled;
        let hover_menu = menu.clone();
        let close_focus = window
            .use_keyed_state("tl-close-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let min_focus = window
            .use_keyed_state("tl-min-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let zoom_focus = window
            .use_keyed_state("tl-zoom-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let close_is_focused = close_focus.is_focused(window);
        let min_is_focused = min_focus.is_focused(window);
        let zoom_is_focused = zoom_focus.is_focused(window);
        let zoom = traffic_light(
            "tl-zoom",
            "Enter Full Screen",
            mac::traffic_zoom(),
            Glyph::FullScreen,
            active,
            zoom_enabled,
            zoom_enabled.then_some((zoom_focus, zoom_is_focused)),
        )
        .when(zoom_enabled, |zoom| {
            zoom.on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                hover_menu.update(cx, |menu, cx| menu.set_hover(Some(hovered), None, cx));
            })
            .on_click(move |event, _, cx| {
                // ⌥-click fills instead of entering full screen, as on macOS.
                send_window_action(
                    if event.modifiers().alt {
                        WindowAction::Fill
                    } else {
                        WindowAction::ToggleFullscreen
                    },
                    cx,
                )
            })
        });
        div()
            .group(LIGHTS_GROUP)
            .relative()
            .flex()
            .items_center()
            .gap(px(metrics.traffic_spacing - hit))
            .child(
                traffic_light(
                    "tl-close",
                    "Close window",
                    mac::traffic_close(),
                    Glyph::Close,
                    active,
                    true,
                    Some((close_focus, close_is_focused)),
                )
                // Route through the app's close guard (e.g. an unsaved-changes
                // prompt) rather than closing the window directly.
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(components::RequestClose), cx)
                }),
            )
            .child(
                traffic_light(
                    "tl-min",
                    "Minimize",
                    mac::traffic_minimize(),
                    Glyph::Minimize,
                    active,
                    true,
                    Some((min_focus, min_is_focused)),
                )
                .on_click(|_, _, cx| send_window_action(WindowAction::Minimize, cx)),
            )
            .child(zoom)
            .when(open, |cluster| {
                // Measured: the popover's left edge 26.5 and top 27.5 from the
                // cluster's top-left, a 10 pt arrow under the green button.
                cluster.child(deferred(
                    div()
                        .absolute()
                        .left(px(26.5))
                        .top(px(27.5))
                        .child(zoom_menu(menu)),
                ))
            })
    }
}

/// Where a [`TrafficLights`] cluster's top-left corner goes for a window with
/// or without a unified toolbar, from the measured first-light centre.
pub fn traffic_lights_origin(unified_toolbar: bool) -> f32 {
    let metrics = rmac_design::Metrics::default();
    let center = if unified_toolbar {
        metrics.traffic_center_toolbar
    } else {
        metrics.traffic_center_titlebar
    };
    center - metrics.traffic_hit / 2.0
}

/// The rmac traffic-light cluster (close / minimise / zoom). It follows the
/// window's key state: grey while the window is inactive, coloured again when
/// the pointer is over the group. Reusable so toolbar apps place it
/// themselves.
pub fn traffic_lights() -> TrafficLights {
    TrafficLights {
        active: None,
        zoom_enabled: true,
    }
}

/// [`traffic_lights`] with an explicit key state.
pub fn traffic_lights_active(active: bool) -> TrafficLights {
    TrafficLights {
        active: Some(active),
        zoom_enabled: true,
    }
}

/// [`traffic_lights`] for a fixed-size window such as Calculator: macOS draws
/// the zoom button in the inactive grey and it does nothing.
pub fn traffic_lights_fixed_size(active: bool) -> TrafficLights {
    TrafficLights {
        active: Some(active),
        zoom_enabled: false,
    }
}

/// A draggable client title bar that deliberately has no platform control
/// cluster. gpui-component's `TitleBar` adds Linux minimize/maximize/close
/// buttons on the right, which duplicated rmac's traffic lights.
fn client_bar(height: f32, base: Hsla, children: impl IntoElement) -> impl IntoElement {
    div()
        .id("rmac-title-bar")
        .h(px(height))
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .pl(px(12.0))
        .bg(mac::chrome())
        .border_b_1()
        .border_color(base)
        .window_control_area(WindowControlArea::Drag)
        .on_double_click(|_, window, _| window.zoom_window())
        .child(div().h_full().flex_1().child(children))
}

/// Overlay the traffic lights at their measured position on a bar.
fn with_traffic_lights(unified_toolbar: bool, bar: impl IntoElement) -> impl IntoElement {
    let origin = traffic_lights_origin(unified_toolbar);
    div().relative().w_full().flex_shrink_0().child(bar).child(
        div()
            .absolute()
            .left(px(origin))
            .top(px(origin))
            .child(traffic_lights()),
    )
}

/// Width from a title bar's left edge to where its title starts: the lights'
/// hit boxes plus the measured gap (TextEdit's title sits at x + 83).
fn title_bar_title_inset() -> f32 {
    let metrics = rmac_design::Metrics::default();
    traffic_lights_origin(false)
        + 3.0 * metrics.traffic_hit
        + 2.0 * (metrics.traffic_spacing - metrics.traffic_hit)
        + metrics.title_gap
}

/// The shared title bar of a title-bar-only window (TextEdit, Terminal):
/// 32 pt tall, the lights centred 16 from the corner and the title in 13 pt
/// bold secondary text immediately after them.
pub fn title_bar(title: impl Into<SharedString>) -> impl IntoElement {
    let title: SharedString = title.into();
    let metrics = rmac_design::Metrics::default();
    with_traffic_lights(
        false,
        client_bar(
            metrics.titlebar_height,
            mac::titlebar_base(),
            div()
                .size_full()
                .flex()
                .items_center()
                .pl(px(title_bar_title_inset() - 12.0))
                .text_size(text_px(metrics.title_titlebar_size))
                .font_weight(mac::BOLD)
                .text_color(mac::text_secondary())
                .truncate()
                .child(title),
        ),
    )
}

/// A title-bar-only window's bar with application-owned content.
pub fn title_bar_content(children: impl IntoElement) -> impl IntoElement {
    with_traffic_lights(
        false,
        client_bar(
            rmac_design::Metrics::default().titlebar_height,
            mac::titlebar_base(),
            children,
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

/// A unified 52 pt toolbar with the chrome colour and a hairline base; the
/// lights sit centred 26 from the corner, as in Finder and Notes.
pub fn toolbar(children: impl IntoElement) -> impl IntoElement {
    with_traffic_lights(
        true,
        client_bar(mac::toolbar_height(), mac::separator(), children),
    )
}

/// A toolbar window's title: 15 pt bold primary text (Finder "jake").
pub fn toolbar_title(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(text_px(rmac_design::Metrics::default().title_toolbar_size))
        .font_weight(mac::BOLD)
        .text_color(mac::text())
        .truncate()
        .child(title.into())
}

/// A grouped glass capsule for toolbar items (Tahoe): 36 pt tall, like
/// Finder's back/forward pair and view switcher.
pub fn toolbar_group(children: impl IntoElement) -> impl IntoElement {
    let height = rmac_design::Metrics::default().toolbar_group_height;
    div()
        .h(px(height))
        .px(px(4.0))
        .flex()
        .items_center()
        .rounded(px(height / 2.0))
        .bg(mac::material_clear())
        .border_1()
        .border_color(mac::separator())
        .child(children)
}
