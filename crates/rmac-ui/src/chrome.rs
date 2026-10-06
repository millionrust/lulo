use std::time::Duration;

use gpui::{
    canvas, deferred, div, point, prelude::FluentBuilder as _, px, App, Bounds, Context, Entity,
    FocusHandle, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    PathBuilder, Pixels, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, WindowControlArea,
};
use gpui_component::{ActiveTheme as _, StyledExt as _};
use rmac_compositor::TileRegion;

use crate::{components, mac, text_px};

/// A window-management request from the traffic lights, the green button's
/// Move & Resize menu, or a title-bar double-click, carried out by the
/// compositor.
#[derive(Clone, Copy, Debug)]
enum WindowAction {
    ToggleFullscreen,
    Fill,
    /// Toggle between the window's user size and the working area, exactly
    /// as ⌥-clicking the green button and 🌐⌃F/🌐⌃R do (SET-33): fill it if
    /// it has no recorded pre-zoom frame, or put it back if it does.
    Zoom,
    Tile(TileRegion),
    Minimize,
}

/// Route a window control through the niri compositor. GPUI's own window
/// controls are no-ops under niri's floating policy, and niri has no native
/// minimize, so all three are compositor actions on this process's focused
/// window. Minimize parks the window on `rmac-parking` and records where it
/// came from so the app menu's Show All can restore it (§2.2).
#[cfg(unix)]
fn send_window_action(action: WindowAction, _cx: &mut App) {
    if let Some(command) = mission_control_command(action) {
        // Fill, Zoom and Tile are a floating-frame change: niri applies it
        // as three separate configures (width, height, position) that this
        // process asked for about itself. That self-targeted request comes
        // back `Handled`, but the window never actually resizes — reliably
        // reproducible, and confirmed unrelated to threading, timing, or
        // keeping this window's own frame loop awake during the round trip
        // (all tried; none changed the outcome). Asking from a different
        // process does work every time, which is exactly what 🌐⌃F and
        // 🌐⌃R already do: they run in Mission Control, never in the
        // window's own app. Route through the same resident service and
        // its one shared tile-history file (SET-33) instead of this
        // process asking niri to resize itself.
        spawn_window_action("rmac-window-action", move || {
            ask_mission_control(command);
        });
        return;
    }
    spawn_window_action("rmac-window-action", move || {
        async_io::block_on(perform_window_action(action));
    });
}

/// Windows carries out the same request through GPUI (see [`native`]).
#[cfg(not(unix))]
fn send_window_action(action: WindowAction, cx: &mut App) {
    native::perform(action, cx);
}

/// The command word Mission Control's resident service
/// (`rmac-mission-control --service`) understands for a `WindowAction`, or
/// `None` for the two actions this process still performs on itself
/// (Minimize's park-and-picture path and the traffic lights' plain-click
/// Full Screen both work fine self-targeted; only a floating-frame change
/// does not — see [`send_window_action`]). `TileRegion`'s four quarters
/// have no Mission Control command and no caller in this crate; they fall
/// back to the (known-unreliable) self-targeted path rather than silently
/// doing nothing.
#[cfg(unix)]
fn mission_control_command(action: WindowAction) -> Option<&'static str> {
    Some(match action {
        WindowAction::Fill => "fill",
        WindowAction::Zoom => "zoom",
        WindowAction::Tile(TileRegion::Left) => "tile-left",
        WindowAction::Tile(TileRegion::Right) => "tile-right",
        WindowAction::Tile(TileRegion::Top) => "tile-top",
        WindowAction::Tile(TileRegion::Bottom) => "tile-bottom",
        WindowAction::Tile(_) | WindowAction::ToggleFullscreen | WindowAction::Minimize => {
            return None
        }
    })
}

/// Fire-and-forget one word to Mission Control's resident service, the same
/// private, permission-checked datagram socket its own `ipc::send` uses for
/// niri binds and hot corners (`rmac-mission-control/src/ipc.rs`,
/// `$XDG_RUNTIME_DIR/rmac/mission-control.sock`). If the service is not
/// running, this is a silent no-op, the same as a niri bind would be.
#[cfg(unix)]
fn ask_mission_control(command: &'static str) {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let Ok(socket) = std::os::unix::net::UnixDatagram::unbound() else {
        return;
    };
    let _ = socket.send_to(
        command.as_bytes(),
        runtime.join("rmac").join("mission-control.sock"),
    );
}

/// Run disk and compositor work only when a user asks for it. GPUI's UI and
/// background executors can be occupied by app services, so neither should
/// hold a title-bar action until another input event wakes them.
#[cfg(unix)]
fn spawn_window_action(name: &'static str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("could not start {name}: {error}");
    }
}

/// The body of [`send_window_action`]'s self-targeted path (Minimize and
/// plain-click Full Screen), split out so the title bar's double-click
/// handler can resolve the saved
/// [`rmac_shell_settings::DoubleClickTitleBarAction`] off the main thread
/// first, then perform the same action the traffic lights use.
#[cfg(unix)]
async fn perform_window_action(action: WindowAction) {
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
        WindowAction::Minimize => {
            // The one minimize path every app shares: record the origin,
            // picture the window for its Dock tile, then park it.
            if let Err(error) = rmac_compositor_niri::minimize_window_in(snapshot, window).await {
                eprintln!("could not minimize: {error}");
            }
            return;
        }
        WindowAction::Fill | WindowAction::Zoom | WindowAction::Tile(_) => {
            unreachable!("Fill, Zoom and Tile run through Mission Control; see send_window_action")
        }
    };
    if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
        eprintln!("could not perform window action: {error:?}");
    }
}

/// Resolve Desktop & Dock's "Double-click a window's title bar to" setting
/// off the main thread, then perform the resulting action the same way the
/// traffic lights do (SET-33). Reads the shell-settings file fresh on every
/// double-click rather than polling or caching it. Public so an app with its
/// own drag region (System Settings' toolbar, Weather's and Clock's title
/// areas) can bind the same double-click behaviour `client_bar` uses.
pub fn double_click_title_bar_action(_cx: &mut App) {
    // A title-bar drag region is the window's caption on Windows, so the
    // system already zooms (maximises or restores) on a double-click.
    #[cfg(unix)]
    spawn_window_action("rmac-title-bar-action", move || {
        let setting = load_double_click_title_bar_action();
        // niri forwards the release to the client before ending its pointer
        // grab. A frame request during that handoff can be acknowledged but
        // dropped. Keep this wait on the on-demand worker, off the UI thread.
        std::thread::sleep(Duration::from_millis(120));
        match setting {
            rmac_shell_settings::DoubleClickTitleBarAction::Zoom => ask_mission_control("zoom"),
            rmac_shell_settings::DoubleClickTitleBarAction::Fill => ask_mission_control("fill"),
            rmac_shell_settings::DoubleClickTitleBarAction::Minimize => {
                async_io::block_on(perform_window_action(WindowAction::Minimize))
            }
            rmac_shell_settings::DoubleClickTitleBarAction::DoNothing => {}
        }
    });
}

/// The saved double-click action, or the Mac's own default (Zoom) when the
/// shell-settings store cannot be read.
#[cfg(unix)]
fn load_double_click_title_bar_action() -> rmac_shell_settings::DoubleClickTitleBarAction {
    rmac_shell_settings::ShellSettingsStore::from_environment()
        .and_then(|store| store.load())
        .map(|snapshot| snapshot.settings.double_click_title_bar)
        .unwrap_or_default()
}

/// Minimize this process's focused window, the same way the yellow traffic
/// light does. Exposed so an app can bind ⌘M to it (`todo.md` journey 8):
/// `WindowAction` and `send_window_action` are private to this module, so a
/// caller in another crate has no other way to reach this path.
pub fn minimize_focused_window(cx: &mut App) {
    send_window_action(WindowAction::Minimize, cx);
}

/// ⌘H: hide this application, parking every visible window it owns the
/// way the menu bar's Hide does, so Show All, the Dock and ⌘Tab bring them
/// back. With `others`, ⌥⌘H parks every other application's windows instead.
pub fn hide_application(others: bool, cx: &mut App) {
    #[cfg(not(unix))]
    native::hide_application(others, cx);
    #[cfg(unix)]
    cx.spawn(async move |_cx: &mut gpui::AsyncApp| {
        let pid = std::process::id() as i32;
        let Ok(snapshot) = rmac_compositor_niri::snapshot().await else {
            eprintln!("could not read windows to hide");
            return;
        };
        let windows = hidden_windows(&snapshot, pid, others);
        if windows.is_empty() {
            return;
        }
        let mut store = rmac_compositor::ParkingStore::load_default();
        store.prune(&snapshot);
        store.record_from(&snapshot, &windows);
        if let Err(error) = store.save_default() {
            eprintln!("could not save the parking set: {error}");
        }
        for window in windows {
            let action = rmac_compositor::Action::MinimizeWindow { window };
            if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                eprintln!("could not hide a window: {error:?}");
            }
        }
    })
    .detach();
}

/// ⌘Q: quit this application the way the menu bar's and the Dock's Quit do,
/// by asking the compositor to close each of its windows. Every window
/// therefore goes through its own close guard, so an edited document still
/// asks Save / Don't Save / Cancel. Files, like Finder, never quits.
pub fn quit_application(cx: &mut App) {
    #[cfg(not(unix))]
    native::quit_application(cx);
    #[cfg(unix)]
    cx.spawn(async move |_cx: &mut gpui::AsyncApp| {
        let pid = std::process::id() as i32;
        let Ok(snapshot) = rmac_compositor_niri::snapshot().await else {
            eprintln!("could not read windows to quit");
            return;
        };
        let Some(windows) = quit_windows(&snapshot, pid) else {
            return;
        };
        let mut store = rmac_compositor::ParkingStore::load_default();
        for window in &windows {
            store.forget(*window);
        }
        if let Err(error) = store.save_default() {
            eprintln!("could not save the parking set: {error}");
        }
        for window in windows {
            let action = rmac_compositor::Action::CloseWindow { window };
            if let Err(error) = rmac_compositor_niri::execute_action(&action).await {
                eprintln!("could not close a window to quit: {error:?}");
            }
        }
    })
    .detach();
}

/// Every window, hidden or not, that ⌘Q closes for the process `pid`, or
/// `None` when the process is Files, which has no Quit.
#[cfg(unix)]
fn quit_windows(
    snapshot: &rmac_compositor::Snapshot,
    pid: i32,
) -> Option<Vec<rmac_compositor::WindowId>> {
    let own = snapshot
        .windows
        .iter()
        .filter(|window| window.pid == Some(pid))
        .collect::<Vec<_>>();
    if own
        .iter()
        .any(|window| window.app_id.as_deref() == Some(rmac_apps::identity::FILES))
    {
        return None;
    }
    Some(own.into_iter().map(|window| window.id).collect())
}

/// The windows ⌘H (`others == false`) or ⌥⌘H (`others == true`) parks for
/// the process `pid`: its own visible windows, or every other application's.
#[cfg(unix)]
fn hidden_windows(
    snapshot: &rmac_compositor::Snapshot,
    pid: i32,
    others: bool,
) -> Vec<rmac_compositor::WindowId> {
    let own_app = snapshot
        .windows
        .iter()
        .filter(|window| window.pid == Some(pid))
        .min_by_key(|window| i32::from(!window.focused))
        .and_then(|window| window.app_id.clone());
    snapshot
        .windows
        .iter()
        .filter(|window| !rmac_compositor::window_is_parked(snapshot, window))
        .filter(|window| {
            let own = window.pid == Some(pid) || (own_app.is_some() && window.app_id == own_app);
            own != others
        })
        .map(|window| window.id)
        .collect()
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
/// One straight stroke of a glyph, from one point to another on its 14pt grid.
type Segment = ((f32, f32), (f32, f32));

fn paint_glyph(glyph: Glyph, bounds: Bounds<Pixels>, window: &mut Window) {
    let scale = f32::from(bounds.size.width) / 14.0;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let at = |x: f32, y: f32| point(px(left + x * scale), px(top + y * scale));
    let color = mac::black().opacity(0.5);
    let stroke = |width: f32, segments: &[Segment], window: &mut Window| {
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
                // A plain click enters full screen, as macOS 26's green
                // button does; ⌥-click Zooms instead (SET-33), toggling
                // between the window's user size and the working area.
                send_window_action(
                    if event.modifiers().alt {
                        WindowAction::Zoom
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

/// The shared drag and double-click region used by app title bars and custom
/// toolbars. Dispatch on the second release: on Wayland, a double-click
/// callback runs on the second press while the compositor still owns the
/// pointer grab, so its frame request can be acknowledged without applying.
pub fn title_bar_drag_region(id: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        // Child toolbar controls can stop bubbling. Observe the release in
        // capture so a double-click on title text still reaches the bar.
        .capture_any_mouse_up(|event, _, cx| {
            if event.button == MouseButton::Left && event.click_count == 2 {
                double_click_title_bar_action(cx);
            }
        })
}

/// A draggable client title bar that deliberately has no platform control
/// cluster. gpui-component's `TitleBar` adds Linux minimize/maximize/close
/// buttons on the right, which duplicated rmac's traffic lights.
fn client_bar(height: f32, base: Hsla, children: impl IntoElement) -> impl IntoElement {
    title_bar_drag_region("rmac-title-bar")
        .h(px(height))
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .pl(px(12.0))
        .bg(mac::chrome())
        .border_b_1()
        .border_color(base)
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

/// Windows: GPUI's own window calls, which Windows' window manager carries
/// out (ADR 0023). There is no Mission Control or parking workspace there yet.
#[cfg(not(unix))]
mod native {
    use gpui::App;

    use super::WindowAction;

    pub(super) fn perform(action: WindowAction, cx: &mut App) {
        let Some(handle) = cx.active_window() else {
            return;
        };
        let _ = handle.update(cx, |_, window, _| match action {
            WindowAction::Minimize => window.minimize_window(),
            WindowAction::ToggleFullscreen => window.toggle_fullscreen(),
            // Windows' own Zoom is Maximise; a second double-click on the
            // caption restores the window.
            WindowAction::Fill | WindowAction::Zoom => window.zoom_window(),
            WindowAction::Tile(_) => {
                eprintln!("tiling a window is not available on Windows yet");
            }
        });
    }

    /// ⌘H minimises this app's windows. ⌥⌘H needs other apps' windows,
    /// which only a Win32 window backend can reach.
    pub(super) fn hide_application(others: bool, cx: &mut App) {
        if others {
            eprintln!("hiding other applications is not available on Windows yet");
            return;
        }
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.minimize_window());
        }
    }

    /// ⌘Q closes each window through its own close guard, as on Linux, so
    /// an edited document still asks Save / Don't Save / Cancel.
    pub(super) fn quit_application(cx: &mut App) {
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, cx| {
                let close = crate::components::RequestClose;
                if window.is_action_available(&close, cx) {
                    window.dispatch_action(Box::new(close), cx);
                } else {
                    window.remove_window();
                }
            });
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rmac_compositor::{Snapshot, WindowId, WindowLayout, Workspace, WorkspaceId};

    fn window(
        id: u64,
        app: &str,
        pid: i32,
        workspace: u64,
        focused: bool,
    ) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: WindowId(id),
            title: None,
            app_id: Some(app.into()),
            pid: Some(pid),
            workspace: Some(WorkspaceId(workspace)),
            focused,
            floating: true,
            urgent: false,
            focus_timestamp: None,
            layout: WindowLayout::default(),
        }
    }

    fn workspace(id: u64, name: &str) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: Some(name.into()),
            output: None,
            urgent: false,
            active: id == 1,
            focused: id == 1,
            active_window: None,
        }
    }

    #[test]
    fn quit_closes_every_window_of_this_process_except_in_files() {
        let snapshot = Snapshot {
            workspaces: vec![
                workspace(1, "Desktop"),
                workspace(2, rmac_compositor::PARKING_WORKSPACE),
            ],
            windows: vec![
                window(10, "org.rmac.TextEditor", 100, 1, true),
                // Hidden windows close too.
                window(12, "org.rmac.TextEditor", 100, 2, false),
                // Another process of the same app is not this process.
                window(11, "org.rmac.TextEditor", 101, 1, false),
                window(40, rmac_apps::identity::FILES, 400, 1, false),
            ],
            ..Snapshot::default()
        };
        assert_eq!(
            quit_windows(&snapshot, 100),
            Some(vec![WindowId(10), WindowId(12)])
        );
        assert_eq!(quit_windows(&snapshot, 400), None);
        assert_eq!(quit_windows(&snapshot, 999), Some(Vec::new()));
    }

    #[test]
    fn hide_parks_this_apps_visible_windows_and_hide_others_the_rest() {
        let snapshot = Snapshot {
            workspaces: vec![
                workspace(1, "Desktop"),
                workspace(2, rmac_compositor::PARKING_WORKSPACE),
            ],
            windows: vec![
                window(10, "org.rmac.TextEditor", 100, 1, true),
                // A second process of the same app still counts as this app.
                window(11, "org.rmac.TextEditor", 101, 1, false),
                // Already hidden: never parked twice.
                window(12, "org.rmac.TextEditor", 100, 2, false),
                window(20, "firefox", 200, 1, false),
                window(21, "firefox", 200, 2, false),
                window(30, "org.rmac.Notes", 300, 1, false),
            ],
            ..Snapshot::default()
        };
        assert_eq!(
            hidden_windows(&snapshot, 100, false),
            vec![WindowId(10), WindowId(11)]
        );
        assert_eq!(
            hidden_windows(&snapshot, 100, true),
            vec![WindowId(20), WindowId(30)]
        );
        // A process with no window of its own hides nothing.
        assert!(hidden_windows(&snapshot, 999, false).is_empty());
    }
}
