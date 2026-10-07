//! The menu bar: the Lulo menu, the front app's name and menus, then the
//! status items, Spotlight and the clock, as on the Mac.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, svg, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
};
use rmac_ui::mac;

use super::{assets, ShellState, BAR_HEIGHT};
use crate::win::trace;

pub(crate) struct BarView {
    shell: Entity<ShellState>,
    title_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    traced: Rc<RefCell<Vec<(SharedString, Bounds<Pixels>)>>>,
    _observe: Subscription,
}

/// The bar's tint: the regular material, opaque enough to read over any
/// wallpaper without a compositor blur behind it.
pub(crate) fn bar_fill() -> gpui::Hsla {
    let mut fill = mac::material();
    fill.a = fill.a.max(0.86);
    fill
}

impl BarView {
    pub(crate) fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shell = super::shell(cx);
        let observe = cx.observe(&shell, |_, _, cx| cx.notify());
        Self {
            shell,
            title_bounds: Rc::new(RefCell::new(Vec::new())),
            traced: Rc::new(RefCell::new(Vec::new())),
            _observe: observe,
        }
    }

    /// Where title `index` sits, in the bar's own coordinates.
    fn title_left(&self, index: usize) -> f32 {
        self.title_bounds
            .borrow()
            .get(index)
            .map_or(0.0, |bounds| f32::from(bounds.left()))
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.shell.read(cx).open_menu == Some(index) {
            super::menu::close(true, cx);
        } else {
            let left = self.title_left(index);
            super::menu::open(index, left, cx);
        }
    }
}

fn status_glyph(path: &'static str, label: &'static str) -> impl IntoElement {
    div()
        .id(label)
        .h_full()
        .px(px(7.0))
        .flex()
        .items_center()
        .role(Role::Image)
        .aria_label(label)
        .child(svg().path(path).size(px(15.0)).text_color(mac::text()))
}

impl Render for BarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.shell.read(cx);
        let open = state.open_menu;
        let mut labels: Vec<SharedString> = vec!["Lulo".into()];
        labels.extend(
            state
                .front_menus()
                .into_iter()
                .map(|menu| SharedString::from(menu.label)),
        );
        let titles = labels.iter().cloned().enumerate().map(|(index, label)| {
            let title = div()
                .id(("lulo-bar-title", index))
                .role(Role::MenuItem)
                .aria_label(label.clone())
                .aria_expanded(open == Some(index))
                .h(px(BAR_HEIGHT - 2.0))
                .px(px(if index == 0 { 11.0 } else { 9.0 }))
                .flex()
                .items_center()
                .rounded(px(mac::radius_menu_item()))
                .when(index == 1, |title| title.font_weight(mac::BOLD))
                .when(open == Some(index), |title| title.bg(mac::hover()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |bar, _, _, cx| {
                        cx.stop_propagation();
                        bar.toggle(index, cx);
                    }),
                )
                .on_hover(cx.listener(move |bar, hovered: &bool, _, cx| {
                    // While a menu is open, moving onto another title opens
                    // that one, as in the Mac menu bar.
                    let open = bar.shell.read(cx).open_menu;
                    if *hovered && open.is_some() && open != Some(index) {
                        let left = bar.title_left(index);
                        super::menu::open(index, left, cx);
                    }
                }));
            if index == 0 {
                title.child(
                    svg()
                        .path("status/rmac.svg")
                        .size(px(15.0))
                        .text_color(mac::text()),
                )
            } else {
                title.child(label)
            }
        });
        let bounds = self.title_bounds.clone();
        let traced = self.traced.clone();
        let traced_labels = labels.clone();
        let row = div()
            .h_full()
            .flex()
            .items_center()
            .on_children_prepainted(move |children, window, _| {
                // The CI checks click titles by their place on screen.
                let current = traced_labels
                    .iter()
                    .cloned()
                    .zip(children.iter().copied())
                    .collect::<Vec<_>>();
                if *traced.borrow() != current {
                    let scale = window.scale_factor();
                    for (index, (label, title)) in current.iter().enumerate() {
                        trace(|| {
                            format!(
                                "bar title {index} {label} at {},{},{},{}",
                                (f32::from(title.left()) * scale).round(),
                                (f32::from(title.top()) * scale).round(),
                                (f32::from(title.size.width) * scale).round(),
                                (f32::from(title.size.height) * scale).round()
                            )
                        });
                    }
                    *traced.borrow_mut() = current;
                }
                *bounds.borrow_mut() = children;
            })
            .children(titles);

        let mut status = Vec::new();
        if let Some(battery) = state.battery {
            status.push(
                div()
                    .id("lulo-bar-battery")
                    .h_full()
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .role(Role::Image)
                    .aria_label(SharedString::from(format!("Battery {}%", battery.percent)))
                    .child(format!("{}%", battery.percent))
                    .child(
                        svg()
                            .path(if battery.charging {
                                "symbols/battery-charging.svg"
                            } else {
                                "status/battery.svg"
                            })
                            .size(px(18.0))
                            .text_color(mac::text()),
                    )
                    .into_any_element(),
            );
        }
        if let Some(wifi) = &state.wifi {
            status.push(status_glyph(assets::wifi_glyph(wifi.bars), "Wi-Fi").into_any_element());
        }
        if let Some(volume) = state.volume {
            status.push(
                status_glyph(assets::speaker_glyph(volume.level, volume.muted), "Sound")
                    .into_any_element(),
            );
        }
        status.push(
            div()
                .id("lulo-bar-spotlight")
                .h_full()
                .px(px(7.0))
                .flex()
                .items_center()
                .role(Role::Button)
                .aria_label("Spotlight")
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                    super::spotlight::toggle(cx);
                })
                .child(
                    svg()
                        .path("status/spotlight.svg")
                        .size(px(14.0))
                        .text_color(mac::text()),
                )
                .into_any_element(),
        );
        status.push(
            div()
                .id("lulo-bar-clock")
                .h_full()
                .pl(px(7.0))
                .pr(px(4.0))
                .flex()
                .items_center()
                .role(Role::Label)
                .aria_label(state.clock.clone())
                .child(state.clock.clone())
                .into_any_element(),
        );

        div()
            .id("lulo-menu-bar")
            .role(Role::MenuBar)
            .aria_label("Menu bar")
            .size_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(8.0))
            .bg(bar_fill())
            .text_size(px(13.0))
            .text_color(mac::text())
            .child(row)
            .child(div().h_full().flex().items_center().children(status))
    }
}
