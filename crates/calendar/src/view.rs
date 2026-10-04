use chrono::{Datelike, Duration, NaiveDate, Timelike};
use gpui::{
    div, prelude::FluentBuilder as _, px, AnyElement, ClickEvent, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement as _, Render, Role, ScrollDelta, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_calendar::{
    current_date, events_on_day, fixture_week, is_weekend, month_grid_start, search_events,
    seed_calendars, store, subscription_default_name, unique_calendar_name, validate_calendar_name,
    validate_subscription_url, Calendar, CalendarColor, Navigator, SearchResult, View,
    WeekSnapshot,
};
use rmac_ui::{
    mac, ContextMenu, ContextMenuState, DialogButtonKind, InputEvent, InputState, StyledExt as _,
};

use crate::{
    CloseWindow, DeleteCalendar, DismissSheet, GoToday, NewCalendar, NewCalendarSubscription,
    NextPeriod, PreviousPeriod, RenameCalendar, Search, SelectNextDay, SelectNextWeek,
    SelectPreviousDay, SelectPreviousWeek, ShowDay, ShowMonth, ShowSettings, ShowWeek, ShowYear,
    ToggleSidebar,
};

const SIDEBAR: f32 = 220.0;
const TOOLBAR: f32 = 52.0;
const GUTTER: f32 = 52.0;
const HOUR: f32 = 48.0;
const START_HOUR: f32 = 8.0;

/// Which modal sheet (CAL-6) is currently shown, if any.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Sheet {
    RenameCalendar,
    DeleteCalendar,
    NewCalendarSubscription,
}

pub struct CalendarView {
    pub focus: FocusHandle,
    nav: Navigator,
    sidebar_visible: bool,
    /// The mutable, session-long calendar list (CAL-6): seeded once from
    /// `seed_calendars()` plus any saved local calendars/subscriptions, then
    /// never rebuilt. `Event::calendar` indexes into it; deleting a calendar
    /// only sets `removed`/`visible` so those indices stay valid.
    calendars: Vec<Calendar>,
    scroll_accumulated: f32,
    general: store::GeneralSettings,
    /// Settings ▸ Accounts lists these (ADR 0022 §1: GOA is the only
    /// account store). Always empty for now — ACC-2 (the GOA adapter) and
    /// ACC-3 (the Internet Accounts pane that creates accounts) are not
    /// built yet, so Calendar has nowhere to read a live account from.
    accounts: Vec<rmac_accounts::model::Account>,
    menu_at: Option<ContextMenuState>,
    /// Set when a calendar row was right-clicked; `None` means the "+" menu
    /// (New Calendar / New Calendar Subscription…) is open instead.
    context_calendar: Option<usize>,
    sheet: Option<Sheet>,
    sheet_error: Option<SharedString>,
    rename_input: Entity<InputState>,
    subscription_name_input: Entity<InputState>,
    subscription_url_input: Entity<InputState>,
    subscription_color: CalendarColor,
    search_open: bool,
    search_input: Entity<InputState>,
    search_results: Vec<SearchResult>,
    search_selected: usize,
}

impl CalendarView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store::load_settings().unwrap_or_default();
        let mut calendars = seed_calendars();
        for saved in &settings.saved_calendars {
            calendars.push(saved.to_calendar(true));
        }
        for calendar in &mut calendars {
            if settings.removed.iter().any(|name| name == &calendar.name) {
                calendar.removed = true;
                calendar.visible = false;
            } else if settings.hidden.iter().any(|name| name == &calendar.name) {
                calendar.visible = false;
            }
        }
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let subscription_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Team Releases"));
        let subscription_url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("webcal:// or https://…"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.update_search(cx);
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            nav: Navigator::new(current_date()),
            sidebar_visible: true,
            calendars,
            scroll_accumulated: 0.0,
            general: settings.general,
            accounts: Vec::new(),
            menu_at: None,
            context_calendar: None,
            sheet: None,
            sheet_error: None,
            rename_input,
            subscription_name_input,
            subscription_url_input,
            subscription_color: CalendarColor::Blue,
            search_open: false,
            search_input,
            search_results: Vec::new(),
            search_selected: 0,
        }
    }

    /// The flags `rmac_calendar::events_on_day`/`search_events` expect: a
    /// calendar is shown exactly when it is both ticked and not deleted.
    fn visible_flags(&self) -> Vec<bool> {
        self.calendars
            .iter()
            .map(|calendar| calendar.visible)
            .collect()
    }

    /// The current week's fixture snapshot — shared by rendering and search
    /// so both agree on what "this week" means.
    fn week_snapshot(&self) -> WeekSnapshot {
        let today = current_date();
        fixture_week(today - Duration::days(today.weekday().num_days_from_monday() as i64))
    }

    fn persist(&self, cx: &mut Context<Self>) {
        let seed_count = seed_calendars().len();
        let mut settings = store::Settings {
            general: self.general.clone(),
            ..store::Settings::default()
        };
        for (index, calendar) in self.calendars.iter().enumerate() {
            if index >= seed_count {
                settings
                    .saved_calendars
                    .push(store::SavedCalendar::from_calendar(calendar));
            }
            if calendar.removed {
                settings.removed.push(calendar.name.clone());
            } else if !calendar.visible {
                settings.hidden.push(calendar.name.clone());
            }
        }
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = store::save_settings(&settings) {
                    eprintln!("rmac-calendar: could not save settings: {error}");
                }
            })
            .detach();
    }

    pub(crate) fn calendar_names(&self) -> Vec<SharedString> {
        self.calendars
            .iter()
            .filter(|calendar| !calendar.removed)
            .map(|calendar| SharedString::from(calendar.name.clone()))
            .collect()
    }

    pub(crate) fn general(&self) -> store::GeneralSettings {
        self.general.clone()
    }

    pub(crate) fn accounts(&self) -> &[rmac_accounts::model::Account] {
        &self.accounts
    }

    pub(crate) fn set_general(&mut self, general: store::GeneralSettings, cx: &mut Context<Self>) {
        self.general = general.normalized();
        self.persist(cx);
        cx.notify();
    }

    fn next_color(calendars: &[Calendar]) -> CalendarColor {
        let count = calendars
            .iter()
            .filter(|calendar| !calendar.removed)
            .count();
        CalendarColor::ALL[count % CalendarColor::ALL.len()]
    }

    fn open_add_menu(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_calendar = None;
        self.menu_at = Some(ContextMenuState::open(position, &self.focus, window, cx));
        cx.notify();
    }

    fn open_row_menu(
        &mut self,
        index: usize,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_calendar = Some(index);
        self.menu_at = Some(ContextMenuState::open(position, &self.focus, window, cx));
        cx.notify();
    }

    fn create_new_calendar(&mut self, cx: &mut Context<Self>) {
        let name = unique_calendar_name(&self.calendars, "New Calendar");
        let color = Self::next_color(&self.calendars);
        self.calendars.push(Calendar {
            name,
            account: "On My Mac".to_owned(),
            color,
            visible: true,
            removed: false,
            subscription_url: None,
        });
        self.persist(cx);
        cx.notify();
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.context_calendar else {
            return;
        };
        let name = self.calendars[index].name.clone();
        self.rename_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.sheet_error = None;
        self.sheet = Some(Sheet::RenameCalendar);
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.context_calendar else {
            return;
        };
        let value = self.rename_input.read(cx).value().to_string();
        match validate_calendar_name(&value) {
            Ok(name) => {
                self.calendars[index].name = name;
                self.sheet = None;
                self.sheet_error = None;
                self.persist(cx);
            }
            Err(_) => {
                self.sheet_error = Some("Give this calendar a name.".into());
            }
        }
        cx.notify();
    }

    fn start_delete(&mut self, cx: &mut Context<Self>) {
        if self.context_calendar.is_none() {
            return;
        }
        self.sheet = Some(Sheet::DeleteCalendar);
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.context_calendar.take() {
            if let Some(calendar) = self.calendars.get_mut(index) {
                calendar.removed = true;
                calendar.visible = false;
            }
            self.persist(cx);
        }
        self.sheet = None;
        cx.notify();
    }

    fn start_new_calendar_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription_name_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.subscription_url_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.subscription_color = Self::next_color(&self.calendars);
        self.sheet_error = None;
        self.sheet = Some(Sheet::NewCalendarSubscription);
        cx.notify();
    }

    fn commit_subscription(&mut self, cx: &mut Context<Self>) {
        let url = self.subscription_url_input.read(cx).value().to_string();
        let url = match validate_subscription_url(&url) {
            Ok(url) => url,
            Err(_) => {
                self.sheet_error =
                    Some("Enter a webcal://, https:// or http:// calendar address.".into());
                cx.notify();
                return;
            }
        };
        let typed_name = self.subscription_name_input.read(cx).value().to_string();
        let name = validate_calendar_name(&typed_name).unwrap_or_else(|_| {
            unique_calendar_name(&self.calendars, &subscription_default_name(&url))
        });
        self.calendars.push(Calendar {
            name,
            account: "Subscribed".to_owned(),
            color: self.subscription_color,
            visible: true,
            removed: false,
            subscription_url: Some(url),
        });
        self.sheet = None;
        self.sheet_error = None;
        self.persist(cx);
        cx.notify();
    }

    fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        if self.sheet.take().is_some() {
            self.sheet_error = None;
            cx.notify();
        } else if self.search_open {
            self.search_open = false;
            cx.notify();
        }
    }

    fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = !self.search_open;
        if self.search_open {
            self.search_input
                .update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.search_results.clear();
        }
        cx.notify();
    }

    fn update_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().to_string();
        let snapshot = self.week_snapshot();
        self.search_results = search_events(&snapshot, &self.visible_flags(), &query);
        self.search_selected = 0;
        cx.notify();
    }

    fn jump_to_search_result(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.search_results.get(self.search_selected).cloned() else {
            return;
        };
        self.nav.view = View::Day;
        self.nav.selected = result.start.date_naive();
        self.search_open = false;
        self.sync_menu(cx);
        cx.notify();
    }

    fn show(&mut self, view: View, cx: &mut Context<Self>) {
        self.nav.view = view;
        self.sync_menu(cx);
        cx.notify();
    }

    fn scroll_period(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Lines(point) => point.y * 40.0,
            ScrollDelta::Pixels(point) => f32::from(point.y),
        };
        self.scroll_accumulated += delta;
        if self.scroll_accumulated.abs() >= 80.0 {
            self.nav
                .step(if self.scroll_accumulated < 0.0 { 1 } else { -1 });
            self.scroll_accumulated = 0.0;
            cx.notify();
        }
    }

    fn sync_menu(&self, cx: &mut Context<Self>) {
        for (view, action) in [
            (View::Day, "calendar::ShowDay"),
            (View::Week, "calendar::ShowWeek"),
            (View::Month, "calendar::ShowMonth"),
            (View::Year, "calendar::ShowYear"),
        ] {
            rmac_ui::set_menu_checked(action, self.nav.view == view, cx);
        }
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

    fn control(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        accessible: &'static str,
        active: bool,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label: SharedString = label.into();
        div()
            .id(id.into())
            .role(Role::Button)
            .aria_label(accessible)
            .h(px(28.0))
            .px(px(10.0))
            .rounded(px(mac::radius_pill()))
            .flex()
            .items_center()
            .justify_center()
            .bg(if active {
                mac::control_fill()
            } else {
                mac::material_clear()
            })
            .text_color(mac::text())
            .text_size(px(12.0))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| click(this, cx)))
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .h(px(TOOLBAR))
            .w_full()
            .flex()
            .items_center()
            .pl(px(if self.sidebar_visible {
                SIDEBAR + 8.0
            } else {
                16.0
            }))
            .pr(px(12.0))
            .gap(px(8.0));
        bar = bar.child(self.control(
            "calendar-sidebar",
            "☷",
            "Sidebar",
            false,
            |this, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                this.sync_menu(cx);
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(self.control("calendar-inbox", "▢", "Invitations", false, |_, _| {}, cx));
        bar = bar.child(self.control("calendar-new", "+", "New Event", false, |_, _| {}, cx));
        bar = bar.child(div().flex_1());
        let mut tabs = div()
            .id("calendar-view-tabs")
            .flex()
            .role(Role::TabList)
            .aria_label("Calendar views");
        for view in View::ALL {
            tabs = tabs.child(
                div()
                    .id(format!("calendar-view-{}", view.label()))
                    .role(Role::Tab)
                    .aria_label(view.label())
                    .aria_selected(self.nav.view == view)
                    .h(px(28.0))
                    .px(px(12.0))
                    .rounded(px(mac::radius_pill()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if self.nav.view == view {
                        mac::control_fill()
                    } else {
                        mac::material_clear()
                    })
                    .text_color(mac::text())
                    .text_size(px(12.0))
                    .child(view.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show(view, cx))),
            );
        }
        bar = bar.child(tabs);
        bar = bar.child(div().flex_1());
        bar.child(
            div()
                .id("calendar-search")
                .role(Role::Button)
                .aria_label("Search")
                .aria_selected(self.search_open)
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(mac::radius_pill()))
                .flex()
                .items_center()
                .justify_center()
                .bg(if self.search_open {
                    mac::control_fill()
                } else {
                    mac::material_clear()
                })
                .text_color(mac::text())
                .text_size(px(12.0))
                .child("⌕")
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_search(window, cx)),
                ),
        )
        .into_any_element()
    }

    fn mini_month(&self, cx: &mut Context<Self>) -> AnyElement {
        let first = month_grid_start(self.nav.selected);
        let selected_month = self.nav.selected.month();
        let today = current_date();
        let mut grid = div().flex().flex_wrap().w_full();
        for title in ["M", "T", "W", "T", "F", "S", "S"] {
            grid = grid.child(
                div()
                    .w(px(26.0))
                    .h(px(20.0))
                    .text_center()
                    .text_size(px(10.0))
                    .text_color(mac::text_secondary())
                    .child(title),
            );
        }
        for offset in 0..42 {
            let day = first + Duration::days(offset);
            let text = day.day().to_string();
            let is_today = day == today;
            grid = grid.child(
                div()
                    .id(format!("mini-{}", offset))
                    .w(px(26.0))
                    .h(px(22.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.0))
                    .bg(if is_today {
                        mac::system_red()
                    } else {
                        mac::material_clear()
                    })
                    .text_color(if is_today {
                        mac::white()
                    } else if day.month() == selected_month {
                        mac::text()
                    } else {
                        mac::text_secondary()
                    })
                    .child(text)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.nav.selected = day;
                        cx.notify();
                    })),
            );
        }
        div()
            .absolute()
            .left(px(14.0))
            .bottom(px(16.0))
            .w(px(184.0))
            .flex()
            .flex_col()
            .gap(px(7.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::text())
                    .child(self.nav.selected.format("%B %Y").to_string())
                    .child("‹  ›"),
            )
            .child(grid)
            .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div()
            .absolute()
            .left(px(8.0))
            .top(px(8.0))
            .bottom(px(8.0))
            .w(px(SIDEBAR - 8.0))
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator());
        panel = panel.child(rmac_ui::traffic_lights());
        let mut list = div()
            .absolute()
            .top(px(48.0))
            .left(px(10.0))
            .right(px(10.0))
            .flex()
            .flex_col();
        let mut account = String::new();
        for (index, calendar) in self.calendars.iter().enumerate() {
            if calendar.removed {
                continue;
            }
            if calendar.account != account {
                account = calendar.account.clone();
                list = list.child(
                    div()
                        .h(px(26.0))
                        .pt(px(8.0))
                        .pl(px(6.0))
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(mac::text_secondary())
                        .child(account.clone()),
                );
            }
            let color = Self::color(calendar.color);
            let visible = calendar.visible;
            list = list.child(
                div()
                    .id(format!("calendar-toggle-{index}"))
                    .role(Role::Row)
                    .aria_label(calendar.name.clone())
                    .aria_selected(visible)
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .pl(px(8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded(px(mac::radius_control()))
                            .bg(if visible {
                                color
                            } else {
                                mac::material_clear()
                            })
                            .border_1()
                            .border_color(color)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(mac::white())
                            .child(if visible { "✓" } else { "" }),
                    )
                    .child(calendar.name.clone())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.calendars[index].visible = !this.calendars[index].visible;
                        this.persist(cx);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_row_menu(index, event.position, window, cx);
                        }),
                    ),
            );
        }
        list = list.child(
            div()
                .id("calendar-add")
                .role(Role::Button)
                .aria_label("Add Calendar")
                .h(px(24.0))
                .mt(px(4.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(8.0))
                .text_size(px(12.0))
                .text_color(mac::text_secondary())
                .child("+")
                .child("Add Calendar")
                .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                    this.open_add_menu(event.position(), window, cx);
                })),
        );
        panel
            .child(list)
            .child(self.mini_month(cx))
            .into_any_element()
    }

    /// The row's right-click (Rename…/Delete…) or the "+" row's click (New
    /// Calendar/New Calendar Subscription…), depending on `context_calendar`.
    fn build_calendar_menu(&self, pos: gpui::Point<gpui::Pixels>) -> ContextMenu {
        if self.context_calendar.is_some() {
            ContextMenu::new(pos)
                .item("Rename…", Box::new(RenameCalendar))
                .danger_item("Delete…", Box::new(DeleteCalendar))
        } else {
            ContextMenu::new(pos)
                .item("New Calendar", Box::new(NewCalendar))
                .item(
                    "New Calendar Subscription…",
                    Box::new(NewCalendarSubscription),
                )
        }
    }

    fn heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, subtitle) = match self.nav.view {
            View::Day => (
                self.nav.selected.format("%-d %B %Y").to_string(),
                self.nav.selected.format("%A").to_string(),
            ),
            View::Week => (self.nav.selected.format("%B %Y").to_string(), String::new()),
            View::Month => (
                self.nav.selected.format("%B").to_string(),
                self.nav.selected.format("%Y").to_string(),
            ),
            View::Year => (self.nav.selected.format("%Y").to_string(), String::new()),
        };
        div()
            .h(px(44.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(mac::text())
                    .child(title),
            )
            .child(
                div()
                    .ml(px(7.0))
                    .text_size(px(22.0))
                    .text_color(mac::text_secondary())
                    .child(subtitle),
            )
            .child(div().flex_1())
            .child(self.control(
                "calendar-prev",
                "‹",
                "Previous Period",
                false,
                |this, cx| {
                    this.nav.step(-1);
                    cx.notify();
                },
                cx,
            ))
            .child(self.control(
                "calendar-today",
                "Today",
                "Today",
                false,
                |this, cx| {
                    this.nav.today(current_date());
                    cx.notify();
                },
                cx,
            ))
            .child(self.control(
                "calendar-next",
                "›",
                "Next Period",
                false,
                |this, cx| {
                    this.nav.step(1);
                    cx.notify();
                },
                cx,
            ))
            .into_any_element()
    }

    fn time_grid(
        &self,
        snapshot: &WeekSnapshot,
        width: f32,
        height: f32,
        first: NaiveDate,
        days: usize,
    ) -> AnyElement {
        let day_width = ((width - GUTTER) / days as f32).max(1.0);
        let grid_top = 60.0;
        let grid_height = (height - grid_top).max(0.0);
        let mut week = div()
            .id(if days == 1 {
                "calendar-day-grid"
            } else {
                "calendar-week"
            })
            .relative()
            .w_full()
            .h_full()
            .overflow_hidden()
            .role(Role::Group)
            .aria_label(if days == 1 {
                format!("Day of {first}")
            } else {
                format!("Week of {first}")
            });
        for index in 0..days {
            let day = first + Duration::days(index as i64);
            let x = GUTTER + index as f32 * day_width;
            let today = day == current_date();
            let label = day.format("%a").to_string();
            week = week.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top_0()
                    .w(px(day_width))
                    .h(px(34.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(5.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(label)
                    .child(
                        div()
                            .w(px(25.0))
                            .h(px(25.0))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.0))
                            .bg(if today {
                                mac::system_red()
                            } else {
                                mac::material_clear()
                            })
                            .text_color(if today { mac::white() } else { mac::text() })
                            .child(day.day().to_string()),
                    ),
            );
            if is_weekend(day) {
                week = week.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(grid_top))
                        .w(px(day_width))
                        .h(px(grid_height))
                        .bg(mac::row_alternate()),
                );
            }
            week = week.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(grid_top))
                    .w(px(1.0))
                    .h(px(grid_height))
                    .bg(mac::separator()),
            );
        }
        week = week.child(
            div()
                .absolute()
                .top(px(34.0))
                .left_0()
                .right_0()
                .h(px(26.0))
                .border_b_1()
                .border_color(mac::separator())
                .text_size(px(10.0))
                .text_color(mac::text_secondary())
                .child("all-day"),
        );
        for event in snapshot.events.iter().filter(|event| event.all_day) {
            let day = event.start.date_naive();
            let offset = (day - first).num_days();
            if (0..days as i64).contains(&offset) && self.calendars[event.calendar].visible {
                let days = (event.end.date_naive() - day)
                    .num_days()
                    .clamp(1, days as i64 - offset);
                week = week.child(
                    div()
                        .absolute()
                        .left(px(GUTTER + offset as f32 * day_width + 2.0))
                        .top(px(37.0))
                        .w(px(days as f32 * day_width - 4.0))
                        .h(px(19.0))
                        .rounded(px(mac::radius_control()))
                        .px(px(5.0))
                        .overflow_hidden()
                        .bg(Self::color(self.calendars[event.calendar].color))
                        .text_color(mac::white())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(event.title),
                );
            }
        }
        for hour in 8..=20 {
            let y = grid_top + (hour as f32 - START_HOUR) * HOUR;
            week = week.child(
                div()
                    .absolute()
                    .left(px(GUTTER))
                    .right_0()
                    .top(px(y))
                    .h(px(1.0))
                    .bg(mac::separator()),
            );
            if hour > 8 {
                week = week.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(y - 7.0))
                        .w(px(44.0))
                        .text_right()
                        .text_size(px(10.0))
                        .text_color(mac::text_secondary())
                        .child(format!("{hour}:00")),
                );
            }
        }
        for slot in &snapshot.slots {
            let Some(event) = snapshot.events.iter().find(|event| event.id == slot.id) else {
                continue;
            };
            if !self.calendars[event.calendar].visible {
                continue;
            }
            let day = (slot.day - first).num_days();
            if !(0..days as i64).contains(&day) {
                continue;
            }
            let columns = slot.columns.max(1) as f32;
            let width = (day_width - 3.0) / columns;
            let x = GUTTER + day as f32 * day_width + slot.column as f32 * width + 1.0;
            let y = grid_top + (slot.start_second as f32 / 3600.0 - START_HOUR) * HOUR;
            let h = ((slot.end_second - slot.start_second) as f32 / 3600.0 * HOUR - 2.0).max(18.0);
            let color = Self::color(self.calendars[event.calendar].color);
            week = week.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(width - 1.0))
                    .h(px(h))
                    .rounded(px(mac::radius_control()))
                    .overflow_hidden()
                    .pl(px(6.0))
                    .pt(px(2.0))
                    .bg(color.opacity(if mac::window().l < 0.5 { 0.22 } else { 0.18 }))
                    .border_l_3()
                    .border_color(color)
                    .flex()
                    .flex_col()
                    .text_size(px(11.0))
                    .text_color(mac::text())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(event.title))
                    .child(div().text_color(mac::text_secondary()).child(format!(
                        "{}{}",
                        event.start.format("%-H:%M"),
                        if event.location.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", event.location)
                        }
                    ))),
            );
        }
        let today = current_date();
        let day = (today - first).num_days();
        if (0..days as i64).contains(&day) {
            let now = chrono::Local::now();
            let hour = now.hour() as f32 + now.minute() as f32 / 60.0;
            if (START_HOUR..=20.0).contains(&hour) {
                let y = grid_top + (hour - START_HOUR) * HOUR;
                week = week.child(
                    div()
                        .absolute()
                        .left(px(GUTTER + day as f32 * day_width))
                        .top(px(y))
                        .w(px(day_width))
                        .h(px(1.0))
                        .bg(mac::system_red()),
                );
                week = week.child(
                    div()
                        .absolute()
                        .left(px(GUTTER + day as f32 * day_width - 4.0))
                        .top(px(y - 4.0))
                        .size(px(8.0))
                        .rounded_full()
                        .bg(mac::system_red()),
                );
                week = week.child(
                    div()
                        .absolute()
                        .left(px(4.0))
                        .top(px(y - 8.0))
                        .w(px(42.0))
                        .h(px(16.0))
                        .rounded_full()
                        .bg(mac::system_red())
                        .text_color(mac::white())
                        .text_size(px(10.0))
                        .text_center()
                        .child(now.format("%-H:%M").to_string()),
                );
            }
        }
        week.into_any_element()
    }

    fn day(&self, snapshot: &WeekSnapshot, width: f32, height: f32) -> AnyElement {
        let grid_width = (width - 300.0).max(200.0);
        let mut detail = div()
            .id("calendar-day-detail")
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(300.0))
            .border_l_1()
            .border_color(mac::separator())
            .p(px(18.0))
            .flex()
            .flex_col()
            .gap(px(9.0))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(17.0))
                    .child(self.nav.selected.format("%A, %-d %B").to_string()),
            );
        let events = events_on_day(snapshot, &self.visible_flags(), self.nav.selected);
        if events.is_empty() {
            detail = detail.child(
                div()
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child("No events"),
            );
        }
        for event in events {
            let color = Self::color(self.calendars[event.calendar].color);
            detail = detail.child(
                div()
                    .border_l_3()
                    .border_color(color)
                    .pl(px(9.0))
                    .py(px(5.0))
                    .rounded(px(mac::radius_control()))
                    .bg(color.opacity(0.18))
                    .flex()
                    .flex_col()
                    .text_size(px(12.0))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(event.title))
                    .child(
                        div()
                            .text_color(mac::text_secondary())
                            .child(if event.all_day {
                                "All day".to_string()
                            } else {
                                format!(
                                    "{}–{} · {}",
                                    event.start.format("%-H:%M"),
                                    event.end.format("%-H:%M"),
                                    self.calendars[event.calendar].name
                                )
                            }),
                    ),
            );
        }
        div()
            .id("calendar-day")
            .relative()
            .size_full()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(grid_width))
                    .child(self.time_grid(snapshot, grid_width, height, self.nav.selected, 1)),
            )
            .child(detail)
            .into_any_element()
    }

    fn month(
        &self,
        snapshot: &WeekSnapshot,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let first = month_grid_start(self.nav.selected);
        let last = self
            .nav
            .selected
            .with_day(1)
            .expect("valid month")
            .checked_add_months(chrono::Months::new(1))
            .expect("valid next month")
            - Duration::days(1);
        let rows = if (last - first).num_days() < 35 { 5 } else { 6 };
        let cell_width = width / 7.0;
        let cell_height = ((height - 24.0) / rows as f32).max(1.0);
        let visible = self.visible_flags();
        let mut month = div()
            .id("calendar-month")
            .relative()
            .size_full()
            .role(Role::Table)
            .aria_label(self.nav.selected.format("%B %Y").to_string());
        for (index, name) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            .into_iter()
            .enumerate()
        {
            month = month.child(
                div()
                    .absolute()
                    .left(px(index as f32 * cell_width))
                    .top_0()
                    .w(px(cell_width))
                    .h(px(24.0))
                    .pr(px(8.0))
                    .text_right()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(name),
            );
        }
        for index in 0..rows * 7 {
            let day = first + Duration::days(index as i64);
            let events = events_on_day(snapshot, &visible, day);
            let selected = day == self.nav.selected;
            let today = day == current_date();
            let mut cell = div()
                .id(format!("calendar-month-day-{day}"))
                .absolute()
                .left(px(index as f32 % 7.0 * cell_width))
                .top(px(24.0 + (index / 7) as f32 * cell_height))
                .w(px(cell_width))
                .h(px(cell_height))
                .border_t_1()
                .border_l_1()
                .border_color(mac::separator())
                .role(Role::Cell)
                .aria_selected(selected)
                .aria_label(format!(
                    "{}, {}, {} events",
                    day,
                    day.format("%A %-d %B"),
                    events.len()
                ))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.nav.selected = day;
                    cx.notify();
                }))
                .child(
                    div()
                        .absolute()
                        .right(px(6.0))
                        .top(px(4.0))
                        .w(px(24.0))
                        .h(px(22.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .font_weight(if today {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .bg(if today {
                            mac::system_red()
                        } else if selected {
                            mac::control_fill()
                        } else {
                            mac::material_clear()
                        })
                        .text_color(if today {
                            mac::white()
                        } else if day.month() == self.nav.selected.month() {
                            mac::text()
                        } else {
                            mac::text_secondary()
                        })
                        .child(day.day().to_string()),
                );
            for (line, event) in events.iter().take(3).enumerate() {
                let color = Self::color(self.calendars[event.calendar].color);
                let mut item = div()
                    .absolute()
                    .left(px(6.0))
                    .right(px(6.0))
                    .top(px(28.0 + line as f32 * 17.0))
                    .h(px(16.0))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .text_size(px(11.0));
                if event.all_day {
                    item = item
                        .rounded(px(mac::radius_control()))
                        .bg(color)
                        .px(px(5.0))
                        .text_color(mac::white())
                        .font_weight(FontWeight::SEMIBOLD);
                } else {
                    item = item.child(div().size(px(6.0)).rounded_full().bg(color));
                }
                cell = cell.child(item.child(event.title));
            }
            if events.len() > 3 {
                cell = cell.child(
                    div()
                        .absolute()
                        .left(px(8.0))
                        .top(px(79.0))
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(format!("{} more", events.len() - 3)),
                );
            }
            month = month.child(cell);
        }
        month.into_any_element()
    }

    fn year(
        &self,
        snapshot: &WeekSnapshot,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let gap_x = 24.0;
        let gap_y = 12.0;
        let month_width = ((width - 32.0 - gap_x * 3.0) / 4.0).max(1.0);
        let month_height = ((height - 24.0 - gap_y * 2.0) / 3.0).max(1.0);
        let day_width = month_width / 7.0;
        let year = self.nav.selected.year();
        let visible = self.visible_flags();
        let mut panel = div()
            .id("calendar-year")
            .relative()
            .size_full()
            .role(Role::Group)
            .aria_label(year.to_string());
        for month_index in 0..12 {
            let first = NaiveDate::from_ymd_opt(year, month_index + 1, 1).expect("valid month");
            let weekday_offset = first.weekday().num_days_from_monday() as usize;
            let mut card = div()
                .absolute()
                .left(px(16.0 + (month_index % 4) as f32 * (month_width + gap_x)))
                .top(px(8.0 + (month_index / 4) as f32 * (month_height + gap_y)))
                .w(px(month_width))
                .h(px(month_height))
                .relative()
                .child(
                    div()
                        .id(format!("calendar-year-month-{}", month_index + 1))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if month_index + 1 == self.nav.selected.month() {
                            mac::system_red()
                        } else {
                            mac::text()
                        })
                        .child(first.format("%B").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.nav.selected = first;
                            this.show(View::Month, cx);
                        })),
                );
            for (column, label) in ["M", "T", "W", "T", "F", "S", "S"].into_iter().enumerate() {
                card = card.child(
                    div()
                        .absolute()
                        .left(px(column as f32 * day_width))
                        .top(px(25.0))
                        .w(px(day_width))
                        .text_center()
                        .text_size(px(10.0))
                        .text_color(mac::text_secondary())
                        .child(label),
                );
            }
            let next = first
                .checked_add_months(chrono::Months::new(1))
                .expect("valid next month");
            let days = (next - first).num_days() as usize;
            for day_index in 0..days {
                let date = first + Duration::days(day_index as i64);
                let count = events_on_day(snapshot, &visible, date).len();
                let position = weekday_offset + day_index;
                let today = date == current_date();
                card = card.child(
                    div()
                        .absolute()
                        .left(px(
                            (position % 7) as f32 * day_width + (day_width - 19.0) / 2.0
                        ))
                        .top(px(42.0 + (position / 7) as f32 * 20.0))
                        .size(px(19.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.0))
                        .bg(if today {
                            mac::system_red()
                        } else if count > 0 {
                            mac::accent().opacity(match count {
                                1 => 0.15,
                                2 => 0.30,
                                _ => 0.45,
                            })
                        } else {
                            mac::material_clear()
                        })
                        .text_color(if today { mac::white() } else { mac::text() })
                        .child(date.day().to_string()),
                );
            }
            panel = panel.child(card);
        }
        panel.into_any_element()
    }

    /// The toolbar search field and its results popover (CAL-6).
    fn render_search(&self, cx: &mut Context<Self>) -> AnyElement {
        let field = div()
            .absolute()
            .right(px(9.0))
            .top(px(8.0))
            .w(px(260.0))
            .child(rmac_ui::SearchField::new(&self.search_input).small());
        let mut popover = div()
            .absolute()
            .right(px(9.0))
            .top(px(44.0))
            .w(px(260.0))
            .max_h(px(320.0))
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .p(px(4.0))
            .flex()
            .flex_col()
            .role(Role::ListBox)
            .aria_label("Search results");
        if self.search_results.is_empty() {
            popover = popover.child(
                div()
                    .p(px(8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(if self.search_input.read(cx).value().is_empty() {
                        "Type to search this week's events."
                    } else {
                        "No matching events."
                    }),
            );
        }
        let snapshot = self.week_snapshot();
        for (index, result) in self.search_results.iter().enumerate() {
            let selected = index == self.search_selected;
            let color = Self::color(
                self.calendars
                    .get(result.calendar)
                    .map_or(CalendarColor::Blue, |calendar| calendar.color),
            );
            let title = snapshot
                .events
                .iter()
                .find(|event| event.id == result.event_id)
                .map(|event| event.title)
                .unwrap_or_default();
            popover = popover.child(
                div()
                    .id(format!("calendar-search-result-{index}"))
                    .role(Role::ListBoxOption)
                    .aria_label(title)
                    .aria_selected(selected)
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .bg(if selected {
                        mac::control_fill()
                    } else {
                        mac::material_clear()
                    })
                    .text_size(px(12.0))
                    .child(div().size(px(7.0)).rounded_full().bg(color))
                    .child(div().flex_1().text_color(mac::text()).child(title))
                    .child(
                        div()
                            .text_color(mac::text_secondary())
                            .text_size(px(11.0))
                            .child(result.start.format("%a %-d %b").to_string()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.search_selected = index;
                        this.jump_to_search_result(cx);
                    })),
            );
        }
        div()
            .id("calendar-search-overlay")
            .absolute()
            .inset_0()
            .child(field)
            .child(popover)
            .into_any_element()
    }

    fn sheet_card(width: f32, title: impl Into<SharedString>) -> gpui::Div {
        div()
            .w(px(width))
            .v_flex()
            .gap_3()
            .p_5()
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::raised())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(mac::text())
                    .child(title.into()),
            )
    }

    fn render_sheet(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.sheet?;
        let error = self.sheet_error.clone();
        let body: AnyElement = match sheet {
            Sheet::RenameCalendar => {
                let mut card = Self::sheet_card(360.0, "Rename Calendar");
                card = card.child(rmac_ui::TextField::new(&self.rename_input).w_full());
                if let Some(error) = error {
                    card = card.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(mac::danger())
                            .child(error),
                    );
                }
                card.child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            rmac_ui::dialog_button(
                                "calendar-rename-cancel",
                                "Cancel",
                                DialogButtonKind::Normal,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_sheet(cx))),
                        )
                        .child(
                            rmac_ui::dialog_button(
                                "calendar-rename-save",
                                "Rename",
                                DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.commit_rename(cx))),
                        ),
                )
                .into_any_element()
            }
            Sheet::DeleteCalendar => {
                let name = self
                    .context_calendar
                    .and_then(|index| self.calendars.get(index))
                    .map(|calendar| calendar.name.clone())
                    .unwrap_or_default();
                return Some(
                    rmac_ui::alert_cancel_default(
                        format!("Delete “{name}”?"),
                        "Its events will also be deleted. This can't be undone.",
                        vec![
                            rmac_ui::dialog_button(
                                "calendar-delete-cancel",
                                "Cancel",
                                DialogButtonKind::Normal,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_sheet(cx)))
                            .into_any_element(),
                            rmac_ui::dialog_button(
                                "calendar-delete-confirm",
                                "Delete",
                                DialogButtonKind::Destructive,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx)))
                            .into_any_element(),
                        ],
                    )
                    .into_any_element(),
                );
            }
            Sheet::NewCalendarSubscription => {
                let mut card = Self::sheet_card(380.0, "New Calendar Subscription");
                card = card
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(mac::text_secondary())
                                    .child("Name"),
                            )
                            .child(rmac_ui::TextField::new(&self.subscription_name_input).w_full()),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(mac::text_secondary())
                                    .child("URL"),
                            )
                            .child(rmac_ui::TextField::new(&self.subscription_url_input).w_full()),
                    );
                let mut swatches = div().flex().gap(px(7.0));
                for color in CalendarColor::ALL {
                    let selected = self.subscription_color == color;
                    swatches = swatches.child(
                        div()
                            .id(format!("calendar-subscription-color-{color:?}"))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(Self::color(color))
                            .when(selected, |el| el.border_2().border_color(mac::text()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.subscription_color = color;
                                cx.notify();
                            })),
                    );
                }
                card = card.child(swatches);
                if let Some(error) = error {
                    card = card.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(mac::danger())
                            .child(error),
                    );
                }
                card.child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            rmac_ui::dialog_button(
                                "calendar-subscription-cancel",
                                "Cancel",
                                DialogButtonKind::Normal,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_sheet(cx))),
                        )
                        .child(
                            rmac_ui::dialog_button(
                                "calendar-subscription-save",
                                "Subscribe",
                                DialogButtonKind::Primary,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.commit_subscription(cx))),
                        ),
                )
                .into_any_element()
            }
        };
        Some(
            rmac_ui::dialog("calendar-sheet", body)
                .restore_focus_to(self.focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.dismiss_sheet(cx);
                        }
                        "enter" => {
                            cx.stop_propagation();
                            match this.sheet {
                                Some(Sheet::RenameCalendar) => this.commit_rename(cx),
                                Some(Sheet::NewCalendarSubscription) => {
                                    this.commit_subscription(cx)
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_menu(cx);
        let size = rmac_ui::window_content_size(window);
        let side = if self.sidebar_visible {
            SIDEBAR + 8.0
        } else {
            0.0
        };
        let content_width = (f32::from(size.width) - side).max(1.0);
        let content_height = (f32::from(size.height) - TOOLBAR - 44.0).max(1.0);
        let snapshot = self.week_snapshot();
        let body = div()
            .absolute()
            .left(px(side))
            .right_0()
            .top(px(TOOLBAR))
            .bottom_0()
            .flex()
            .flex_col()
            .child(self.heading(cx))
            .child(div().flex_1().relative().child(match self.nav.view {
                View::Day => self.day(&snapshot, content_width, content_height),
                View::Week => self.time_grid(
                    &snapshot,
                    content_width,
                    content_height,
                    self.nav.week_start(),
                    7,
                ),
                View::Month => self.month(&snapshot, content_width, content_height, cx),
                View::Year => self.year(&snapshot, content_width, content_height, cx),
            }));
        let mut root = div()
            .size_full()
            .relative()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Calendar")
            .on_action(cx.listener(|this, _: &ShowDay, _, cx| this.show(View::Day, cx)))
            .on_action(cx.listener(|this, _: &ShowWeek, _, cx| this.show(View::Week, cx)))
            .on_action(cx.listener(|this, _: &ShowMonth, _, cx| this.show(View::Month, cx)))
            .on_action(cx.listener(|this, _: &ShowYear, _, cx| this.show(View::Year, cx)))
            .on_action(cx.listener(|this, _: &GoToday, _, cx| {
                this.nav.today(current_date());
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PreviousPeriod, _, cx| {
                this.nav.step(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextPeriod, _, cx| {
                this.nav.step(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPreviousDay, _, cx| {
                if this.nav.view == View::Month {
                    this.nav.move_day(-1);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNextDay, _, cx| {
                if this.nav.view == View::Month {
                    this.nav.move_day(1);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPreviousWeek, _, cx| {
                if this.nav.view == View::Month {
                    this.nav.move_day(-7);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNextWeek, _, cx| {
                if this.nav.view == View::Month {
                    this.nav.move_day(7);
                    cx.notify();
                }
            }))
            .on_scroll_wheel(
                cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.scroll_period(event, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                this.sync_menu(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Search, window, cx| this.toggle_search(window, cx)))
            .on_action(
                cx.listener(|this: &mut Self, _: &NewCalendar, _, cx| this.create_new_calendar(cx)),
            )
            .on_action(
                cx.listener(|this: &mut Self, _: &RenameCalendar, window, cx| {
                    this.start_rename(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this: &mut Self, _: &DeleteCalendar, _, cx| this.start_delete(cx)),
            )
            .on_action(
                cx.listener(|this: &mut Self, _: &NewCalendarSubscription, window, cx| {
                    this.start_new_calendar_subscription(window, cx)
                }),
            )
            .on_action(cx.listener(|_, _: &ShowSettings, _, cx| {
                let main = cx.entity();
                cx.defer(move |cx| crate::settings_window::show(main, cx));
            }))
            .on_action(
                cx.listener(|this: &mut Self, _: &DismissSheet, _, cx| this.dismiss_sheet(cx)),
            )
            .on_action(
                cx.listener(|this: &mut Self, _: &rmac_ui::DismissMenu, window, cx| {
                    rmac_ui::ContextMenuState::dismiss(&mut this.menu_at, window, cx);
                }),
            )
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            );
        root = root.child(self.toolbar(cx)).child(body);
        if self.sidebar_visible {
            root = root.child(self.sidebar(cx));
        }
        if self.search_open {
            root = root.child(self.render_search(cx));
        }
        if let Some(menu_at) = self.menu_at.clone() {
            root = root.child(
                self.build_calendar_menu(menu_at.position())
                    .render(&menu_at),
            );
        }
        if let Some(sheet) = self.render_sheet(cx) {
            root = root.child(sheet);
        }
        root
    }
}
