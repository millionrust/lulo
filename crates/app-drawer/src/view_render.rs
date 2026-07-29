use gpui::{
    div, img, prelude::FluentBuilder as _, px, svg, Context, Div, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{mac, EmptyState, SearchField};

use crate::catalog::{App, Category};
use crate::{
    AppDrawer, ClearSearch, Launch, LaunchDesktopAction, MoveDown, MoveLeft, MoveRight, MoveUp,
    OpenApp, RevealInFinder, ViewMode, ICON, ROW_ICON, TILE_W,
};

impl AppDrawer {
    fn icon_element(&self, app: &App, size: f32) -> gpui::AnyElement {
        match &app.icon {
            Some(path) => img(path.clone()).w(px(size)).h(px(size)).into_any_element(),
            None => {
                let initial = app
                    .name
                    .chars()
                    .next()
                    .map(|character| character.to_uppercase().to_string())
                    .unwrap_or_default();
                div()
                    .w(px(size))
                    .h(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(size * 0.23))
                    .bg(mac::control_fill())
                    .text_color(mac::text_secondary())
                    .text_size(rmac_ui::text_px(size * 0.43))
                    .child(initial)
                    .into_any_element()
            }
        }
    }

    /// `position` is the index in the visible projection tracked by selection.
    fn tile(
        &self,
        app: &App,
        position: usize,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let launch = app.launch.clone();
        div()
            .id(SharedString::from(format!("app-{}", app.path.display())))
            .w(px(TILE_W))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .px_1()
            .py_2()
            .rounded(px(10.0))
            .when(selected, |element: Stateful<Div>| {
                element
                    .bg(mac::accent_subtle())
                    .border_1()
                    .border_color(mac::accent_border())
            })
            .when(!selected, |element: Stateful<Div>| {
                element.border_1().border_color(gpui::transparent_black())
            })
            .hover(|hover| hover.bg(mac::hover()))
            .child(self.icon_element(app, ICON))
            .child(
                div()
                    .max_w(px(TILE_W - 8.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text())
                    .text_center()
                    .truncate()
                    .child(app.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = position;
                this.launch_application(launch.clone(), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.selected = position;
                    this.menu_at = Some(event.position);
                    cx.notify();
                }),
            )
    }

    fn row(
        &self,
        app: &App,
        position: usize,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let launch = app.launch.clone();
        div()
            .id(SharedString::from(format!("row-{}", app.path.display())))
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .px_3()
            .py_1p5()
            .rounded(px(8.0))
            .when(selected, |element: Stateful<Div>| {
                element.bg(mac::accent_subtle())
            })
            .when(!selected, |element: Stateful<Div>| {
                element.hover(|hover| hover.bg(mac::hover()))
            })
            .child(self.icon_element(app, ROW_ICON))
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text())
                    .truncate()
                    .child(app.name.clone()),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_secondary())
                    .child(app.category.label()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = position;
                this.launch_application(launch.clone(), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.selected = position;
                    this.menu_at = Some(event.position);
                    cx.notify();
                }),
            )
    }

    fn view_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let segment = |id: &'static str, glyph: &'static str, mode: ViewMode, active: bool| {
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |element: Stateful<Div>| element.bg(mac::raised()))
                .child(
                    svg()
                        .path(glyph)
                        .w(px(15.0))
                        .h(px(15.0))
                        .text_color(if active {
                            mac::text()
                        } else {
                            mac::text_secondary()
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.view = mode;
                    cx.notify();
                }))
        };
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(mac::control_fill())
            .child(segment(
                "v-grid",
                "icons/layout-dashboard.svg",
                ViewMode::Grid,
                self.view == ViewMode::Grid,
            ))
            .child(segment(
                "v-list",
                "icons/menu.svg",
                ViewMode::List,
                self.view == ViewMode::List,
            ))
    }

    fn category_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let present = self.present_categories(cx);
        let pill =
            |id: SharedString, label: SharedString, active: bool, target: Option<Category>| {
                div()
                    .id(id)
                    .px_3()
                    .py_1()
                    .rounded(px(13.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .when(active, |element: Stateful<Div>| {
                        element.bg(mac::accent()).text_color(mac::on_accent())
                    })
                    .when(!active, |element: Stateful<Div>| {
                        element
                            .bg(mac::control_fill())
                            .text_color(mac::text())
                            .hover(|hover| hover.bg(mac::control_fill_hover()))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_filter(target, cx);
                    }))
            };

        let mut bar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .justify_center()
            .px_8()
            .pb_2()
            .child(pill(
                "cat-all".into(),
                "All".into(),
                self.filter.is_none(),
                None,
            ));
        for category in present {
            bar = bar.child(pill(
                SharedString::from(format!("cat-{}", category.label())),
                category.label().into(),
                self.filter == Some(category),
                Some(category),
            ));
        }
        bar
    }
}

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport_width = f32::from(window.viewport_size().width);
        let usable_width = (viewport_width - 64.0).max(TILE_W);
        self.cols = ((usable_width / (TILE_W + 8.0)).floor() as usize).max(1);

        let visible = self.visible_indices(cx);
        let selected = self.selected.min(visible.len().saturating_sub(1));
        let notice = if self.launching {
            Some((SharedString::from("Opening application…"), false))
        } else {
            self.action_error
                .clone()
                .or_else(|| self.catalog_error.clone())
                .map(|message| (message, true))
        };
        let context_menu = self.menu_at.map(|position| self.app_menu(position, cx));

        let body: gpui::AnyElement = if visible.is_empty() {
            let empty = if self.apps.is_empty() {
                EmptyState::new("No applications found").message(
                    "Install an application or add a visible desktop entry to an XDG application directory",
                )
            } else {
                EmptyState::new("No matching applications")
                    .message("Try another name, keyword, category, or application action")
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
                    self.tile(&self.apps[index], position, position == selected, cx)
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
                    self.row(&self.apps[index], position, position == selected, cx)
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
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.dismiss(window, cx);
            }))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Applications"))
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
                        .when(is_error, |element| element.cursor_pointer())
                        .child(div().flex_1().child(message))
                        .when(is_error, |element| element.child("Dismiss"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.launching {
                                this.catalog_error = None;
                                this.action_error = None;
                                cx.notify();
                            }
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_8()
                    .py_4()
                    .child(div().w(px(34.0)))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .child(div().w(px(280.0)).child(SearchField::new(&self.query))),
                    )
                    .child(self.view_toggle(cx)),
            )
            .child(self.category_bar(cx))
            .child(
                div()
                    .id("grid-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_8()
                    .pb_8()
                    .child(body),
            )
            .when_some(context_menu, |element: Div, menu| {
                element.child(menu.render())
            })
    }
}
