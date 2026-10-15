mod content;

use crate::accessibility::{DrawerEmptyState, OPENING_ANNOUNCEMENT};
use gpui::{
    div, prelude::FluentBuilder as _, px, svg, AppContext as _, Context, Div,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ObjectFit, ParentElement,
    Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled, StyledImage as _,
    Window,
};
#[cfg(windows)]
use gpui::{DragMoveEvent, MouseUpEvent};
use gpui_component::StyledExt as _;
use rmac_ui::{mac, Button, EmptyState, SearchField, Spinner};

use crate::catalog::{App, Category};
use crate::{ClearSearch, Launch, MoveDown, MoveLeft, MoveRight, MoveUp, OpenApp, RevealInFinder};

use super::{AppDrawer, LaunchDesktopAction, ViewMode, DRAWER_WIDTH, ICON, ROW_ICON, TILE_W};

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel is always exactly `DRAWER_WIDTH` wide.
        let viewport_width = DRAWER_WIDTH;
        let usable_width = (viewport_width - 48.0).max(TILE_W);
        self.cols = ((usable_width / (TILE_W + 8.0)).floor() as usize).max(1);
        self.scale_factor = window.scale_factor();

        let visible = self.visible_indices(cx);
        let selected = self.selected.min(visible.len().saturating_sub(1));
        let notice = if self.launching {
            Some((SharedString::from(OPENING_ANNOUNCEMENT), false))
        } else {
            self.action_error
                .clone()
                .or_else(|| self.catalog_error.clone())
                .map(|message| (message, true))
        };
        let context_menu = self.menu_at.clone().map(|state| {
            let menu = self.app_menu(state.position(), cx);
            (menu, state)
        });

        // APPS-01: the Mac's Apps window opens with a row of 7 recent apps
        // above a divider, then A–Z. Lulo shows it under the same
        // conditions the Mac shows it in — the unfiltered, unsearched grid.
        let recents = (!self.loading
            && self.view == ViewMode::Grid
            && self.filter.is_none()
            && self.query.read(cx).value().trim().is_empty())
        .then(|| self.recent_indices())
        .filter(|indices| !indices.is_empty())
        .map(|indices| self.recents_row(&indices, cx));

        #[cfg(windows)]
        if !self.loading && !self.apps.is_empty() && !self.ready_traced.replace(true) {
            rmac_shell_layer::windows::trace::trace(|| {
                format!("apps ready {} apps", self.apps.len())
            });
        }
        let body: gpui::AnyElement = if self.loading {
            div()
                .min_h(px(320.0))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap_3()
                        .child(Spinner::large())
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(mac::text_secondary())
                                .child("Loading Applications…"),
                        ),
                )
                .into_any_element()
        } else if visible.is_empty() {
            let empty = if self.apps.is_empty() {
                let state = DrawerEmptyState::EmptyCatalog;
                EmptyState::new(state.title()).message(state.message())
            } else {
                let state = DrawerEmptyState::NoMatches;
                EmptyState::new(state.title()).message(state.message())
            };
            div()
                .min_h(px(320.0))
                .w_full()
                .flex()
                .items_center()
                .child(empty)
                .into_any_element()
        } else if self.view == ViewMode::Grid {
            let tiles = visible
                .iter()
                .enumerate()
                .map(|(position, &index)| {
                    self.tile(
                        &self.apps[index],
                        position,
                        self.selection_visible && position == selected,
                        cx,
                    )
                })
                .collect::<Vec<_>>();
            #[cfg(windows)]
            {
                self.virtual_grid(tiles)
            }
            #[cfg(not(windows))]
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .justify_center()
                .children(tiles)
                .into_any_element()
        } else {
            let rows = visible
                .iter()
                .enumerate()
                .map(|(position, &index)| {
                    self.row(
                        &self.apps[index],
                        position,
                        self.selection_visible && position == selected,
                        cx,
                    )
                })
                .collect::<Vec<_>>();
            div()
                .v_flex()
                .gap_0p5()
                .w_full()
                .max_w(px(640.0))
                .mx_auto()
                .children(rows)
                .into_any_element()
        };

        let panel = div()
            .track_focus(&self.focus)
            .key_context("AppDrawer")
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| {
                this.move_by(-1, 0, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| {
                this.move_by(1, 0, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_by(0, -1, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_by(0, 1, cx);
            }))
            .on_action(cx.listener(|this, _: &Launch, _, cx| {
                this.launch_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenApp, _, cx| {
                this.open_selected(cx);
            }))
            .on_action(cx.listener(|this, action: &LaunchDesktopAction, _, cx| {
                // A declared desktop action (e.g. "New Window") is not "open
                // the app" for recents purposes, so no id is recorded.
                this.launch_application(None, action.launch.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| {
                this.reveal_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| {
                this.clear_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx) {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.dismiss(window, cx);
            }))
            // Drag from Apps to the Dock keeps the app there (macOS 26). On
            // Lulo OS the drag becomes a Wayland file drag of the app's
            // desktop entry once it nears Apps' edge (`content::DraggedApp`).
            // On Windows Apps and the Dock share `lulo-shell`, which holds
            // the pointer for Apps through the drag, so Apps reports where
            // the drag is on screen and the Dock opens its slot there.
            .map(|root| {
                #[cfg(windows)]
                let root = root
                    .on_drag_move(cx.listener(
                        |this, event: &DragMoveEvent<content::DraggedApp>, window, cx| {
                            use rmac_shell_layer::windows::dock_drag::{send, Hint, Phase};
                            let app_id = event.drag(cx).app_id.clone();
                            let screen = window.bounds().origin + event.event.position;
                            if !event.bounds.contains(&event.event.position) {
                                this.dock_drag = Some(app_id.clone());
                                send(
                                    cx,
                                    Hint {
                                        app_id,
                                        phase: Phase::Hover,
                                        screen,
                                    },
                                );
                            } else if this.dock_drag.take().is_some() {
                                send(
                                    cx,
                                    Hint {
                                        app_id,
                                        phase: Phase::Cancel,
                                        screen,
                                    },
                                );
                            }
                        },
                    ))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, window, cx| {
                            use rmac_shell_layer::windows::dock_drag::{send, Hint, Phase};
                            let Some(app_id) = this.dock_drag.take() else {
                                return;
                            };
                            let screen = window.bounds().origin + event.position;
                            send(
                                cx,
                                Hint {
                                    app_id,
                                    phase: Phase::Drop,
                                    screen,
                                },
                            );
                        }),
                    );
                root
            })
            .size_full()
            .v_flex()
            .bg(mac::material_popover())
            .rounded(px(mac::radius_large_surface()))
            .overflow_hidden()
            .border_1()
            .border_color(mac::separator())
            .occlude()
            .text_color(mac::text())
            .child(
                div()
                    .h(px(58.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_5()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        svg()
                            .path("icons/layout-dashboard.svg")
                            .w(px(24.0))
                            .h(px(24.0))
                            .text_color(mac::text_secondary()),
                    )
                    .child(
                        div().flex_1().child(
                            SearchField::new(&self.query)
                                .appearance(false)
                                .text_size(rmac_ui::text_px(25.0)),
                        ),
                    )
                    .child(self.view_toggle(cx)),
            )
            .when_some(notice, |drawer, (message, is_error)| {
                drawer.child(
                    div()
                        .id("app-drawer-notice")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(if is_error {
                            mac::error_background()
                        } else {
                            mac::control_fill()
                        })
                        .border_b_1()
                        .border_color(if is_error {
                            mac::error_border()
                        } else {
                            mac::separator()
                        })
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(if is_error {
                            mac::danger()
                        } else {
                            mac::text_secondary()
                        })
                        .child(div().flex_1().child(message))
                        .when(is_error, |element| {
                            element.child(
                                Button::new("dismiss-app-drawer-error", "Dismiss")
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.catalog_error = None;
                                        this.action_error = None;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
            })
            .child(self.category_bar(cx))
            .child(
                div()
                    .id("grid-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .map(|scroll| {
                        #[cfg(windows)]
                        let scroll = scroll.track_scroll(&self.grid_scroll);
                        scroll
                    })
                    .px_5()
                    .pb_5()
                    .children(recents)
                    .child(body),
            )
            .when_some(context_menu, |element: Div, (menu, state)| {
                element.child(menu.render(&state))
            });
        // On Windows the blur under the panel is drawn here, not by the
        // compositor (`rmac_shell_layer::over_backdrop`).
        #[cfg(windows)]
        {
            rmac_shell_layer::set_corner_radius(window, mac::radius_large_surface());
            rmac_shell_layer::over_backdrop(window, cx, mac::radius_large_surface(), panel)
        }
        #[cfg(not(windows))]
        panel
    }
}

/// WIN-OS-98: one grid row's height, tiles and the gap below them.
#[cfg(windows)]
const GRID_ROW_H: f32 = 104.0;

#[cfg(windows)]
impl AppDrawer {
    /// The grid in rows of `cols` tiles, building only the rows in view
    /// (and one either side): every tile's layout and paint ran on every
    /// scrolled frame before, about 40 ms a frame on the owner's laptop.
    /// Rows out of view keep their height as empty spacers, so the scroll
    /// range and positions do not change. Rows above are counted as if the
    /// recents block had no height, which only builds a few more.
    fn virtual_grid(&self, tiles: Vec<impl IntoElement>) -> gpui::AnyElement {
        let cols = self.cols.max(1);
        let viewport = f32::from(self.grid_scroll.bounds().size.height);
        let viewport = if viewport > 0.0 {
            viewport
        } else {
            super::DRAWER_HEIGHT
        };
        let (first, last) = grid_rows_in_view(
            -f32::from(self.grid_scroll.offset().y),
            f32::from(self.grid_scroll.max_offset().y),
            viewport,
        );
        let mut tiles = tiles.into_iter();
        let mut rows = Vec::new();
        let mut index = 0;
        loop {
            let row: Vec<_> = tiles.by_ref().take(cols).collect();
            if row.is_empty() {
                break;
            }
            let shown = (first..=last).contains(&index);
            rows.push(
                div()
                    .h(px(GRID_ROW_H))
                    .flex_none()
                    .flex()
                    .gap_2()
                    .justify_center()
                    .when(shown, |element| element.children(row))
                    .into_any_element(),
            );
            index += 1;
        }
        div().v_flex().children(rows).into_any_element()
    }
}

/// The grid rows to build for a scroll position `top` (px scrolled down).
/// A wheel or glide past either end leaves the handle's offset beyond the
/// range until the next layout clamps it; reading it unclamped built rows
/// below the real view, so the bottom rows flashed empty on every delta
/// past the end (owner report, WIN-OS-98).
#[cfg(windows)]
fn grid_rows_in_view(top: f32, max: f32, viewport: f32) -> (usize, usize) {
    let top = top.min(max.max(0.0)).max(0.0);
    let first = ((top / GRID_ROW_H).floor() as usize).saturating_sub(1);
    let last = ((top + viewport) / GRID_ROW_H).ceil() as usize + 1;
    (first, last)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn rows_past_either_end_are_those_at_the_end() {
        let max = 30.0 * GRID_ROW_H;
        let at_bottom = grid_rows_in_view(max, max, 400.0);
        assert_eq!(grid_rows_in_view(max + 500.0, max, 400.0), at_bottom);
        assert_eq!(grid_rows_in_view(max * 3.0, max, 400.0), at_bottom);
        assert!(at_bottom.0 <= 29 && at_bottom.1 >= 34);
        let at_top = grid_rows_in_view(0.0, max, 400.0);
        assert_eq!(grid_rows_in_view(-300.0, max, 400.0), at_top);
        assert_eq!(at_top, (0, 5));
        // Before the first layout there is no range yet: the top rows.
        assert_eq!(grid_rows_in_view(200.0, 0.0, 400.0), at_top);
    }
}
