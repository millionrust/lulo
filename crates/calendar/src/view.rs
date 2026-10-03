use chrono::{Datelike, Duration, Timelike};
use gpui::{div, px, AnyElement, ClickEvent, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window};
use rmac_calendar::{current_date, fixture_week, is_weekend, month_grid_start,
    CalendarColor, Navigator, View, WeekSnapshot};
use rmac_ui::mac;

use crate::{CloseWindow, GoToday, NextPeriod, PreviousPeriod, ShowDay, ShowMonth,
    ShowWeek, ShowYear, ToggleSidebar};

const SIDEBAR: f32 = 220.0;
const TOOLBAR: f32 = 52.0;
const GUTTER: f32 = 52.0;
const HOUR: f32 = 48.0;
const START_HOUR: f32 = 8.0;

pub struct CalendarView {
    pub focus: FocusHandle,
    nav: Navigator,
    sidebar_visible: bool,
    visible: [bool; 6],
}

impl CalendarView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { focus: cx.focus_handle(), nav: Navigator::new(current_date()),
            sidebar_visible: true, visible: [true, true, true, true, false, true] }
    }

    fn show(&mut self, view: View, cx: &mut Context<Self>) {
        self.nav.view = view;
        self.sync_menu(cx);
        cx.notify();
    }

    fn sync_menu(&self, cx: &mut Context<Self>) {
        for (view, action) in [
            (View::Day, "calendar::ShowDay"), (View::Week, "calendar::ShowWeek"),
            (View::Month, "calendar::ShowMonth"), (View::Year, "calendar::ShowYear"),
        ] { rmac_ui::set_menu_checked(action, self.nav.view == view, cx); }
        rmac_ui::set_menu_checked("calendar::ToggleSidebar", self.sidebar_visible, cx);
    }

    fn color(color: CalendarColor) -> gpui::Hsla {
        match color {
            CalendarColor::Blue => mac::system_blue(),
            CalendarColor::Teal => mac::system_teal(),
            CalendarColor::Orange => mac::system_orange(),
            CalendarColor::Green => mac::system_green(),
            CalendarColor::Purple => mac::system_purple(),
            CalendarColor::Red => mac::system_red(),
        }
    }

    fn control(&self, id: impl Into<SharedString>, label: impl Into<SharedString>,
        accessible: &'static str,
        active: bool, click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>) -> AnyElement {
        let label: SharedString = label.into();
        div().id(id.into()).role(Role::Button).aria_label(accessible)
            .h(px(28.0)).px(px(10.0)).rounded(px(mac::radius_pill()))
            .flex().items_center().justify_center()
            .bg(if active { mac::control_fill() } else { mac::material_clear() })
            .text_color(mac::text()).text_size(px(12.0)).child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| click(this, cx)))
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div().h(px(TOOLBAR)).w_full().flex().items_center()
            .pl(px(if self.sidebar_visible { SIDEBAR + 8.0 } else { 16.0 }))
            .pr(px(12.0)).gap(px(8.0));
        bar = bar.child(self.control("calendar-sidebar", "☷", "Sidebar", false, |this, cx| {
            this.sidebar_visible = !this.sidebar_visible; this.sync_menu(cx); cx.notify();
        }, cx));
        bar = bar.child(self.control("calendar-inbox", "▢", "Invitations", false, |_, _| {}, cx));
        bar = bar.child(self.control("calendar-new", "+", "New Event", false, |_, _| {}, cx));
        bar = bar.child(div().flex_1());
        let mut tabs = div().id("calendar-view-tabs").flex().role(Role::TabList)
            .aria_label("Calendar views");
        for view in View::ALL {
            tabs = tabs.child(div().id(format!("calendar-view-{}", view.label()))
                .role(Role::Tab).aria_label(view.label())
                .aria_selected(self.nav.view == view)
                .h(px(28.0)).px(px(12.0)).rounded(px(mac::radius_pill()))
                .flex().items_center().justify_center()
                .bg(if self.nav.view == view { mac::control_fill() } else { mac::material_clear() })
                .text_color(mac::text()).text_size(px(12.0)).child(view.label())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show(view, cx))));
        }
        bar = bar.child(tabs);
        bar = bar.child(div().flex_1());
        bar.child(self.control("calendar-search", "⌕", "Search", false, |_, _| {}, cx)).into_any_element()
    }

    fn mini_month(&self, cx: &mut Context<Self>) -> AnyElement {
        let first = month_grid_start(self.nav.selected);
        let selected_month = self.nav.selected.month();
        let today = current_date();
        let mut grid = div().flex().flex_wrap().w_full();
        for title in ["M", "T", "W", "T", "F", "S", "S"] {
            grid = grid.child(div().w(px(26.0)).h(px(20.0)).text_center()
                .text_size(px(10.0)).text_color(mac::text_secondary()).child(title));
        }
        for offset in 0..42 {
            let day = first + Duration::days(offset);
            let text = day.day().to_string();
            let is_today = day == today;
            grid = grid.child(div().id(format!("mini-{}", offset))
                .w(px(26.0)).h(px(22.0)).rounded(px(11.0))
                .flex().items_center().justify_center().text_size(px(11.0))
                .bg(if is_today { mac::system_red() } else { mac::material_clear() })
                .text_color(if is_today { mac::white() } else if day.month() == selected_month {
                    mac::text() } else { mac::text_secondary() })
                .child(text)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.nav.selected = day; cx.notify();
                })));
        }
        div().absolute().left(px(14.0)).bottom(px(16.0)).w(px(184.0))
            .flex().flex_col().gap(px(7.0))
            .child(div().flex().justify_between().text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD).text_color(mac::text())
                .child(self.nav.selected.format("%B %Y").to_string())
                .child("‹  ›"))
            .child(grid).into_any_element()
    }

    fn sidebar(&self, snapshot: &WeekSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().absolute().left(px(8.0)).top(px(8.0)).bottom(px(8.0))
            .w(px(SIDEBAR - 8.0)).rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar()).border_1().border_color(mac::separator());
        panel = panel.child(rmac_ui::traffic_lights());
        let mut list = div().absolute().top(px(48.0)).left(px(10.0)).right(px(10.0))
            .flex().flex_col();
        let mut account = "";
        for (index, calendar) in snapshot.calendars.iter().enumerate() {
            if calendar.account != account {
                account = calendar.account;
                list = list.child(div().h(px(26.0)).pt(px(8.0)).pl(px(6.0))
                    .text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::text_secondary()).child(account));
            }
            let color = Self::color(calendar.color);
            let visible = self.visible[index];
            list = list.child(div().id(format!("calendar-toggle-{index}"))
                .h(px(24.0)).flex().items_center().gap(px(8.0)).pl(px(8.0))
                .text_size(px(12.0)).text_color(mac::text())
                .child(div().w(px(14.0)).h(px(14.0)).rounded(px(4.0))
                    .bg(if visible { color } else { mac::material_clear() })
                    .border_1().border_color(color)
                    .flex().items_center().justify_center()
                    .text_size(px(10.0)).text_color(mac::white())
                    .child(if visible { "✓" } else { "" }))
                .child(calendar.name)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.visible[index] = !this.visible[index]; cx.notify();
                })));
        }
        panel.child(list).child(self.mini_month(cx)).into_any_element()
    }

    fn heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = match self.nav.view {
            View::Year => self.nav.selected.format("%Y").to_string(),
            _ => self.nav.selected.format("%B %Y").to_string(),
        };
        div().h(px(44.0)).px(px(16.0)).flex().items_center()
            .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD)
                .text_color(mac::text()).child(title))
            .child(div().flex_1())
            .child(self.control("calendar-prev", "‹", "Previous Period", false, |this, cx| {
                this.nav.step(-1); cx.notify();
            }, cx))
            .child(self.control("calendar-today", "Today", "Today", false, |this, cx| {
                this.nav.today(current_date()); cx.notify();
            }, cx))
            .child(self.control("calendar-next", "›", "Next Period", false, |this, cx| {
                this.nav.step(1); cx.notify();
            }, cx)).into_any_element()
    }

    fn week(&self, snapshot: &WeekSnapshot, width: f32, height: f32) -> AnyElement {
        let first = self.nav.week_start();
        let day_width = ((width - GUTTER) / 7.0).max(1.0);
        let grid_top = 60.0;
        let grid_height = (height - grid_top).max(0.0);
        let mut week = div().id("calendar-week").relative().w_full().h_full().overflow_hidden()
            .role(Role::Group).aria_label(format!("Week of {first}"));
        for index in 0..7 {
            let day = first + Duration::days(index);
            let x = GUTTER + index as f32 * day_width;
            let today = day == current_date();
            let label = day.format("%a").to_string();
            week = week.child(div().absolute().left(px(x)).top_0().w(px(day_width))
                .h(px(34.0)).flex().items_center().justify_center().gap(px(5.0))
                .text_size(px(12.0)).text_color(mac::text_secondary())
                .child(label)
                .child(div().w(px(25.0)).h(px(25.0)).rounded(px(13.0))
                    .flex().items_center().justify_center()
                    .font_weight(FontWeight::SEMIBOLD).text_size(px(15.0))
                    .bg(if today { mac::system_red() } else { mac::material_clear() })
                    .text_color(if today { mac::white() } else { mac::text() })
                    .child(day.day().to_string())));
            if is_weekend(day) {
                week = week.child(div().absolute().left(px(x)).top(px(grid_top))
                    .w(px(day_width)).h(px(grid_height)).bg(mac::row_alternate()));
            }
            week = week.child(div().absolute().left(px(x)).top(px(grid_top))
                .w(px(1.0)).h(px(grid_height)).bg(mac::separator()));
        }
        week = week.child(div().absolute().top(px(34.0)).left_0().right_0()
            .h(px(26.0)).border_b_1().border_color(mac::separator())
            .text_size(px(10.0)).text_color(mac::text_secondary())
            .child("all-day"));
        for event in snapshot.events.iter().filter(|event| event.all_day) {
            let day = event.start.date_naive();
            let offset = (day - first).num_days();
            if (0..7).contains(&offset) && self.visible[event.calendar] {
                let days = (event.end.date_naive() - day).num_days().clamp(1, 7 - offset);
                week = week.child(div().absolute()
                    .left(px(GUTTER + offset as f32 * day_width + 2.0))
                    .top(px(37.0)).w(px(days as f32 * day_width - 4.0)).h(px(19.0))
                    .rounded(px(4.0)).px(px(5.0)).overflow_hidden()
                    .bg(Self::color(snapshot.calendars[event.calendar].color))
                    .text_color(mac::white()).text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD).child(event.title));
            }
        }
        for hour in 8..=20 {
            let y = grid_top + (hour as f32 - START_HOUR) * HOUR;
            week = week.child(div().absolute().left(px(GUTTER)).right_0()
                .top(px(y)).h(px(1.0)).bg(mac::separator()));
            if hour > 8 {
                week = week.child(div().absolute().left_0().top(px(y - 7.0))
                    .w(px(44.0)).text_right().text_size(px(10.0))
                    .text_color(mac::text_secondary()).child(format!("{hour}:00")));
            }
        }
        for slot in &snapshot.slots {
            let Some(event) = snapshot.events.iter().find(|event| event.id == slot.id) else { continue; };
            if !self.visible[event.calendar] { continue; }
            let day = (slot.day - first).num_days();
            if !(0..7).contains(&day) { continue; }
            let columns = slot.columns.max(1) as f32;
            let width = (day_width - 3.0) / columns;
            let x = GUTTER + day as f32 * day_width + slot.column as f32 * width + 1.0;
            let y = grid_top + (slot.start_second as f32 / 3600.0 - START_HOUR) * HOUR;
            let h = ((slot.end_second - slot.start_second) as f32 / 3600.0 * HOUR - 2.0).max(18.0);
            let color = Self::color(snapshot.calendars[event.calendar].color);
            week = week.child(div().absolute().left(px(x)).top(px(y)).w(px(width - 1.0))
                .h(px(h)).rounded(px(4.0)).overflow_hidden().pl(px(6.0)).pt(px(2.0))
                .bg(color.opacity(if mac::window().l < 0.5 { 0.22 } else { 0.18 }))
                .border_l_3().border_color(color)
                .flex().flex_col().text_size(px(11.0)).text_color(mac::text())
                .child(div().font_weight(FontWeight::SEMIBOLD).child(event.title))
                .child(div().text_color(mac::text_secondary())
                    .child(format!("{}{}", event.start.format("%-H:%M"),
                        if event.location.is_empty() { String::new() }
                        else { format!(" · {}", event.location) }))));
        }
        let today = current_date();
        let day = (today - first).num_days();
        if (0..7).contains(&day) {
            let now = chrono::Local::now();
            let hour = now.hour() as f32 + now.minute() as f32 / 60.0;
            if (START_HOUR..=20.0).contains(&hour) {
                let y = grid_top + (hour - START_HOUR) * HOUR;
                week = week.child(div().absolute().left(px(GUTTER + day as f32 * day_width))
                    .top(px(y)).w(px(day_width)).h(px(1.0)).bg(mac::system_red()));
                week = week.child(div().absolute().left(px(4.0)).top(px(y - 8.0))
                    .w(px(42.0)).h(px(16.0)).rounded(px(8.0)).bg(mac::system_red())
                    .text_color(mac::white()).text_size(px(10.0)).text_center()
                    .child(now.format("%-H:%M").to_string()));
            }
        }
        week.into_any_element()
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_menu(cx);
        let size = rmac_ui::window_content_size(window);
        let side = if self.sidebar_visible { SIDEBAR + 8.0 } else { 0.0 };
        let content_width = (f32::from(size.width) - side).max(1.0);
        let content_height = (f32::from(size.height) - TOOLBAR - 44.0).max(1.0);
        let snapshot = fixture_week(self.nav.week_start());
        let body = div().absolute().left(px(side)).right_0().top(px(TOOLBAR)).bottom_0()
            .flex().flex_col()
            .child(self.heading(cx))
            .child(div().flex_1().relative().child(match self.nav.view {
                View::Week => self.week(&snapshot, content_width, content_height),
                _ => div().h_full().flex().items_center().justify_center()
                    .text_color(mac::text_secondary())
                    .child(format!("{} view arrives in CAL-4", self.nav.view.label()))
                    .into_any_element(),
            }));
        let mut root = div().size_full().relative().bg(mac::window())
            .track_focus(&self.focus).key_context("Calendar")
            .on_action(cx.listener(|this, _: &ShowDay, _, cx| this.show(View::Day, cx)))
            .on_action(cx.listener(|this, _: &ShowWeek, _, cx| this.show(View::Week, cx)))
            .on_action(cx.listener(|this, _: &ShowMonth, _, cx| this.show(View::Month, cx)))
            .on_action(cx.listener(|this, _: &ShowYear, _, cx| this.show(View::Year, cx)))
            .on_action(cx.listener(|this, _: &GoToday, _, cx| { this.nav.today(current_date()); cx.notify(); }))
            .on_action(cx.listener(|this, _: &PreviousPeriod, _, cx| { this.nav.step(-1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &NextPeriod, _, cx| { this.nav.step(1); cx.notify(); }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_visible = !this.sidebar_visible; this.sync_menu(cx); cx.notify();
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()));
        root = root.child(self.toolbar(cx)).child(body);
        if self.sidebar_visible { root = root.child(self.sidebar(&snapshot, cx)); }
        root
    }
}
