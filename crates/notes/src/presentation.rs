//! Reusable Notes rows, empty states, and display-only formatting.

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    div, font, prelude::FluentBuilder as _, px, AnyElement, Div, InteractiveElement as _,
    IntoElement, ParentElement, SharedString, Stateful, StatefulInteractiveElement as _,
    StrikethroughStyle, Styled, StyledText, TextRun, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use rmac_ui::{mac, StyledExt as _};

use super::search_highlight::SearchTextFragment;

pub(super) fn centered_state(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(mac::window())
        .child(
            div()
                .w(px(440.0))
                .v_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(18.0))
                        .font_weight(mac::SEMIBOLD)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .text_center()
                        .child(detail.into()),
                ),
        )
        .into_any_element()
}

pub(super) fn folder_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    icon: IconName,
    count: usize,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(px(rmac_ui::mac::radius_menu_item()))
        .when(selected, |element: Stateful<Div>| {
            element.bg(mac::sidebar_selection())
        })
        .when(!selected, |element: Stateful<Div>| {
            element.hover(|hover| hover.bg(mac::hover()))
        })
        .child(
            Icon::new(icon)
                .text_color(mac::notes_accent())
                .with_size(Size::Small),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_size(rmac_ui::text_px(13.0))
                .child(label.into()),
        )
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(mac::text_tertiary())
                .child(count.to_string()),
        )
        .on_click(on_click)
}

pub(super) fn tag_pill(fragment: SearchTextFragment) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(px(rmac_ui::mac::radius_menu_item()))
        .bg(mac::control_fill())
        .text_size(rmac_ui::text_px(10.0))
        .text_color(mac::text_secondary())
        .child(styled_search_fragment(fragment, true, false))
}

pub(super) fn attachment_match_row(fragment: SearchTextFragment) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(rmac_ui::text_px(10.0))
        .text_color(mac::text_secondary())
        .child(
            Icon::new(IconName::File)
                .with_size(Size::XSmall)
                .text_color(mac::text_tertiary()),
        )
        .child(styled_search_fragment(fragment, true, false))
}

pub(super) fn format_storage_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f64 = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes_f64 < MIB {
        format!("{:.1} KiB", bytes_f64 / KIB)
    } else if bytes_f64 < GIB {
        format!("{:.1} MiB", bytes_f64 / MIB)
    } else {
        format!("{:.1} GiB", bytes_f64 / GIB)
    }
}

pub(super) fn date_label(unix_ms: u64) -> SharedString {
    let time = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(unix_ms))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let date: DateTime<Local> = time.into();
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(date.date_naive())
        .num_days();
    if days == 0 {
        let hour = date.hour();
        let (hour, suffix) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{hour}:{:02} {suffix}", date.minute()).into()
    } else if days == 1 {
        "Yesterday".into()
    } else if (2..7).contains(&days) {
        date.format("%A").to_string().into()
    } else {
        format!("{}/{}/{:02}", date.month(), date.day(), date.year() % 100).into()
    }
}

pub(super) fn styled_search_fragment(
    fragment: SearchTextFragment,
    secondary: bool,
    bold: bool,
) -> StyledText {
    let mut text_font = font(rmac_ui::UI_FONT);
    if bold {
        text_font = text_font.bold();
    }
    let base_color = if secondary {
        mac::text_secondary()
    } else {
        mac::text()
    };
    let text_len = fragment.text().len();
    let mut runs = Vec::with_capacity(3);
    let mut push_run = |len: usize, highlighted: bool| {
        if len == 0 {
            return;
        }
        runs.push(TextRun {
            len,
            font: text_font.clone(),
            color: if highlighted { mac::text() } else { base_color },
            background_color: highlighted.then(mac::accent_subtle),
            underline: None,
            strikethrough: None,
        });
    };
    if let Some(highlight) = fragment
        .highlight()
        .filter(|range| range.start < range.end && range.end <= text_len)
    {
        push_run(highlight.start, false);
        push_run(highlight.end - highlight.start, true);
        push_run(text_len - highlight.end, false);
    } else {
        push_run(text_len, false);
    }
    StyledText::new(fragment.text().to_string()).with_runs(runs)
}
