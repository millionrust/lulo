mod content;

use gpui::{
    div, img, prelude::FluentBuilder as _, px, svg, AppContext as _, Context, Div, DragMoveEvent,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ObjectFit,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    StyledImage as _, Window,
};
use gpui_component::StyledExt as _;
use rmac_app_drawer::accessibility::{DrawerEmptyState, OPENING_ANNOUNCEMENT};
use rmac_ui::{mac, Button, EmptyState, SearchField, Spinner};

use crate::catalog::{App, Category};
use crate::{ClearSearch, Launch, MoveDown, MoveLeft, MoveRight, MoveUp, OpenApp, RevealInFinder};

use super::{AppDrawer, LaunchDesktopAction, ViewMode, ICON, ROW_ICON, TILE_W};

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport_width = f32::from(window.viewport_size().width);
        let usable_width = (viewport_width - 48.0).max(TILE_W);
        self.cols = ((usable_width / (TILE_W + 8.0)).floor() as usize).max(1);

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

        div()
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
            // Drag from Apps to the Dock (§ drag from Apps): GPUI's Linux
            // backend has no cross-process drag source, so this reports
            // Apps' own window-relative pointer position over
            // `drag_endpoint` instead of a real Wayland drag. Crossing
            // below Apps' own window is the nearest available proxy for
            // "heading toward the Dock", which normally sits at the
            // screen's bottom edge, below Apps' centered popover.
            .on_drag_move(cx.listener(
                |this, event: &DragMoveEvent<content::DraggedApp>, window, cx| {
                    let app_id = event.drag(cx).app_id.clone();
                    let size = window.viewport_size();
                    let below = f32::from(event.event.position.y) > f32::from(size.height);
                    if below {
                        let fraction = (f32::from(event.event.position.x) / f32::from(size.width))
                            .clamp(0.0, 1.0);
                        crate::drag_endpoint::send(
                            &app_id,
                            crate::drag_endpoint::Phase::Hover(fraction),
                        );
                        this.dock_drag = Some(app_id);
                    } else if this.dock_drag.take().is_some() {
                        crate::drag_endpoint::send(&app_id, crate::drag_endpoint::Phase::Cancel);
                    }
                },
            ))
            // The drop itself is only meaningful once the pointer has left
            // Apps' own window (see the comment above `on_drag_move`), so
            // this listens for release *outside* the root hitbox, not a
            // release inside it.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, _cx| {
                    let Some(app_id) = this.dock_drag.take() else {
                        return;
                    };
                    let size = window.viewport_size();
                    let fraction =
                        (f32::from(event.position.x) / f32::from(size.width)).clamp(0.0, 1.0);
                    crate::drag_endpoint::send(
                        &app_id,
                        crate::drag_endpoint::Phase::Drop(fraction),
                    );
                }),
            )
            .size_full()
            .v_flex()
            .bg(mac::material_popover())
            .rounded(px(mac::radius_large_surface()))
            .overflow_hidden()
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
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
                    .px_5()
                    .pb_5()
                    .children(recents)
                    .child(body),
            )
            .when_some(context_menu, |element: Div, (menu, state)| {
                element.child(menu.render(&state))
            })
    }
}
