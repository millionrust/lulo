//! Deterministic component-state specimen rendering.

use gpui::{
    div, prelude::FluentBuilder as _, px, rgba, AnyElement, IntoElement, ParentElement,
    SharedString, Styled,
};
use gpui_component::StyledExt as _;
use rmac_ui::{
    gallery::{GalleryComponent, GalleryState, PreviewScale},
    mac,
};

fn p(scale: PreviewScale, value: f32) -> gpui::Pixels {
    px(scale.px(value))
}

fn is_selected(state: GalleryState) -> bool {
    matches!(
        state,
        GalleryState::Selected | GalleryState::On | GalleryState::Pressed | GalleryState::Complete
    )
}

fn is_disabled(state: GalleryState) -> bool {
    matches!(state, GalleryState::Disabled | GalleryState::Unavailable)
}

fn is_error(state: GalleryState) -> bool {
    matches!(
        state,
        GalleryState::Error | GalleryState::Invalid | GalleryState::Destructive
    )
}

fn state_background(state: GalleryState) -> gpui::Hsla {
    match state {
        GalleryState::Error | GalleryState::Invalid | GalleryState::Destructive => {
            mac::error_background()
        }
        GalleryState::Warning | GalleryState::Stale => mac::warning_background(),
        GalleryState::Success | GalleryState::Complete => mac::accent_subtle(),
        _ if is_selected(state) => mac::accent_subtle(),
        _ => mac::control_fill(),
    }
}

fn state_border(state: GalleryState) -> gpui::Hsla {
    match state {
        GalleryState::Focused => mac::accent(),
        GalleryState::Error | GalleryState::Invalid | GalleryState::Destructive => {
            mac::error_border()
        }
        GalleryState::Warning | GalleryState::Stale => mac::warning_border(),
        _ if is_selected(state) => mac::accent_border(),
        _ => mac::separator(),
    }
}

fn state_text(state: GalleryState) -> gpui::Hsla {
    match state {
        GalleryState::Error | GalleryState::Invalid | GalleryState::Destructive => mac::danger(),
        GalleryState::Warning | GalleryState::Stale => mac::warning_text(),
        _ => mac::text(),
    }
}

fn specimen_frame(
    state: GalleryState,
    scale: PreviewScale,
    content: impl IntoElement,
) -> gpui::Div {
    div()
        .min_w(p(scale, 112.0))
        .min_h(p(scale, 38.0))
        .px(p(scale, 9.0))
        .py(p(scale, 6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(p(scale, 7.0))
        .border_1()
        .border_color(state_border(state))
        .bg(state_background(state))
        .text_color(state_text(state))
        .text_size(p(scale, 11.0))
        .when(is_disabled(state), |element| element.opacity(0.42))
        .child(content)
}

pub(crate) fn render_specimen(
    component: GalleryComponent,
    state: GalleryState,
    scale: PreviewScale,
) -> AnyElement {
    match component {
        GalleryComponent::Button => render_button(state, scale),
        GalleryComponent::Toggle => render_toggle(state, scale),
        GalleryComponent::Slider => render_slider(state, scale),
        GalleryComponent::TextField | GalleryComponent::SearchField => {
            render_field(component, state, scale)
        }
        GalleryComponent::List | GalleryComponent::Table | GalleryComponent::Tree => {
            render_collection(component, state, scale)
        }
        GalleryComponent::Tabs => render_tab(state, scale),
        GalleryComponent::Dialog => render_dialog(state, scale),
        GalleryComponent::Alert | GalleryComponent::Toast => {
            render_message(component, state, scale)
        }
        GalleryComponent::ContextMenu => render_menu_item(state, scale),
        GalleryComponent::Tooltip => render_tooltip(state, scale),
        GalleryComponent::Progress => render_progress(state, scale),
        GalleryComponent::EmptyState => render_empty_state(state, scale),
    }
}

fn render_button(state: GalleryState, scale: PreviewScale) -> AnyElement {
    let filled = matches!(
        state,
        GalleryState::Pressed | GalleryState::Busy | GalleryState::Destructive
    );
    let background = if state == GalleryState::Destructive {
        mac::danger()
    } else if filled {
        mac::accent()
    } else {
        state_background(state)
    };
    let foreground = if filled {
        if state == GalleryState::Destructive {
            mac::on_danger()
        } else {
            mac::on_accent()
        }
    } else {
        state_text(state)
    };
    div()
        .min_w(p(scale, 104.0))
        .h(p(scale, 28.0))
        .px(p(scale, 12.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(p(scale, 7.0))
        .border_1()
        .border_color(state_border(state))
        .bg(background)
        .text_color(foreground)
        .text_size(p(scale, 11.0))
        .font_weight(mac::MEDIUM)
        .when(is_disabled(state), |element| element.opacity(0.42))
        .child(if state == GalleryState::Busy {
            "Working…"
        } else {
            "Continue"
        })
        .into_any_element()
}

fn render_toggle(state: GalleryState, scale: PreviewScale) -> AnyElement {
    let on = matches!(state, GalleryState::On | GalleryState::Mixed);
    specimen_frame(
        state,
        scale,
        div()
            .flex()
            .items_center()
            .gap(p(scale, 7.0))
            .child(
                div()
                    .w(p(scale, 32.0))
                    .h(p(scale, 18.0))
                    .px(p(scale, 2.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .rounded_full()
                    .bg(if on { mac::accent() } else { mac::separator() })
                    .when(!on, |element| element.justify_start())
                    .child(
                        div()
                            .size(p(scale, 14.0))
                            .rounded_full()
                            .bg(mac::raised())
                            .shadow_sm(),
                    ),
            )
            .child(if state == GalleryState::Mixed {
                "Mixed"
            } else if on {
                "On"
            } else {
                "Off"
            }),
    )
    .into_any_element()
}

fn render_slider(state: GalleryState, scale: PreviewScale) -> AnyElement {
    specimen_frame(
        state,
        scale,
        div()
            .w(p(scale, 90.0))
            .h(p(scale, 18.0))
            .flex()
            .items_center()
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(p(scale, 4.0))
                    .rounded_full()
                    .bg(mac::separator())
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w_3_5()
                            .rounded_full()
                            .bg(mac::accent()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(p(scale, 50.0))
                            .top(p(scale, -5.0))
                            .size(p(scale, 14.0))
                            .rounded_full()
                            .border_1()
                            .border_color(state_border(state))
                            .bg(mac::raised())
                            .shadow_sm(),
                    ),
            ),
    )
    .into_any_element()
}

fn render_field(
    component: GalleryComponent,
    state: GalleryState,
    scale: PreviewScale,
) -> AnyElement {
    let text: SharedString = match (component, state) {
        (_, GalleryState::Empty) => "Placeholder".into(),
        (GalleryComponent::SearchField, GalleryState::Loading) => "Searching…".into(),
        (GalleryComponent::SearchField, _) => "⌕  settings".into(),
        (_, GalleryState::Invalid) => "Invalid value".into(),
        _ => "Editable text".into(),
    };
    specimen_frame(
        state,
        scale,
        div()
            .w(p(scale, 126.0))
            .truncate()
            .text_color(if state == GalleryState::Empty {
                mac::text_tertiary()
            } else {
                state_text(state)
            })
            .child(text),
    )
    .into_any_element()
}

fn render_collection(
    component: GalleryComponent,
    state: GalleryState,
    scale: PreviewScale,
) -> AnyElement {
    let prefix = match component {
        GalleryComponent::Table => {
            if state == GalleryState::SortedAscending {
                "Name  ↑"
            } else if state == GalleryState::SortedDescending {
                "Name  ↓"
            } else {
                "Row"
            }
        }
        GalleryComponent::Tree => {
            if state == GalleryState::Expanded {
                "⌄  Folder"
            } else {
                "›  Folder"
            }
        }
        _ => "List item",
    };
    let label = match state {
        GalleryState::Empty => "No items",
        GalleryState::Loading => "Loading…",
        GalleryState::Stale => "Cached item",
        GalleryState::Unavailable => "Unavailable",
        _ => prefix,
    };
    specimen_frame(state, scale, div().w(p(scale, 116.0)).child(label)).into_any_element()
}

fn render_tab(state: GalleryState, scale: PreviewScale) -> AnyElement {
    let selected = state == GalleryState::Selected;
    div()
        .min_w(p(scale, 96.0))
        .h(p(scale, 30.0))
        .px(p(scale, 10.0))
        .flex()
        .items_center()
        .justify_center()
        .border_b_2()
        .border_color(if selected || state == GalleryState::Focused {
            state_border(state)
        } else {
            rgba(0x00000000).into()
        })
        .text_color(if selected {
            mac::text()
        } else {
            mac::text_secondary()
        })
        .text_size(p(scale, 11.0))
        .when(is_disabled(state), |element| element.opacity(0.42))
        .child("General")
        .into_any_element()
}

fn render_dialog(state: GalleryState, scale: PreviewScale) -> AnyElement {
    specimen_frame(
        state,
        scale,
        div()
            .v_flex()
            .gap(p(scale, 4.0))
            .w(p(scale, 136.0))
            .child(
                div()
                    .font_weight(mac::SEMIBOLD)
                    .child(if state == GalleryState::Destructive {
                        "Delete item?"
                    } else {
                        "Save changes?"
                    }),
            )
            .child(div().text_color(mac::text_secondary()).child(
                if state == GalleryState::Error {
                    "Could not complete"
                } else {
                    "Cancel     Continue"
                },
            )),
    )
    .into_any_element()
}

fn render_message(
    component: GalleryComponent,
    state: GalleryState,
    scale: PreviewScale,
) -> AnyElement {
    let symbol = match state {
        GalleryState::Success => "✓",
        GalleryState::Warning => "!",
        GalleryState::Error => "×",
        _ => "i",
    };
    specimen_frame(
        state,
        scale,
        div()
            .w(p(scale, 142.0))
            .flex()
            .items_center()
            .gap(p(scale, 7.0))
            .child(
                div()
                    .size(p(scale, 17.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(state_border(state))
                    .text_color(mac::on_accent())
                    .child(symbol),
            )
            .child(if component == GalleryComponent::Toast {
                "Temporary notice"
            } else {
                "Important message"
            }),
    )
    .into_any_element()
}

fn render_menu_item(state: GalleryState, scale: PreviewScale) -> AnyElement {
    specimen_frame(
        state,
        scale,
        div()
            .w(p(scale, 132.0))
            .flex()
            .justify_between()
            .child(if is_error(state) { "Delete" } else { "Open" })
            .child("⌘O"),
    )
    .into_any_element()
}

fn render_tooltip(state: GalleryState, scale: PreviewScale) -> AnyElement {
    div()
        .min_w(p(scale, 112.0))
        .px(p(scale, 9.0))
        .py(p(scale, 5.0))
        .rounded(p(scale, 6.0))
        .border_1()
        .border_color(state_border(state))
        .bg(mac::text())
        .text_color(mac::window())
        .text_size(p(scale, 10.0))
        .child("Helpful description")
        .into_any_element()
}

fn render_progress(state: GalleryState, scale: PreviewScale) -> AnyElement {
    let width = match state {
        GalleryState::Complete => 100.0,
        GalleryState::Indeterminate => 34.0,
        GalleryState::Error => 72.0,
        _ => 62.0,
    };
    specimen_frame(
        state,
        scale,
        div()
            .w(p(scale, 112.0))
            .h(p(scale, 6.0))
            .rounded_full()
            .bg(mac::separator())
            .child(div().h_full().w(p(scale, width)).rounded_full().bg(
                if state == GalleryState::Error {
                    mac::danger()
                } else {
                    mac::accent()
                },
            )),
    )
    .into_any_element()
}

fn render_empty_state(state: GalleryState, scale: PreviewScale) -> AnyElement {
    specimen_frame(
        state,
        scale,
        div()
            .v_flex()
            .items_center()
            .gap(p(scale, 3.0))
            .w(p(scale, 124.0))
            .child(
                div()
                    .text_size(p(scale, 18.0))
                    .text_color(state_text(state))
                    .child("◇"),
            )
            .child(match state {
                GalleryState::Unavailable => "Service unavailable",
                GalleryState::Error => "Could not load",
                _ => "Nothing here yet",
            }),
    )
    .into_any_element()
}
