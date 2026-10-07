//! The Dock: pinned apps, then every other running app after a separator,
//! each with a dot while it has a window. A click brings the app's front
//! window forward or opens the app. The shelf sits centred in the AppBar
//! strip the Dock reserves along the bottom of the screen.

use gpui::{
    div, img, prelude::FluentBuilder as _, px, App, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window,
};
use rmac_ui::{mac, IconSource};
use windows::Win32::Foundation::RECT;

use super::{runtime, shell, ShellState};
use crate::model::dock::{self as dock_model, Click, Launch, Tile, TileIcon};
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
const DOT: f32 = 4.0;

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

/// The shelf's width in GPUI pixels for `tiles`.
fn shelf_width(tiles: &[Tile]) -> f32 {
    let count = tiles.len() as f32;
    let separator = separator_at(tiles).map_or(0.0, |_| SEPARATOR_SLOT);
    2.0 * PADDING + count * ICON + (count - 1.0).max(0.0) * GAP + separator + 2.0
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
    let tiles = shell(cx).read(cx).tiles.clone();
    let width = (shelf_width(&tiles) * scale).round() as i32;
    let left = strip.left + ((strip.right - strip.left) - width) / 2;
    let rect = RECT {
        left,
        right: left + width,
        ..strip
    };
    super::later(cx, move || {
        surface::show_at(windows_list::handle(dock.hwnd), rect)
    });
    // The CI checks click tiles by their place on screen.
    let separator = separator_at(&tiles);
    for (index, tile) in tiles.iter().enumerate() {
        let gap = if separator.is_some_and(|at| index >= at) {
            SEPARATOR_SLOT
        } else {
            0.0
        };
        let centre = 1.0 + PADDING + index as f32 * (ICON + GAP) + gap + ICON / 2.0;
        trace(|| {
            format!(
                "dock tile {} {} at {},{}{}",
                tile.key,
                tile.name,
                rect.left + (centre * scale).round() as i32,
                rect.top + (SHELF / 2.0 * scale).round() as i32,
                if tile.running() { " running" } else { "" }
            )
        });
    }
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

impl Render for DockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        let state = self.shell.read(cx);
        let separator = separator_at(&state.tiles);
        let mut children = Vec::new();
        for (index, tile) in state.tiles.iter().enumerate() {
            if separator == Some(index) {
                children.push(
                    div()
                        .w(px(SEPARATOR_SLOT - GAP))
                        .h(px(ICON))
                        .flex()
                        .justify_center()
                        .child(div().w(px(1.0)).h_full().bg(mac::separator()))
                        .into_any_element(),
                );
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
                                .top(px(ICON + 1.5))
                                .left(px((ICON - DOT) / 2.0))
                                .size(px(DOT))
                                .rounded(px(mac::radius_pill()))
                                .bg(mac::text()),
                        )
                    })
                    .on_click(move |_, _, cx| open_tile(&clicked, cx))
                    .into_any_element(),
            );
        }
        let mut shelf_fill = mac::material_popover();
        shelf_fill.a = shelf_fill.a.max(0.8);
        div().size_full().flex().justify_center().child(
            div()
                .id("lulo-dock")
                .role(Role::Toolbar)
                .aria_label("Dock")
                .h(px(SHELF))
                .px(px(PADDING))
                .flex()
                .items_center()
                .gap(px(GAP))
                .rounded(px(mac::radius_dock()))
                .bg(shelf_fill)
                .border_1()
                .border_color(mac::separator())
                .children(children),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dock::default_pins;

    #[test]
    fn the_shelf_fits_its_tiles_and_separator() {
        let tiles = dock_model::tiles(&default_pins(), &[]);
        let width = shelf_width(&tiles);
        let count = tiles.len() as f32;
        let expected = 2.0 * PADDING + count * ICON + (count - 1.0) * GAP + 2.0;
        assert!((width - expected).abs() < 1e-3);
        assert!((DOCK_HEIGHT - 67.0).abs() < 1e-3);
    }
}
