mod results;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, svg, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::StyledExt as _;
use gpui_component::{Icon, IconName};
use rmac_launcher::{ActivationMode, ApplicationGroup, Category};
use rmac_launcher_runtime::accessibility::{
    visible_phase_label, APPLICATIONS_SECTION_NAME, KEYBOARD_HELP, QUERY_ID, RESULTS_ID,
    SEARCH_SCOPE_DESCRIPTION, SUGGESTIONS_SECTION_NAME,
};
use rmac_launcher_runtime::{KeyCommand, Phase, Row};
use rmac_ui::{mac, Button, SearchField};

use super::{ApplicationView, BrowseMode, LauncherView};

impl LauncherView {
    fn browse_button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        mode: Option<BrowseMode>,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id, "")
            .icon(Icon::new(icon).text_color(mac::text_secondary()))
            .tooltip(tooltip)
            .disabled(mode.is_none())
            .w(px(56.0))
            .h(px(56.0))
            .rounded_full()
            .border_1()
            .border_color(mac::separator())
            .bg(mac::material_clear())
            .when_some(mode, |button, mode| {
                button.on_click(
                    cx.listener(move |this, _, window, cx| this.open_browse(mode, window, cx)),
                )
            })
    }

    fn application_category_bar(
        &self,
        groups: &[ApplicationGroup],
        cx: &Context<Self>,
    ) -> AnyElement {
        let pill = |id: SharedString,
                    label: SharedString,
                    group: Option<ApplicationGroup>,
                    selected: bool| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded(px(mac::radius_pill()))
                .border_1()
                .border_color(if selected {
                    mac::accent_border()
                } else {
                    gpui::transparent_black()
                })
                .bg(if selected {
                    mac::accent_subtle()
                } else {
                    mac::control_fill()
                })
                .hover(|hover| hover.bg(mac::control_fill_hover()))
                .text_size(rmac_ui::text_px(11.0))
                .text_color(mac::text())
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_application_group(group, cx);
                }))
                .into_any_element()
        };

        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .overflow_x_scrollbar()
            .border_b_1()
            .border_color(mac::separator())
            .child(pill(
                "apps-category-all".into(),
                "All".into(),
                None,
                self.application_group.is_none(),
            ))
            .children(groups.iter().copied().map(|group| {
                pill(
                    SharedString::from(format!("apps-category-{}", group.label())),
                    group.label().into(),
                    Some(group),
                    self.application_group == Some(group),
                )
            }))
            .into_any_element()
    }

    fn application_options(&self, cx: &Context<Self>) -> AnyElement {
        let option =
            |id: &'static str, label: &'static str, view: ApplicationView, selected: bool| {
                div()
                    .id(id)
                    .h(px(32.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .rounded(px(mac::radius_control()))
                    .hover(|hover| hover.bg(mac::hover()))
                    .text_size(rmac_ui::text_px(12.0))
                    .child(label)
                    .when(selected, |row| row.child("✓"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_application_view(view, cx);
                    }))
            };

        div()
            .absolute()
            .top(px(60.0))
            .right(px(12.0))
            .w(px(168.0))
            .p_1()
            .rounded(px(mac::radius_card()))
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .bg(mac::material())
            .text_color(mac::text())
            .child(option(
                "apps-view-grid",
                "Grid",
                ApplicationView::Grid,
                self.application_view == ApplicationView::Grid,
            ))
            .child(option(
                "apps-view-list",
                "List",
                ApplicationView::List,
                self.application_view == ApplicationView::List,
            ))
            .into_any_element()
    }
}

impl Render for LauncherView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.coordinator.snapshot();
        let phase_message: SharedString = visible_phase_label(&snapshot).into();
        let activating = snapshot.phase == Phase::Activating;
        let browse_rows = snapshot
            .rows
            .iter()
            .filter(|row| {
                self.browse_mode
                    .is_none_or(|mode| row.category == mode.category())
            })
            .cloned()
            .collect::<Vec<_>>();
        let application_groups = ApplicationGroup::ORDER
            .into_iter()
            .filter(|group| {
                browse_rows
                    .iter()
                    .any(|row| row.application_group == Some(*group))
            })
            .collect::<Vec<_>>();
        let rows = browse_rows
            .into_iter()
            .filter(|row| {
                self.browse_mode != Some(BrowseMode::Applications)
                    || self
                        .application_group
                        .is_none_or(|group| row.application_group == Some(group))
            })
            .collect::<Vec<_>>();
        let query = snapshot.query.clone();
        let compact = self.compact;
        let has_rows = !rows.is_empty();

        div()
            .relative()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.application_options_open {
                        this.application_options_open = false;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    if this.application_options_open {
                        this.application_options_open = false;
                        cx.notify();
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.secondary() {
                    let mode = match event.keystroke.key.as_str() {
                        "1" => Some(BrowseMode::Applications),
                        "2" => Some(BrowseMode::Files),
                        _ => None,
                    };
                    if let Some(mode) = mode {
                        cx.stop_propagation();
                        this.open_browse(mode, window, cx);
                        return;
                    }
                }
                if event.keystroke.key == "escape" && this.application_options_open {
                    cx.stop_propagation();
                    this.application_options_open = false;
                    cx.notify();
                    return;
                }
                let command = match event.keystroke.key.as_str() {
                    "down" => Some(KeyCommand::ArrowDown),
                    "up" => Some(KeyCommand::ArrowUp),
                    "enter" => Some(if event.keystroke.modifiers.secondary() {
                        KeyCommand::AlternateReturn
                    } else {
                        KeyCommand::Return
                    }),
                    "escape" => Some(KeyCommand::Escape),
                    _ => None,
                };
                if let Some(command) = command {
                    this.handle_key(command, window, cx);
                }
            }))
            .v_flex()
            .overflow_hidden()
            .text_color(mac::text())
            .when(compact, |surface| {
                surface.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(388.0))
                                .h(px(60.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_4()
                                .rounded(px(mac::radius_pill()))
                                .border_1()
                                .border_color(mac::separator())
                                .shadow_xl()
                                .bg(mac::material())
                                .child(
                                    svg()
                                        .path("icons/search.svg")
                                        .size(px(22.0))
                                        .text_color(mac::text_secondary()),
                                )
                                .child(
                                    div().id(QUERY_ID).flex_1().child(
                                        SearchField::new(&self.query)
                                            .appearance(false)
                                            .disabled(activating)
                                            .text_size(rmac_ui::text_px(20.0)),
                                    ),
                                ),
                        )
                        .child(self.browse_button(
                            "spotlight-applications",
                            IconName::LayoutDashboard,
                            "Apps (Command-1)",
                            Some(BrowseMode::Applications),
                            cx,
                        ))
                        .child(self.browse_button(
                            "spotlight-files",
                            IconName::Folder,
                            "Files (Command-2)",
                            Some(BrowseMode::Files),
                            cx,
                        ))
                        .child(self.browse_button(
                            "spotlight-actions",
                            IconName::Asterisk,
                            "Actions are not available yet",
                            None,
                            cx,
                        ))
                        .child(self.browse_button(
                            "spotlight-clipboard",
                            IconName::Copy,
                            "Clipboard history is not available yet",
                            None,
                            cx,
                        )),
                )
            })
            .when(!compact, |surface| {
                surface
                    .rounded(px(mac::radius_large_surface()))
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_xl()
                    .bg(mac::material())
                    .child(
                        div()
                            .h(px(72.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .border_b_1()
                            .border_color(mac::separator())
                            .child(
                                svg()
                                    .path("icons/search.svg")
                                    .size(px(20.0))
                                    .text_color(mac::text_secondary()),
                            )
                            .child(
                                div().id(QUERY_ID).flex_1().child(
                                    SearchField::new(&self.query)
                                        .appearance(false)
                                        .disabled(activating)
                                        .text_size(rmac_ui::text_px(17.0)),
                                ),
                            )
                            .when(
                                self.browse_mode == Some(BrowseMode::Applications),
                                |header| {
                                    header.child(
                                        Button::new("spotlight-apps-more", "")
                                            .icon(
                                                Icon::new(IconName::Ellipsis)
                                                    .text_color(mac::text()),
                                            )
                                            .ghost()
                                            .tooltip("More")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.application_options_open =
                                                    !this.application_options_open;
                                                cx.notify();
                                            })),
                                    )
                                },
                            ),
                    )
                    .when(
                        self.browse_mode == Some(BrowseMode::Applications),
                        |surface| {
                            surface.child(self.application_category_bar(&application_groups, cx))
                        },
                    )
                    .when_some(self.settings_error.clone(), |surface, error| {
                        surface.child(
                            div()
                                .flex_none()
                                .px_5()
                                .py_2()
                                .bg(mac::warning_background())
                                .border_b_1()
                                .border_color(mac::warning_border())
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::warning_text())
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .id(RESULTS_ID)
                            .flex_1()
                            .overflow_y_scroll()
                            .px_4()
                            .py_3()
                            .when(has_rows, |content| {
                                content.child(self.results(&rows, &query, cx))
                            })
                            .when(!has_rows, |content| {
                                content.child(
                                    div()
                                        .size_full()
                                        .v_flex()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .text_color(mac::text_secondary())
                                        .child(
                                            div()
                                                .text_size(rmac_ui::text_px(15.0))
                                                .font_weight(mac::MEDIUM)
                                                .child(phase_message.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(rmac_ui::text_px(11.0))
                                                .text_color(mac::text_tertiary())
                                                .child(SEARCH_SCOPE_DESCRIPTION),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .h(px(36.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .border_t_1()
                            .border_color(mac::separator())
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(mac::text_tertiary())
                            .child(phase_message)
                            .child(KEYBOARD_HELP),
                    )
                    .when(self.application_options_open, |surface| {
                        surface.child(self.application_options(cx))
                    })
            })
    }
}
