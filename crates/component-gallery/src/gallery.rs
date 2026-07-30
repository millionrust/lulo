//! Component Gallery interaction controller and root presentation.

use crate::specimens::render_specimen;

use gpui::{
    div, prelude::FluentBuilder as _, px, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{
    gallery::{ComponentSpec, PreviewScale, COMPONENT_SPECS, PREVIEW_SCALES},
    mac,
};

gpui::actions!(
    component_gallery,
    [Scale100, Scale150, Scale200, PreviousScale, NextScale]
);

const KEY_CONTEXT: &str = "ComponentGallery";

pub(crate) struct ComponentGallery {
    focus: FocusHandle,
    scale_index: usize,
}

impl ComponentGallery {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("1", Scale100, Some(KEY_CONTEXT)),
            KeyBinding::new("2", Scale150, Some(KEY_CONTEXT)),
            KeyBinding::new("3", Scale200, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd--", PreviousScale, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl--", PreviousScale, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-=", NextScale, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-=", NextScale, Some(KEY_CONTEXT)),
        ]);
        let focus = cx.focus_handle();
        focus.focus(window);
        Self {
            focus,
            scale_index: 0,
        }
    }

    fn scale(&self) -> PreviewScale {
        PREVIEW_SCALES[self.scale_index]
    }

    fn set_scale(&mut self, index: usize, cx: &mut Context<Self>) {
        self.scale_index = index.min(PREVIEW_SCALES.len() - 1);
        cx.notify();
    }

    fn previous_scale(&mut self, cx: &mut Context<Self>) {
        self.scale_index = self.scale_index.saturating_sub(1);
        cx.notify();
    }

    fn next_scale(&mut self, cx: &mut Context<Self>) {
        self.scale_index = (self.scale_index + 1).min(PREVIEW_SCALES.len() - 1);
        cx.notify();
    }

    fn render_scale_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .children(PREVIEW_SCALES.iter().enumerate().map(|(index, scale)| {
                let selected = index == self.scale_index;
                div()
                    .id(("gallery-scale", index))
                    .h(px(28.0))
                    .min_w(px(62.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(if selected {
                        mac::accent_border()
                    } else {
                        mac::separator()
                    })
                    .bg(if selected {
                        mac::accent()
                    } else {
                        mac::control_fill()
                    })
                    .text_color(if selected {
                        mac::on_accent()
                    } else {
                        mac::text()
                    })
                    .text_size(px(12.0))
                    .font_weight(mac::MEDIUM)
                    .cursor_pointer()
                    .hover(|style| style.bg(mac::control_fill_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_scale(index, cx)))
                    .child(format!("{}  [{}]", scale.label, index + 1))
            }))
    }

    fn render_spec(&self, spec: &ComponentSpec) -> impl IntoElement {
        let scale = self.scale();
        let keyboard = spec.keyboard.map(|behavior| {
            format!(
                "Enter: {}  •  Operate: {}  •  Exit: {}",
                behavior.enter, behavior.operate, behavior.exit
            )
        });

        div()
            .v_flex()
            .gap_3()
            .p_4()
            .rounded(px(12.0))
            .bg(mac::raised())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .child(spec.component.label()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(mac::text_tertiary())
                                    .child(spec.component.id()),
                            ),
                    )
                    .when_some(keyboard, |row, keyboard| {
                        row.child(
                            div()
                                .max_w(px(620.0))
                                .text_size(px(11.0))
                                .text_color(mac::text_secondary())
                                .child(keyboard),
                        )
                    }),
            )
            .child(div().flex().flex_wrap().items_start().gap_3().children(
                spec.states.iter().copied().map(|state| {
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(mac::MEDIUM)
                                .text_color(mac::text_tertiary())
                                .child(state.label()),
                        )
                        .child(render_specimen(spec.component, state, scale))
                }),
            ))
    }
}

impl Render for ComponentGallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Scale100, _, cx| this.set_scale(0, cx)))
            .on_action(cx.listener(|this, _: &Scale150, _, cx| this.set_scale(1, cx)))
            .on_action(cx.listener(|this, _: &Scale200, _, cx| this.set_scale(2, cx)))
            .on_action(cx.listener(|this, _: &PreviousScale, _, cx| this.previous_scale(cx)))
            .on_action(cx.listener(|this, _: &NextScale, _, cx| this.next_scale(cx)))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("rmac Component Gallery"))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_5()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(mac::separator())
                    .bg(mac::chrome())
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .child("Shared control states"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(mac::text_secondary())
                                    .child(
                                        "Deterministic logical previews; validate native scaling on Linux hardware",
                                    ),
                            ),
                    )
                    .child(self.render_scale_selector(cx)),
            )
            .child(
                div()
                    .id("component-gallery-scroll")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .v_flex()
                            .gap_4()
                            .p_5()
                            .children(COMPONENT_SPECS.iter().map(|spec| self.render_spec(spec))),
                    ),
            )
            .child(
                div()
                    .h(px(34.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .border_t_1()
                    .border_color(mac::separator())
                    .bg(mac::chrome())
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} components • {} state specimens • {} preview",
                        COMPONENT_SPECS.len(),
                        COMPONENT_SPECS
                            .iter()
                            .map(|spec| spec.states.len())
                            .sum::<usize>(),
                        self.scale().label
                    ))
                    .child("Keys 1/2/3 change scale • Ctrl/⌘ −/= steps scale"),
            )
    }
}
