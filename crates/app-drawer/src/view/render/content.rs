//! Apps icon, grid/list item, category, and view-toggle projection.

use super::*;

const ICON_INSET: f32 = 4.0;
// design-lab/tokens.css --icon-radius, expressed as a fraction of the plate.
const ICON_PLATE_RADIUS_RATIO: f32 = 0.225;

fn icon_plate_edge(size: f32) -> f32 {
    (size - ICON_INSET).max(0.0)
}

fn third_party_art_edge(size: f32) -> f32 {
    let scale = rmac_icon::third_party_plate(rmac_icon::IconShape::Other).unwrap_or(1.0);
    icon_plate_edge(size) * scale
}

impl AppDrawer {
    pub(super) fn icon_element(&self, app: &App, size: f32) -> gpui::AnyElement {
        let plate_edge = icon_plate_edge(size);
        let frame = div()
            .w(px(size))
            .h(px(size))
            .flex()
            .items_center()
            .justify_center();

        if app.first_party {
            return frame
                .child(match &app.icon {
                    Some(path) => img(path.clone())
                        .w(px(plate_edge))
                        .h(px(plate_edge))
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => fallback_icon(app, plate_edge, size),
                })
                .into_any_element();
        }

        let art_edge = third_party_art_edge(size);
        let content = match &app.icon {
            Some(path) => img(path.clone())
                .w(px(art_edge))
                .h(px(art_edge))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => fallback_icon(app, art_edge, size),
        };
        frame
            .child(
                div()
                    .w(px(plate_edge))
                    .h(px(plate_edge))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(plate_edge * ICON_PLATE_RADIUS_RATIO))
                    .bg(mac::icon_plate())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_md()
                    .overflow_hidden()
                    .child(content),
            )
            .into_any_element()
    }

    /// `position` is the index in the visible projection tracked by selection.
    pub(super) fn tile(
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
            .py_1p5()
            .rounded(px(mac::radius_menu()))
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
                    .w(px(TILE_W - 8.0))
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
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.selected = position;
                    this.menu_at = Some(rmac_ui::ContextMenuState::open(
                        event.position,
                        &this.focus,
                        window,
                        cx,
                    ));
                    cx.notify();
                }),
            )
    }

    pub(super) fn row(
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
            .rounded(px(mac::radius_control()))
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
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.selected = position;
                    this.menu_at = Some(rmac_ui::ContextMenuState::open(
                        event.position,
                        &this.focus,
                        window,
                        cx,
                    ));
                    cx.notify();
                }),
            )
    }

    pub(super) fn view_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let segment = |id: &'static str, glyph: &'static str, mode: ViewMode, active: bool| {
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(mac::radius_menu_item()))
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
            .rounded(px(mac::radius_segmented()))
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

    pub(super) fn category_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let present = self.present_categories(cx);
        let pill =
            |id: SharedString, label: SharedString, active: bool, target: Option<Category>| {
                div()
                    .id(id)
                    .px_3()
                    .py_1()
                    .rounded(px(mac::radius_card()))
                    .text_size(rmac_ui::text_px(12.0))
                    .when(active, |element: Stateful<Div>| {
                        element
                            .bg(mac::accent_subtle())
                            .border_1()
                            .border_color(mac::accent_border())
                            .text_color(mac::text())
                    })
                    .when(!active, |element: Stateful<Div>| {
                        element
                            .bg(mac::control_fill())
                            .border_1()
                            .border_color(gpui::transparent_black())
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
            .px_5()
            .py_3()
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

fn fallback_icon(app: &App, edge: f32, text_basis: f32) -> gpui::AnyElement {
    let initial = app
        .name
        .chars()
        .next()
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .w(px(edge))
        .h(px(edge))
        .flex()
        .items_center()
        .justify_center()
        .text_color(mac::text_secondary())
        .text_size(rmac_ui::text_px(text_basis * 0.43))
        .child(initial)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{icon_plate_edge, third_party_art_edge};

    #[test]
    fn third_party_art_uses_the_measured_seventy_six_percent_scale() {
        assert_eq!(icon_plate_edge(54.0), 50.0);
        assert!((third_party_art_edge(54.0) - 38.0).abs() < f32::EPSILON);
    }
}
