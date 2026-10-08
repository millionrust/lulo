//! The Dock: pinned apps, then every other running app after a separator,
//! each with a dot while it has a window, and the Recycle Bin at the end
//! after another separator, where the Mac has the Trash. A click brings the
//! app's front window forward or opens the app; the Recycle Bin opens on a
//! click and offers Open and Empty Recycle Bin on a right-click. The
//! window is exactly the rounded shelf, frosted by Windows behind it
//! (`win::backdrop`), centred in the AppBar strip the Dock reserves along
//! the bottom of the screen.

use gpui::{
    div, img, prelude::FluentBuilder as _, px, App, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use rmac_ui::{mac, IconSource};
use windows::Win32::Foundation::RECT;

use super::{runtime, shell, ShellState};
use crate::model::dock::{self as dock_model, Click, Launch, Tile, TileIcon};
use crate::model::menus;
use crate::win::backdrop::{self, Surface as Backdrop};
use crate::win::{launch, surface, trace, windows_list};

/// The Mac's tile at its default size on a small screen.
pub(crate) const ICON: f32 = 48.0;
const GAP: f32 = 3.0;
/// Tile to shelf rim on every side (the Mac's 10 of 64).
const PADDING: f32 = 7.5;
const SHELF: f32 = ICON + 2.0 * PADDING;
/// Shelf rim to screen edge.
const MARGIN: f32 = 4.0;
/// The AppBar strip the Dock reserves.
pub(crate) const DOCK_HEIGHT: f32 = SHELF + MARGIN;
const SEPARATOR_SLOT: f32 = 1.0 + 2.0 * 10.0;
/// The running dot: a touch larger than the Mac's 4 pt, which reads too
/// faintly on a low-resolution screen.
const DOT: f32 = 5.0;
/// The Recycle Bin's key in the CI trace.
const BIN_KEY: &str = "recycle-bin";
const BIN_NAME: &str = "Recycle Bin";
/// The Recycle Bin menu's height, for placing it above the tile.
const BIN_MENU_HEIGHT: f32 = 66.0;

pub(crate) struct DockView {
    shell: Entity<ShellState>,
    _observe: Subscription,
}

impl DockView {
    pub(crate) fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shell = shell(cx);
        let observe = cx.observe(&shell, |_, _, cx| cx.notify());
        Self {
            shell,
            _observe: observe,
        }
    }
}

/// Where the separator goes: before the first app that is not pinned.
fn separator_at(tiles: &[Tile]) -> Option<usize> {
    tiles.iter().position(|tile| !tile.pinned)
}

/// The shelf's width in GPUI pixels for `tiles` and the Recycle Bin.
fn shelf_width(tiles: &[Tile]) -> f32 {
    let count = tiles.len() as f32 + 1.0;
    let separators = separator_at(tiles).map_or(0.0, |_| SEPARATOR_SLOT) + SEPARATOR_SLOT;
    2.0 * PADDING + count * ICON + (count - 1.0).max(0.0) * GAP + separators + 2.0
}

/// The left edge of tile `index` in the shelf (the Recycle Bin is index
/// `tiles.len()`).
fn tile_left(tiles: &[Tile], index: usize) -> f32 {
    let separators = separator_at(tiles)
        .filter(|&at| index >= at)
        .map_or(0.0, |_| SEPARATOR_SLOT)
        + if index >= tiles.len() {
            SEPARATOR_SLOT
        } else {
            0.0
        };
    1.0 + PADDING + index as f32 * (ICON + GAP) + separators
}

/// Size the Dock's window to its shelf and centre it in its strip.
pub(crate) fn place(cx: &mut App) {
    let runtime = runtime(cx);
    let (Some(dock), strip, scale) = (runtime.dock, runtime.dock_strip, runtime.scale) else {
        return;
    };
    if strip.right <= strip.left {
        return;
    }
    let state = shell(cx).read(cx);
    let tiles = state.tiles.clone();
    let bin_full = state.bin_full;
    let width = (shelf_width(&tiles) * scale).round() as i32;
    let height = (SHELF * scale).round() as i32;
    let left = strip.left + ((strip.right - strip.left) - width) / 2;
    let rect = RECT {
        left,
        right: left + width,
        top: strip.top,
        bottom: strip.top + height,
    };
    let radius = (mac::radius_dock() * scale).round() as i32;
    cx.global_mut::<super::Runtime>().dock_rect = rect;
    super::later(cx, move || {
        let hwnd = windows_list::handle(dock.hwnd);
        surface::show_at(hwnd, rect);
        backdrop::round(hwnd, width, height, radius);
    });
    // The CI checks click tiles by their place on screen.
    let centre_y = rect.top + (SHELF / 2.0 * scale).round() as i32;
    let at =
        |index: usize| rect.left + ((tile_left(&tiles, index) + ICON / 2.0) * scale).round() as i32;
    for (index, tile) in tiles.iter().enumerate() {
        trace(|| {
            format!(
                "dock tile {} {} at {},{}{}{}",
                tile.key,
                tile.name,
                at(index),
                centre_y,
                if tile.running() { " running" } else { "" },
                match &tile.icon {
                    TileIcon::Asset(path) => format!(" icon {path}"),
                    TileIcon::Shell(_) => " icon shell".to_owned(),
                }
            )
        });
    }
    trace(|| {
        format!(
            "dock tile {BIN_KEY} {BIN_NAME} at {},{} {}",
            at(tiles.len()),
            centre_y,
            if bin_full { "full" } else { "empty" }
        )
    });
}

fn open_tile(tile: &Tile, cx: &mut App) {
    match dock_model::click(tile) {
        Click::Activate(hwnd) => super::later(cx, move || windows_list::activate(hwnd)),
        Click::Launch(Launch::Lulo(app)) => launch::open(launch::Request::Lulo(app.exe.to_owned())),
        Click::Launch(Launch::Shell(target)) => launch::open(launch::Request::Shell(target)),
        Click::Launch(Launch::None) => {}
    }
    trace(|| format!("dock click {}", tile.key));
}

/// The Recycle Bin's menu, above its tile.
fn open_bin_menu(cx: &mut App) {
    let runtime = runtime(cx);
    let (rect, scale) = (runtime.dock_rect, runtime.scale);
    let state = shell(cx).read(cx);
    let left = rect.left + (tile_left(&state.tiles, state.tiles.len()) * scale).round() as i32;
    let top = rect.top - ((BIN_MENU_HEIGHT + 6.0) * scale).round() as i32;
    let items = menus::recycle_bin_menu(state.bin_full);
    super::menu::open_dock_menu(items, (left, top), cx);
}

fn separator() -> gpui::AnyElement {
    div()
        .w(px(SEPARATOR_SLOT - GAP))
        .h(px(ICON))
        .flex()
        .justify_center()
        .child(div().w(px(1.0)).h_full().bg(mac::separator()))
        .into_any_element()
}

impl Render for DockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        let state = self.shell.read(cx);
        let separator_index = separator_at(&state.tiles);
        let mut dot_fill = mac::text();
        dot_fill.a = 1.0;
        let mut children = Vec::new();
        for (index, tile) in state.tiles.iter().enumerate() {
            if separator_index == Some(index) {
                children.push(separator());
            }
            let icon = match &tile.icon {
                TileIcon::Asset(path) => {
                    rmac_ui::svg_icon(IconSource::from(*path), ICON, scale, cx)
                        .size(px(ICON))
                        .into_any_element()
                }
                TileIcon::Shell(source) => match state.cached_icon(source) {
                    Some(image) => img(image).size(px(ICON)).into_any_element(),
                    None => div()
                        .size(px(ICON))
                        .rounded(px(mac::radius_card()))
                        .bg(mac::control_fill())
                        .into_any_element(),
                },
            };
            let clicked = tile.clone();
            children.push(
                div()
                    .id(("lulo-dock-tile", index))
                    .role(Role::Button)
                    .aria_label(SharedString::from(tile.name.clone()))
                    .relative()
                    .size(px(ICON))
                    .child(icon)
                    .when(tile.running(), |tile| {
                        tile.child(
                            div()
                                .absolute()
                                .top(px(ICON + 1.0))
                                .left(px((ICON - DOT) / 2.0))
                                .size(px(DOT))
                                .rounded(px(mac::radius_pill()))
                                .bg(dot_fill),
                        )
                    })
                    .on_click(move |_, _, cx| open_tile(&clicked, cx))
                    .into_any_element(),
            );
        }
        children.push(separator());
        children.push(
            div()
                .id("lulo-dock-recycle-bin")
                .role(Role::Button)
                .aria_label(BIN_NAME)
                .size(px(ICON))
                .child(
                    rmac_ui::svg_icon(
                        IconSource::from(dock_model::bin_icon(state.bin_full)),
                        ICON,
                        scale,
                        cx,
                    )
                    .size(px(ICON)),
                )
                .on_click(|_, _, _| {
                    trace(|| format!("dock click {BIN_KEY}"));
                    launch::open(launch::Request::Shell("shell:RecycleBinFolder".into()));
                })
                .on_mouse_down(MouseButton::Right, |_, _, cx| {
                    cx.stop_propagation();
                    cx.defer(open_bin_menu);
                })
                .into_any_element(),
        );
        // Over Windows' blur the tint is light, as the Mac's Dock material;
        // without it (transparency effects off) it is nearly opaque.
        let mut shelf_fill = mac::material_popover();
        shelf_fill.a = if backdrop::frosted(Backdrop::Dock) {
            shelf_fill.a.min(0.45)
        } else {
            shelf_fill.a.max(0.85)
        };
        div()
            .id("lulo-dock")
            .role(Role::Toolbar)
            .aria_label("Dock")
            .size_full()
            .px(px(PADDING))
            .flex()
            .items_center()
            .gap(px(GAP))
            .rounded(px(mac::radius_dock()))
            .bg(shelf_fill)
            .border_1()
            .border_color(mac::separator())
            .children(children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dock::default_pins;

    #[test]
    fn the_shelf_fits_its_tiles_the_bin_and_its_separator() {
        let tiles = dock_model::tiles(&default_pins(), &[]);
        let width = shelf_width(&tiles);
        let count = tiles.len() as f32 + 1.0;
        let expected = 2.0 * PADDING + count * ICON + (count - 1.0) * GAP + SEPARATOR_SLOT + 2.0;
        assert!((width - expected).abs() < 1e-3);
        assert!((DOCK_HEIGHT - 67.0).abs() < 1e-3);
        // The bin sits after its separator, at the shelf's right end.
        let bin = tile_left(&tiles, tiles.len());
        assert!((bin + ICON + PADDING + 1.0 - width).abs() < 1e-3);
    }
}
