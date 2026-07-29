use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, img, px, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::StyledExt as _;
use rmac_launcher::{ActivationMode, Category};
use rmac_launcher_runtime::{KeyCommand, Phase, Row};
use rmac_ui::{mac, SearchField};

use super::LauncherView;

impl LauncherView {
    fn result_icon(row: &Row, size: f32) -> AnyElement {
        if let Some(icon) = row.icon.clone() {
            return img(icon)
                .size(px(size))
                .rounded(px(size * 0.2))
                .flex_none()
                .into_any_element();
        }
        let (glyph, color): (&str, Hsla) = match row.category {
            Category::Applications => ("A", gpui::rgb(0x0a84ff).into()),
            Category::Settings => ("⚙", gpui::rgb(0x8e8e93).into()),
            Category::Calculator => ("=", gpui::rgb(0xff9f0a).into()),
            Category::Files => ("▤", gpui::rgb(0x30b0c7).into()),
            Category::Other => ("•", gpui::rgb(0x5e5ce6).into()),
        };
        div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.22))
            .bg(color)
            .text_color(mac::on_accent())
            .font_weight(mac::SEMIBOLD)
            .text_size(rmac_ui::text_px(size * 0.42))
            .child(glyph)
            .into_any_element()
    }

    fn grid_tile(&self, row: &Row, index: usize, cx: &Context<Self>) -> AnyElement {
        let id = row.id.clone();
        div()
            .id(SharedString::from(format!("launcher-grid-{index}")))
            .w(px(104.0))
            .h(px(112.0))
            .v_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(px(12.0))
            .cursor_pointer()
            .when(row.selected, |tile| tile.bg(mac::accent_subtle()))
            .when(!row.selected, |tile| {
                tile.hover(|hover| hover.bg(mac::hover()))
            })
            .child(Self::result_icon(row, 54.0))
            .child(
                div()
                    .w_full()
                    .px_1()
                    .text_center()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text())
                    .child(row.title.clone()),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(id.clone(), ActivationMode::Primary, window, cx);
            }))
            .into_any_element()
    }

    fn list_row(&self, row: &Row, index: usize, cx: &Context<Self>) -> AnyElement {
        let primary_id = row.id.clone();
        let alternate_id = row.id.clone();
        div()
            .id(SharedString::from(format!("launcher-row-{index}")))
            .h(px(56.0))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .rounded(px(9.0))
            .cursor_pointer()
            .when(row.selected, |item| item.bg(mac::accent_subtle()))
            .when(!row.selected, |item| {
                item.hover(|hover| hover.bg(mac::hover()))
            })
            .child(Self::result_icon(row, 36.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(mac::MEDIUM)
                            .text_color(mac::text())
                            .child(row.title.clone()),
                    )
                    .when_some(row.subtitle.clone(), |text, subtitle| {
                        text.child(
                            div()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_secondary())
                                .child(subtitle),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(10.0))
                    .text_color(mac::text_tertiary())
                    .child(row.category_label),
            )
            .when(row.has_alternate, |item| {
                item.child(
                    div()
                        .id(SharedString::from(format!("launcher-alternate-{index}")))
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.0))
                        .bg(mac::control_fill())
                        .hover(|hover| hover.bg(mac::control_fill_hover()))
                        .child("•••")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.select_and_activate(
                                alternate_id.clone(),
                                ActivationMode::Alternate,
                                window,
                                cx,
                            );
                        })),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(primary_id.clone(), ActivationMode::Primary, window, cx);
            }))
            .into_any_element()
    }

    fn results(&self, rows: &[Row], query: &str, cx: &Context<Self>) -> AnyElement {
        if query.is_empty() {
            let applications = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.category == Category::Applications)
                .map(|(index, row)| self.grid_tile(row, index, cx))
                .collect::<Vec<_>>();
            let remaining = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.category != Category::Applications)
                .map(|(index, row)| self.list_row(row, index, cx))
                .collect::<Vec<_>>();
            return div()
                .v_flex()
                .gap_3()
                .when(!applications.is_empty(), |content| {
                    content.child(section_label("Applications")).child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .justify_center()
                            .children(applications),
                    )
                })
                .when(!remaining.is_empty(), |content| {
                    content
                        .child(section_label("Suggestions"))
                        .child(div().v_flex().gap_0p5().children(remaining))
                })
                .into_any_element();
        }

        let mut content = div().v_flex().gap_2();
        let mut last_category = None;
        for (index, row) in rows.iter().enumerate() {
            if last_category != Some(row.category) {
                content = content.child(section_label(row.category_label));
                last_category = Some(row.category);
            }
            content = content.child(self.list_row(row, index, cx));
        }
        content.into_any_element()
    }
}

impl Render for LauncherView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.coordinator.snapshot();
        let phase_message: SharedString = match snapshot.phase {
            Phase::Closed => "Closed".into(),
            Phase::Loading => "Searching…".into(),
            Phase::Results {
                still_searching: true,
                degraded: false,
            } => "Searching…".into(),
            Phase::Results { degraded: true, .. } => "Some results are unavailable".into(),
            Phase::Results { .. } => format!("{} results", snapshot.rows.len()).into(),
            Phase::Empty => "No results".into(),
            Phase::Unavailable => "Search providers are unavailable".into(),
            Phase::Activating => "Opening…".into(),
            Phase::ActivationFailed => snapshot
                .announcement
                .clone()
                .unwrap_or_else(|| "Could not open the selection".into())
                .into(),
        };
        let rows = snapshot.rows.clone();
        let query = snapshot.query.clone();
        let has_rows = !rows.is_empty();

        div()
            .size_full()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
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
                    cx.stop_propagation();
                    this.handle_key(command, window, cx);
                }
            }))
            .v_flex()
            .overflow_hidden()
            .rounded(px(18.0))
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .bg(mac::raised())
            .text_color(mac::text())
            .child(
                div()
                    .h(px(76.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(26.0))
                            .text_color(mac::text_secondary())
                            .child("⌕"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(SearchField::new(&self.query).appearance(false)),
                    ),
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
                    .id("launcher-results-scroll")
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
                                        .child("Applications, Settings, files, and calculations"),
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
                    .child("↑↓ Select   Return Open   Ctrl/⌘-Return More   Esc Close"),
            )
    }
}

fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .pt_1()
        .px_2()
        .text_size(rmac_ui::text_px(10.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text_tertiary())
        .child(label.into())
}
