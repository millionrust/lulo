mod mode_panel;
mod results;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, svg, AnyElement, BoxShadow, Context, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tooltip::Tooltip;
use gpui_component::StyledExt as _;
use gpui_component::{Icon, IconName};
use rmac_launcher::{ActivationMode, ApplicationGroup, Category};
use rmac_launcher_runtime::accessibility::{
    visible_phase_label, APPLICATIONS_SECTION_NAME, QUERY_ID, QUERY_NAME, RESULTS_ID,
    SEARCH_SCOPE_DESCRIPTION, SUGGESTIONS_SECTION_NAME,
};
use rmac_launcher_runtime::{KeyCommand, Phase, Row};
use rmac_ui::{mac, Button, TextField};

use super::completion;
use super::panel::PanelMode;
use super::{ApplicationView, BrowseMode, LauncherView};

/// Spotlight geometry in logical points, measured from macOS 26.2 dark
/// (`design-lab/spotlight.html` records every source value). Values marked
/// S are estimates until a Tahoe results-list capture exists.
mod metrics {
    use rmac_launcher::surface;

    /// Transparent margin around the shapes, holding their shadows.
    pub(super) const GUTTER: f32 = surface::SHADOW_GUTTER as f32;
    pub(super) const BAR_HEIGHT: f32 = surface::BAR_HEIGHT as f32;
    /// The idle capsule; it widens to the whole group while typing.
    pub(super) const PILL_WIDTH: f32 = 384.0;
    pub(super) const GROUP_WIDTH: f32 = surface::GROUP_WIDTH as f32;
    /// The light rim sits inside each shape's outer edge.
    pub(super) const RIM: f32 = 1.0;
    pub(super) const CIRCLE: f32 = 54.0;
    pub(super) const CIRCLE_GAP: f32 = 10.0;
    /// Quick-action glyph box; the ink is ≈ 22 × 20 inside it.
    pub(super) const CIRCLE_GLYPH: f32 = 28.0;
    pub(super) const SEARCH_GLYPH: f32 = 22.5;
    pub(super) const SEARCH_GLYPH_LEFT: f32 = 20.5;
    /// Placeholder and query text origin from the capsule's outer edge.
    pub(super) const TEXT_LEFT: f32 = 60.0;
    pub(super) const TEXT_SIZE: f32 = 26.0;
    pub(super) const TEXT_LINE: f32 = 32.0;
    pub(super) const TEXT_TRAIL: f32 = 20.0;
    pub(super) const COMPLETION_HEIGHT: f32 = 32.0;
    pub(super) const COMPLETION_RADIUS: f32 = 6.0;
    pub(super) const COMPLETION_TRAIL: f32 = 8.0;
    pub(super) const TOP_HIT_ICON: f32 = 26.0;
    pub(super) const TOP_HIT_RIGHT: f32 = 23.0;
    pub(super) const MODE_TOKEN_HEIGHT: f32 = 28.0;
    /// S: results card below the bar.
    pub(super) const RESULTS_GAP: f32 = 8.0;
    pub(super) const RESULTS_RADIUS: f32 = 24.0;
    pub(super) const RESULTS_PADDING: f32 = 8.0;
    /// The card ends where Spotlight's 576 pt window ends.
    pub(super) const RESULTS_MAX_HEIGHT: f32 =
        (surface::WINDOW_HEIGHT - surface::BAR_HEIGHT) as f32 - RESULTS_GAP;
    pub(super) const CATEGORY_BAR_HEIGHT: f32 = 40.0;
    pub(super) const ERROR_HEIGHT: f32 = 32.0;
    /// S: rows and section headers.
    pub(super) const SECTION_HEIGHT: f32 = 24.0;
    pub(super) const ROW_HEIGHT: f32 = 32.0;
    pub(super) const ROW_ICON: f32 = 22.0;
    pub(super) const ROW_RADIUS: f32 = 10.0;
    pub(super) const ROW_INSET: f32 = 10.0;
    pub(super) const ROW_GAP: f32 = 10.0;
    pub(super) const ROW_TEXT: f32 = 13.0;
    pub(super) const ROW_TITLE_MAX: f32 = 380.0;
}

/// The measured colours are for dark appearance; light appearance keeps the
/// same structure with light glass.
fn is_dark() -> bool {
    mac::text().l > 0.5
}

/// Spotlight glass. The surface is transparent (compositor blur would be one
/// square behind every shape), so each shape carries its own material: a
/// 1 pt light rim, a 0.5 pt dark hairline outside it, a dark tint that lands
/// on the measured (22, 23, 27) over a dark backdrop, and a soft shadow.
fn glass<E: Styled>(element: E, radius: f32) -> E {
    let (fill, rim, hairline, shadow) = if is_dark() {
        (
            Hsla::from(gpui::rgba(0x1d1e23bf)),
            gpui::hsla(0.0, 0.0, 1.0, 0.32),
            gpui::hsla(0.0, 0.0, 0.0, 0.85),
            gpui::hsla(0.0, 0.0, 0.0, 0.30),
        )
    } else {
        (
            Hsla::from(gpui::rgba(0xf6f6f8d9)),
            gpui::hsla(0.0, 0.0, 1.0, 0.60),
            gpui::hsla(0.0, 0.0, 0.0, 0.12),
            gpui::hsla(0.0, 0.0, 0.0, 0.15),
        )
    };
    element
        .rounded(px(radius))
        .border_1()
        .border_color(rim)
        .bg(fill)
        .shadow(vec![
            BoxShadow::new(px(0.0), px(0.0), hairline).spread_radius(px(0.5)),
            BoxShadow::new(px(0.0), px(3.0), shadow).blur_radius(px(12.0)),
        ])
}

fn glass_hover() -> Hsla {
    if is_dark() {
        Hsla::from(gpui::rgba(0x2b2c32d0))
    } else {
        Hsla::from(gpui::rgba(0xffffffe6))
    }
}

/// The plate behind the inline completion and the browse-mode token: the
/// measured white 20 % over the dark capsule.
fn plate() -> Hsla {
    if is_dark() {
        gpui::hsla(0.0, 0.0, 1.0, 0.20)
    } else {
        gpui::hsla(0.0, 0.0, 0.0, 0.10)
    }
}

#[derive(Clone, Copy)]
enum QuickTarget {
    Browse(BrowseMode),
    Panel(PanelMode),
}

/// A text layer placed exactly over the query field's text.
fn text_overlay() -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .h_full()
        .flex()
        .items_center()
        .whitespace_nowrap()
}

fn mode_token(mode: BrowseMode) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(metrics::MODE_TOKEN_HEIGHT))
        .mr(px(8.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(metrics::MODE_TOKEN_HEIGHT / 2.0))
        .bg(plate())
        .text_size(rmac_ui::text_px(13.0))
        .font_weight(mac::MEDIUM)
        .text_color(mac::text())
        .child(match mode {
            BrowseMode::Applications => "Applications",
            BrowseMode::Files => "Files",
        })
}

impl LauncherView {
    /// One of the four circular quick actions beside the idle capsule
    /// (Apps, Files, Actions, Clipboard). As on macOS 26.2 they appear only
    /// while the pointer is over the bar.
    fn quick_action(
        &self,
        id: &'static str,
        glyph: &'static str,
        tooltip: &'static str,
        target: QuickTarget,
        cx: &Context<Self>,
    ) -> AnyElement {
        glass(
            div()
                .id(id)
                .size(px(metrics::CIRCLE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer(),
            metrics::CIRCLE / 2.0,
        )
        .hover(|style| style.bg(glass_hover()))
        .child(
            svg()
                .path(glyph)
                .size(px(metrics::CIRCLE_GLYPH))
                .text_color(mac::text_tertiary()),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
        .on_click(cx.listener(move |this, _, window, cx| match target {
            QuickTarget::Browse(mode) => this.open_browse(mode, window, cx),
            QuickTarget::Panel(mode) => this.open_panel(mode, window, cx),
        }))
        .into_any_element()
    }

    /// The query text layer shared by the capsule and the mode panel:
    /// placeholder, typed text, and the top row's inline completion on its
    /// plate (shown only while the caret is at the end).
    pub(super) fn query_field(
        &self,
        query: &str,
        placeholder: &'static str,
        completion_label: Option<String>,
        activating: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let caret_at_end = {
            let state = self.query.read(cx);
            let selection = state.selected_range();
            selection.is_empty() && selection.end == state.value().len()
        };
        let completion_label = completion_label.filter(|_| caret_at_end);
        let text_size = rmac_ui::text_px(metrics::TEXT_SIZE);
        let line_height = px(metrics::TEXT_LINE);
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .text_size(text_size)
            .line_height(line_height)
            .when(query.is_empty(), |text| {
                text.child(
                    text_overlay()
                        .text_color(mac::text_tertiary())
                        .child(placeholder),
                )
            })
            .when_some(completion_label, |text, label| {
                text.child(
                    text_overlay()
                        // Invisible copy of the query so the plate starts
                        // exactly where the typed text ends.
                        .child(
                            div()
                                .text_color(gpui::transparent_black())
                                .child(query.to_owned()),
                        )
                        .child(
                            div()
                                .h(px(metrics::COMPLETION_HEIGHT))
                                .flex()
                                .items_center()
                                .pr(px(metrics::COMPLETION_TRAIL))
                                .rounded(px(metrics::COMPLETION_RADIUS))
                                .bg(plate())
                                .text_color(mac::text().opacity(0.73))
                                .child(label),
                        ),
                )
            })
            .child(
                div().id(QUERY_ID).size_full().child(
                    TextField::new(&self.query)
                        .appearance(false)
                        .disabled(activating)
                        .px_0()
                        .py_0()
                        .h(px(metrics::BAR_HEIGHT - 2.0 * metrics::RIM))
                        .text_size(text_size)
                        .line_height(line_height),
                ),
            )
            .into_any_element()
    }

    /// The search capsule: glyph, placeholder or query with the top hit's
    /// inline completion, and the top hit's icon at the right end.
    fn search_bar(
        &self,
        query: &str,
        rows: &[Row],
        activating: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let compact = self.compact;
        // The circles show only while the pointer is over the bar; without
        // them the idle capsule spans the whole group (measured on 26.2).
        let circles = compact && self.bar_hovered;
        let pill_width = if circles {
            metrics::PILL_WIDTH
        } else {
            metrics::GROUP_WIDTH
        };
        let top_hit = if query.is_empty() { None } else { rows.first() };
        let completion_label = top_hit.and_then(|row| {
            completion::inline_completion(query, &row.title)
                .map(|suffix| completion::completion_label(suffix, row.primary_label))
        });
        let trailing = if top_hit.is_some() {
            metrics::TOP_HIT_RIGHT
        } else {
            metrics::TEXT_TRAIL
        };
        let text = self.query_field(query, QUERY_NAME, completion_label, activating, cx);

        let pill = glass(
            div()
                .relative()
                .w(px(pill_width))
                .h(px(metrics::BAR_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .pl(px(metrics::TEXT_LEFT - metrics::RIM))
                .pr(px(trailing - metrics::RIM)),
            metrics::BAR_HEIGHT / 2.0,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
        .child(
            svg()
                .path("spotlight/search.svg")
                .absolute()
                .left(px(metrics::SEARCH_GLYPH_LEFT - metrics::RIM))
                .top(px(
                    (metrics::BAR_HEIGHT - metrics::SEARCH_GLYPH) / 2.0 - metrics::RIM
                ))
                .size(px(metrics::SEARCH_GLYPH))
                .text_color(mac::text_tertiary()),
        )
        .when_some(self.browse_mode, |pill, mode| pill.child(mode_token(mode)))
        .child(text)
        .when(self.browse_mode == Some(BrowseMode::Applications), |pill| {
            pill.child(
                Button::new("spotlight-apps-more", "")
                    .icon(Icon::new(IconName::Ellipsis).text_color(mac::text()))
                    .ghost()
                    .tooltip("More")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.application_options_open = !this.application_options_open;
                        cx.notify();
                    })),
            )
        })
        .when_some(top_hit, |pill, row| {
            pill.child(
                div()
                    .flex_none()
                    .ml(px(8.0))
                    .child(Self::result_icon(row, metrics::TOP_HIT_ICON)),
            )
        });

        div()
            .id("spotlight-bar")
            .flex_none()
            .h(px(metrics::BAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(metrics::CIRCLE_GAP))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.bar_hovered != *hovered {
                    this.bar_hovered = *hovered;
                    cx.notify();
                }
            }))
            .child(pill)
            .when(circles, |bar| {
                bar.child(self.quick_action(
                    "spotlight-applications",
                    "spotlight/apps.svg",
                    "Apps (Command-1)",
                    QuickTarget::Browse(BrowseMode::Applications),
                    cx,
                ))
                .child(self.quick_action(
                    "spotlight-files",
                    "spotlight/folder.svg",
                    "Files (Command-2)",
                    QuickTarget::Browse(BrowseMode::Files),
                    cx,
                ))
                .child(self.quick_action(
                    "spotlight-actions",
                    "spotlight/shortcuts.svg",
                    "Actions (Command-3)",
                    QuickTarget::Panel(PanelMode::Actions),
                    cx,
                ))
                .child(self.quick_action(
                    "spotlight-clipboard",
                    "spotlight/clipboard.svg",
                    "Clipboard (Command-4)",
                    QuickTarget::Panel(PanelMode::Clipboard),
                    cx,
                ))
            })
            .into_any_element()
    }

    /// The results card below the bar. S: no Tahoe results capture yet.
    fn results_card(
        &self,
        rows: &[Row],
        query: &str,
        groups: &[ApplicationGroup],
        phase_message: SharedString,
        cx: &Context<Self>,
    ) -> AnyElement {
        let applications = self.browse_mode == Some(BrowseMode::Applications);
        let mut list_height =
            metrics::RESULTS_MAX_HEIGHT - 2.0 * (metrics::RESULTS_PADDING + metrics::RIM);
        if applications {
            list_height -= metrics::CATEGORY_BAR_HEIGHT;
        }
        if self.settings_error.is_some() {
            list_height -= metrics::ERROR_HEIGHT;
        }

        glass(
            div()
                .w(px(metrics::GROUP_WIDTH))
                .max_h(px(metrics::RESULTS_MAX_HEIGHT))
                .flex_none()
                .v_flex()
                .overflow_hidden()
                .p(px(metrics::RESULTS_PADDING)),
            metrics::RESULTS_RADIUS,
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
        .when(applications, |card| {
            card.child(self.application_category_bar(groups, cx))
        })
        .when_some(self.settings_error.clone(), |card, error| {
            card.child(
                div()
                    .flex_none()
                    .h(px(metrics::ERROR_HEIGHT - 4.0))
                    .mb(px(4.0))
                    .px(px(metrics::ROW_INSET))
                    .flex()
                    .items_center()
                    .rounded(px(metrics::ROW_RADIUS))
                    .bg(mac::warning_background())
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::warning_text())
                    .child(error),
            )
        })
        .when(!rows.is_empty(), |card| {
            card.child(
                div()
                    .id(RESULTS_ID)
                    .max_h(px(list_height))
                    .overflow_y_scroll()
                    .child(self.results(rows, query, cx)),
            )
        })
        .when(rows.is_empty(), |card| {
            card.child(
                div()
                    .id(RESULTS_ID)
                    .py(px(10.0))
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(metrics::ROW_TEXT))
                            .font_weight(mac::MEDIUM)
                            .text_color(mac::text_secondary())
                            .child(phase_message),
                    )
                    .when(self.browse_mode.is_some(), |empty| {
                        empty.child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_tertiary())
                                .child(SEARCH_SCOPE_DESCRIPTION),
                        )
                    }),
            )
        })
        .into_any_element()
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
            .h(px(metrics::CATEGORY_BAR_HEIGHT))
            .flex()
            .items_center()
            .gap_2()
            .px(px(2.0))
            .overflow_x_scrollbar()
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

        glass(
            div()
                .absolute()
                .top(px(metrics::GUTTER + metrics::BAR_HEIGHT + 6.0))
                .right(px(metrics::GUTTER + 8.0))
                .w(px(168.0))
                .p_1()
                .text_color(mac::text()),
            mac::radius_card(),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
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
        let application_groups = ApplicationGroup::ORDER
            .into_iter()
            .filter(|group| {
                snapshot.rows.iter().any(|row| {
                    row.category == Category::Applications && row.application_group == Some(*group)
                })
            })
            .collect::<Vec<_>>();
        let rows = self.visible_rows();
        let query = snapshot.query.clone();
        let compact = self.compact;

        div()
            .relative()
            .size_full()
            .p(px(metrics::GUTTER))
            .v_flex()
            .gap(px(metrics::RESULTS_GAP))
            .text_color(mac::text())
            // Presses that reach the surface itself missed every shape:
            // Spotlight closes, as on macOS.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if std::mem::take(&mut this.press_inside) {
                        return;
                    }
                    if this.application_options_open {
                        this.application_options_open = false;
                        cx.notify();
                        return;
                    }
                    this.dismiss(window, cx);
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
                    let target = match event.keystroke.key.as_str() {
                        "1" => Some(QuickTarget::Browse(BrowseMode::Applications)),
                        "2" => Some(QuickTarget::Browse(BrowseMode::Files)),
                        "3" => Some(QuickTarget::Panel(PanelMode::Actions)),
                        "4" => Some(QuickTarget::Panel(PanelMode::Clipboard)),
                        _ => None,
                    };
                    match target {
                        Some(QuickTarget::Browse(mode)) => {
                            cx.stop_propagation();
                            this.open_browse(mode, window, cx);
                            return;
                        }
                        Some(QuickTarget::Panel(mode)) => {
                            cx.stop_propagation();
                            this.open_panel(mode, window, cx);
                            return;
                        }
                        None => {}
                    }
                    if event.keystroke.key == "backspace" && this.panel.is_some() {
                        cx.stop_propagation();
                        this.remove_clipboard_row(cx);
                        return;
                    }
                }
                if event.keystroke.key == "tab"
                    && !event.keystroke.modifiers.modified()
                    && this.accept_completion(window, cx)
                {
                    cx.stop_propagation();
                    return;
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
            .when(self.panel.is_none(), |surface| {
                surface.child(self.search_bar(&query, &rows, activating, cx))
            })
            .when(self.panel.is_some(), |surface| {
                surface.child(self.mode_panel(&query, cx))
            })
            .when(!compact && self.panel.is_none(), |surface| {
                surface.child(self.results_card(
                    &rows,
                    &query,
                    &application_groups,
                    phase_message,
                    cx,
                ))
            })
            .when(self.application_options_open, |surface| {
                surface.child(self.application_options(cx))
            })
    }
}
