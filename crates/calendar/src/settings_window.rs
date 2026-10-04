//! Calendar ▸ Settings… (⌘,, CAL-6): General (default calendar, start of
//! week, day starts/ends, time zone support) and Accounts (from
//! `rmac-accounts`; empty until ACC-2/ACC-3 land a live account store).

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Window, WindowHandle,
};
use rmac_ui::{mac, Button, InputEvent, InputState, Root, StyledExt as _, TextField};

use rmac_accounts::model::{Service, Services};
use rmac_calendar::store::GeneralSettings;

use crate::view::CalendarView;
use crate::{CloseWindow, ShowSettings};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 360.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn show(main: Entity<CalendarView>, cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::CALENDAR, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Settings");
        let view = cx.new(|cx| SettingsView::new(main, window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-calendar: could not open Settings: {error}"),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Tab {
    General,
    Accounts,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Accounts => "Accounts",
        }
    }
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    main: Entity<CalendarView>,
    time_zone_input: Entity<InputState>,
}

impl SettingsView {
    fn new(main: Entity<CalendarView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        rmac_ui::observe_window_state(rmac_ui::app_id::CALENDAR, window, cx);
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        let time_zone = main.read(cx).general().time_zone;
        let time_zone_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Europe/London")
                .default_value(time_zone)
        });
        cx.subscribe(&time_zone_input, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.update_time_zone(cx);
            }
        })
        .detach();
        Self {
            focus,
            tab: Tab::General,
            main,
            time_zone_input,
        }
    }

    fn general(&self, cx: &App) -> GeneralSettings {
        self.main.read(cx).general()
    }

    fn set_general(&mut self, general: GeneralSettings, cx: &mut Context<Self>) {
        self.main
            .update(cx, |view, cx| view.set_general(general, cx));
        cx.notify();
    }

    fn update_time_zone(&mut self, cx: &mut Context<Self>) {
        let value = self.time_zone_input.read(cx).value().to_string();
        let mut general = self.general(cx);
        general.time_zone_support = !value.trim().is_empty();
        general.time_zone = value;
        self.set_general(general, cx);
    }

    fn hour_label(hour: u32) -> String {
        let hour = hour % 24;
        let (display, suffix) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{display}:00 {suffix}")
    }

    /// A "‹ 8:00 AM ›" control that nudges `hour` by one through `on_change`
    /// (clamped there, since Starts/Ends clamp differently).
    fn hour_stepper(
        cx: &mut Context<Self>,
        id: &'static str,
        hour: u32,
        general: &GeneralSettings,
        on_change: fn(&mut GeneralSettings, u32),
    ) -> impl IntoElement {
        let dec = general.clone();
        let inc = general.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new(format!("{id}-dec"), "‹")
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut next = dec.clone();
                        on_change(&mut next, hour.saturating_sub(1));
                        this.set_general(next, cx);
                    })),
            )
            .child(
                div()
                    .w(px(84.0))
                    .text_center()
                    .text_color(mac::text())
                    .child(Self::hour_label(hour)),
            )
            .child(
                Button::new(format!("{id}-inc"), "›")
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut next = inc.clone();
                        on_change(&mut next, hour + 1);
                        this.set_general(next, cx);
                    })),
            )
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let general = self.general(cx);
        let names = self.main.read(cx).calendar_names();
        let default_calendar = if names
            .iter()
            .any(|name| name.as_ref() == general.default_calendar)
        {
            general.default_calendar.clone()
        } else {
            names
                .first()
                .map(|name| name.to_string())
                .unwrap_or_default()
        };

        let mut default_pills = div().flex().flex_wrap().gap_2();
        for name in &names {
            let selected = name.as_ref() == default_calendar;
            let general = general.clone();
            let picked = name.to_string();
            default_pills = default_pills.child(
                Button::new(format!("calendar-settings-default-{name}"), name.clone())
                    .small()
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut general = general.clone();
                        general.default_calendar = picked.clone();
                        this.set_general(general, cx);
                    })),
            );
        }

        let week_general = general.clone();
        let week_row =
            div()
                .flex()
                .gap_2()
                .children([("Monday", false), ("Sunday", true)].into_iter().map(
                    |(label, sunday)| {
                        let selected = general.start_of_week_sunday == sunday;
                        let mut next = week_general.clone();
                        Button::new(format!("calendar-settings-week-{label}"), label)
                            .small()
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                next.start_of_week_sunday = sunday;
                                this.set_general(next.clone(), cx);
                            }))
                    },
                ));

        let time_zone_support = general.time_zone_support;
        let tz_general = general.clone();

        div()
            .v_flex()
            .gap_4()
            .child(labelled_row("Default Calendar", default_pills))
            .child(labelled_row("Start Week On", week_row))
            .child(labelled_row(
                "Day Starts At",
                Self::hour_stepper(
                    cx,
                    "calendar-settings-day-start",
                    general.day_starts_hour,
                    &general,
                    |general, hour| general.day_starts_hour = hour.min(23),
                ),
            ))
            .child(labelled_row(
                "Day Ends At",
                Self::hour_stepper(
                    cx,
                    "calendar-settings-day-end",
                    general.day_ends_hour,
                    &general,
                    |general, hour| general.day_ends_hour = hour.clamp(1, 24),
                ),
            ))
            .child(
                div().flex().items_center().gap_2().child(
                    Button::new("calendar-settings-tz-toggle", "Turn on time zone support")
                        .small()
                        .selected(time_zone_support)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut next = tz_general.clone();
                            next.time_zone_support = !next.time_zone_support;
                            if !next.time_zone_support {
                                next.time_zone.clear();
                            }
                            this.set_general(next, cx);
                        })),
                ),
            )
            .when(time_zone_support, |el| {
                el.child(labelled_row(
                    "Time Zone",
                    TextField::new(&self.time_zone_input).small().w(px(200.0)),
                ))
            })
    }

    /// "Mail, Calendars" — the services summary next to an account, as the
    /// Internet Accounts list shows it.
    fn services_summary(services: Services) -> String {
        [
            (Service::Mail, "Mail"),
            (Service::Calendar, "Calendars"),
            (Service::Contacts, "Contacts"),
        ]
        .into_iter()
        .filter(|(service, _)| services.enabled(*service))
        .map(|(_, label)| label)
        .collect::<Vec<_>>()
        .join(", ")
    }

    fn render_accounts(&self, cx: &App) -> impl IntoElement {
        let accounts = self.main.read(cx).accounts();
        if accounts.is_empty() {
            // ACC-2/ACC-3 (the GOA adapter and the Internet Accounts pane)
            // are not built yet, so there is never a live account to list
            // here — see docs/design/calendar-mail.md §2 and ADR 0022. This
            // empty state matches design-lab/calendar.html's Accounts frame.
            return div()
                .v_flex()
                .items_center()
                .gap_1()
                .pt(px(28.0))
                .text_color(mac::text_secondary())
                .child("No Internet Accounts yet.")
                .child("Add one in System Settings ▸ Internet Accounts.")
                .into_any_element();
        }
        let mut list = div().v_flex().gap_2();
        for account in accounts {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(26.0))
                            .rounded_full()
                            .bg(mac::control_fill())
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(11.0))
                            .child(
                                account
                                    .display_name
                                    .chars()
                                    .next()
                                    .map(String::from)
                                    .unwrap_or_default(),
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(account.display_name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(mac::text_secondary())
                                    .child(Self::services_summary(account.services)),
                            ),
                    ),
            );
        }
        list.into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::General => self.render_general(cx).into_any_element(),
            Tab::Accounts => self.render_accounts(cx).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .key_context("Calendar")
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, _, cx| cx.quit()))
            .on_action(cx.listener(|_, _: &ShowSettings, window, _| window.activate_window()))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(mac::separator())
                    .children([Tab::General, Tab::Accounts].into_iter().map(|tab| {
                        Button::new(format!("calendar-settings-{}", tab.label()), tab.label())
                            .small()
                            .selected(self.tab == tab)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = tab;
                                cx.notify();
                            }))
                    })),
            )
            .child(div().flex_1().px_4().py_4().child(body))
    }
}

fn labelled_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(130.0))
                .text_color(mac::text_secondary())
                .text_size(px(12.0))
                .child(label),
        )
        .child(control)
}
