mod content;

use gpui::{
    div, img, prelude::FluentBuilder as _, px, svg, Context, Div, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_app_drawer::accessibility::{DrawerEmptyState, OPENING_ANNOUNCEMENT};
use rmac_ui::{mac, Button, EmptyState, SearchField};

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

        let body: gpui::AnyElement = if visible.is_empty() {
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
                this.launch_application(action.launch.clone(), cx);
            }))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| {
                this.reveal_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| {
                this.clear_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                if rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window) {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.dismiss(window, cx);
            }))
            .size_full()
            .v_flex()
            .bg(mac::material())
            .rounded(px(mac::radius_large_surface()))
            .border_1()
            .border_color(mac::separator())
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
                    .child(body),
            )
            .when_some(context_menu, |element: Div, (menu, state)| {
                element.child(menu.render(&state))
            })
    }
}
