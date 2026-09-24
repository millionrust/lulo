//! Reusable Notes rows, empty states, and display-only formatting.

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::Hsla;
use gpui::{
    div, font, prelude::FluentBuilder as _, px, AnyElement, Div, InteractiveElement as _,
    IntoElement, ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled, StyledText, TextRun, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};
use rmac_ui::{mac, StyledExt as _};

use super::glyphs::glyph;
use super::notes_style::{
    folder_glyph, sidebar_count, sidebar_selected_count, sidebar_selected_text, sidebar_selection,
    sidebar_text, SIDEBAR_COUNT_RIGHT, SIDEBAR_GLYPH, SIDEBAR_GLYPH_CENTRE, SIDEBAR_ROW_HEIGHT,
    SIDEBAR_ROW_RADIUS, SIDEBAR_TEXT_X,
};

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

/// A folder in the sidebar: 32 pt row, the selection inset 11 from the
/// panel with radius 8, a yellow outline glyph centred 14 in, the name at
/// 29.5 (yellow while selected) and the note count 8 from the right.
pub(super) fn folder_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    glyph_path: &'static str,
    count: usize,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let label = label.into();
    // No AccessKit `description` setter is exposed on `div()` (see
    // `docs/accessibility-audit.md`'s Toast fix for the same constraint), so
    // the item count is folded into the accessible name, matching the visible
    // row's own text.
    let accessible_label = format!(
        "{label}, {count} {}",
        if count == 1 { "note" } else { "notes" }
    );
    div()
        .id(id)
        .role(Role::ListItem)
        .aria_label(accessible_label)
        .aria_selected(selected)
        .h(px(SIDEBAR_ROW_HEIGHT))
        .flex_none()
        .relative()
        .rounded(px(SIDEBAR_ROW_RADIUS))
        .when(selected, |element: Stateful<Div>| {
            element.bg(sidebar_selection())
        })
        .when(!selected, |element: Stateful<Div>| {
            element.hover(|hover| hover.bg(mac::hover()))
        })
        .child(
            div()
                .absolute()
                .left(px(SIDEBAR_GLYPH_CENTRE - SIDEBAR_GLYPH / 2.0))
                .top(px((SIDEBAR_ROW_HEIGHT - SIDEBAR_GLYPH) / 2.0))
                .child(glyph(glyph_path, SIDEBAR_GLYPH, folder_glyph())),
        )
        .child(
            div()
                .absolute()
                .left(px(SIDEBAR_TEXT_X))
                .right(px(SIDEBAR_COUNT_RIGHT + 28.0))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(if selected {
                    sidebar_selected_text()
                } else {
                    sidebar_text()
                })
                .child(div().truncate().child(label.clone())),
        )
        .child(
            div()
                .absolute()
                .right(px(SIDEBAR_COUNT_RIGHT))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(if selected {
                    sidebar_selected_count()
                } else {
                    sidebar_count()
                })
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

fn local_date(unix_ms: u64) -> DateTime<Local> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(unix_ms))
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .into()
}

/// The note list's date section, as Notes groups a list sorted by date:
/// Today, Yesterday, Previous 7 Days, Previous 30 Days, then the month for
/// this year and the year before that.
pub(super) fn date_section(unix_ms: u64) -> SharedString {
    section_for(local_date(unix_ms), Local::now())
}

fn section_for(date: DateTime<Local>, now: DateTime<Local>) -> SharedString {
    let days = now
        .date_naive()
        .signed_duration_since(date.date_naive())
        .num_days();
    match days {
        i64::MIN..=0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=7 => "Previous 7 Days".into(),
        8..=30 => "Previous 30 Days".into(),
        _ if date.year() == now.year() => date.format("%B").to_string().into(),
        _ => date.year().to_string().into(),
    }
}

/// The editor's centred timestamp: "23 September 2026 at 5:14 PM".
pub(super) fn full_date_label(unix_ms: u64) -> SharedString {
    local_date(unix_ms)
        .format("%-d %B %Y at %-I:%M %p")
        .to_string()
        .into()
}

pub(super) fn styled_search_fragment(
    fragment: SearchTextFragment,
    secondary: bool,
    bold: bool,
) -> StyledText {
    let base_color = if secondary {
        mac::text_secondary()
    } else {
        mac::text()
    };
    styled_search_fragment_in(fragment, base_color, bold)
}

/// A search fragment drawn in `base_color`, its match highlighted.
pub(super) fn styled_search_fragment_in(
    fragment: SearchTextFragment,
    base_color: Hsla,
    bold: bool,
) -> StyledText {
    let mut text_font = font(rmac_ui::UI_FONT);
    if bold {
        text_font = text_font.bold();
    }
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

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone as _};

    use super::section_for;

    #[test]
    fn note_list_sections_follow_notes_date_groups() {
        let now = Local.with_ymd_and_hms(2026, 9, 23, 17, 0, 0).unwrap();
        let at = |y, m, d| Local.with_ymd_and_hms(y, m, d, 9, 0, 0).unwrap();
        assert_eq!(section_for(at(2026, 9, 23), now), "Today");
        assert_eq!(section_for(at(2026, 9, 22), now), "Yesterday");
        assert_eq!(section_for(at(2026, 9, 17), now), "Previous 7 Days");
        assert_eq!(section_for(at(2026, 9, 1), now), "Previous 30 Days");
        assert_eq!(section_for(at(2026, 3, 4), now), "March");
        assert_eq!(section_for(at(2024, 3, 4), now), "2024");
    }
}
