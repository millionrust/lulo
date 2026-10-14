//! Calendar ▸ Settings… (⌘,): General (days per week, start of week, week
//! scrolling, day starts/ends, hours shown, default event duration, default
//! calendar; CAL-6, MROBOT-07), Accounts (from `rmac-accounts`), Alerts
//! (Default Alerts for events and all-day events, CAL-7, ADR 0022 §6) and
//! Advanced (time zone support, year view events, week numbers).
//!
//! Left out, as nothing in Lulo could honour them: Default calendar app
//! (Lulo has one calendar app), Show Birthdays/Holidays calendar and the
//! alternative calendar (no Contacts birthdays or holiday source), Time to
//! Leave (no location or traffic service), the Notification Centre message
//! toggles for shared calendars and invitations (Calendar posts no such
//! notifications), and Advanced's open-in-separate-windows, ask-before-
//! sending, invitee declines and recent-locations rows.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Window, WindowHandle,
};
use rmac_ui::{
    mac, Button, Checkbox, InputEvent, InputState, PopUpButton, PopupMenuItem, Root,
    StyledExt as _, TextField,
};

use rmac_accounts::model::{Service, Services};
use rmac_calendar::editing::{AlertOffset, ALL_DAY_ALERT_OFFSETS, EVENT_ALERT_OFFSETS};
use rmac_calendar::store::{
    GeneralSettings, WeekScroll, EVENT_DURATION_CHOICES, HOURS_SHOWN_CHOICES,
};

use crate::view::CalendarView;
use crate::{CloseWindow, ShowSettings};

// The Mac's General pane is 629 points wide.
const WIDTH: f32 = 629.0;
const HEIGHT: f32 = 520.0;

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
    // Not `window_options_for_app`: Settings would then inherit whatever
    // size the main Calendar window last saved under the same app_id (the
    // UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::CALENDAR, WIDTH, HEIGHT, cx);
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
    Alerts,
    Advanced,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Accounts => "Accounts",
            Self::Alerts => "Alerts",
            Self::Advanced => "Advanced",
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
        // `register_menu_target` below already tracks this as a key-window
        // candidate; `observe_window_state` would additionally persist its
        // geometry under `app_id::CALENDAR`, the same key the main window
        // uses, letting Settings' own fixed size overwrite the main
        // window's restored one on close (the UIA-06/UIA-09 bug).
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

    /// A pop-up of `choices` that writes the picked one through `apply`.
    fn popup<T: Clone + PartialEq + 'static>(
        &self,
        id: &'static str,
        current: T,
        choices: Vec<(T, String)>,
        apply: fn(&mut GeneralSettings, T),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let label = choices
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, label)| label.clone())
            .unwrap_or_default();
        PopUpButton::new(id, label)
            .w(px(180.0))
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, (value, label)| {
                    let view = view.clone();
                    let checked = *value == current;
                    let value = value.clone();
                    menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(
                        move |_, _, cx| {
                            view.update(cx, |this, cx| {
                                let mut next = this.general(cx);
                                apply(&mut next, value.clone());
                                this.set_general(next, cx);
                            });
                        },
                    ))
                })
            })
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
        let weekdays = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        let hours: Vec<(u32, String)> = (0..=24)
            .map(|hour| (hour, Self::hour_label(hour)))
            .collect();

        div()
            .v_flex()
            .gap_3()
            .child(labelled_row(
                "Days per week:",
                self.popup(
                    "calendar-settings-days-per-week",
                    general.days_per_week,
                    vec![(5, "5".to_owned()), (7, "7".to_owned())],
                    |general, days| general.days_per_week = days,
                    cx,
                ),
            ))
            .child(labelled_row(
                "Start week on:",
                self.popup(
                    "calendar-settings-start-week",
                    general.start_of_week,
                    weekdays
                        .iter()
                        .enumerate()
                        .map(|(index, name)| (index as u32, (*name).to_owned()))
                        .collect(),
                    |general, day| general.start_of_week = day,
                    cx,
                ),
            ))
            .child(labelled_row(
                "Scroll in week view by:",
                self.popup(
                    "calendar-settings-week-scroll",
                    general.week_scroll,
                    vec![
                        (WeekScroll::Week, "Week".to_owned()),
                        (WeekScroll::Day, "Day".to_owned()),
                    ],
                    |general, scroll| general.week_scroll = scroll,
                    cx,
                ),
            ))
            .child(labelled_row(
                "Day starts at:",
                self.popup(
                    "calendar-settings-day-start",
                    general.day_starts_hour,
                    hours[..24].to_vec(),
                    |general, hour| general.day_starts_hour = hour.min(23),
                    cx,
                ),
            ))
            .child(labelled_row(
                "Day ends at:",
                self.popup(
                    "calendar-settings-day-end",
                    general.day_ends_hour,
                    hours[1..].to_vec(),
                    |general, hour| general.day_ends_hour = hour.clamp(1, 24),
                    cx,
                ),
            ))
            .child(labelled_row(
                "Show:",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.popup(
                            "calendar-settings-hours-shown",
                            general.hours_shown,
                            HOURS_SHOWN_CHOICES
                                .iter()
                                .map(|hours| (*hours, hours.to_string()))
                                .collect(),
                            |general, hours| general.hours_shown = hours,
                            cx,
                        ),
                    )
                    .child("hours at a time"),
            ))
            .child(labelled_row(
                "Default event duration:",
                self.popup(
                    "calendar-settings-event-duration",
                    general.default_event_minutes,
                    EVENT_DURATION_CHOICES
                        .iter()
                        .map(|minutes| (*minutes, duration_label(*minutes)))
                        .collect(),
                    |general, minutes| general.default_event_minutes = minutes,
                    cx,
                ),
            ))
            .child(labelled_row(
                "Default Calendar:",
                self.popup(
                    "calendar-settings-default-calendar",
                    default_calendar,
                    names
                        .iter()
                        .map(|name| (name.to_string(), name.to_string()))
                        .collect(),
                    |general, name| general.default_calendar = name,
                    cx,
                ),
            ))
    }

    /// Advanced: time zone support, week numbers and the year view's
    /// event shading, each read by the main window as it draws.
    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let general = self.general(cx);
        let time_zone_support = general.time_zone_support;
        div()
            .v_flex()
            .gap_3()
            .child(self.checkbox(
                "calendar-settings-tz-toggle",
                "Turn on time zone support",
                time_zone_support,
                |general, on| {
                    general.time_zone_support = on;
                    if !on {
                        general.time_zone.clear();
                    }
                },
                cx,
            ))
            .when(time_zone_support, |el| {
                el.child(labelled_row(
                    "Time Zone",
                    TextField::new(&self.time_zone_input).small().w(px(200.0)),
                ))
            })
            .child(self.checkbox(
                "calendar-settings-year-events",
                "Show events in year view",
                general.show_events_in_year_view,
                |general, on| general.show_events_in_year_view = on,
                cx,
            ))
            .child(self.checkbox(
                "calendar-settings-week-numbers",
                "Show week numbers",
                general.show_week_numbers,
                |general, on| general.show_week_numbers = on,
                cx,
            ))
    }

    fn checkbox(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        apply: fn(&mut GeneralSettings, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        Checkbox::new(id)
            .label(label)
            .checked(checked)
            .on_change(move |value: &bool, _, cx| {
                let value = *value;
                view.update(cx, |this, cx| {
                    let mut next = this.general(cx);
                    apply(&mut next, value);
                    this.set_general(next, cx);
                });
            })
    }

    /// Default Alerts (CAL-7, ADR 0022 §6): the alert new events get when
    /// the person does not pick one in the inspector. Birthdays has no row
    /// -- Lulo has no Birthdays calendar (no Contacts app), so a default
    /// for it would do nothing.
    fn render_alerts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let general = self.general(cx);
        let offsets = |options: &'static [AlertOffset]| -> Vec<(AlertOffset, String)> {
            options
                .iter()
                .map(|offset| (*offset, offset.label().to_owned()))
                .collect()
        };
        div()
            .v_flex()
            .gap_3()
            .child(labelled_row(
                "Events:",
                self.popup(
                    "calendar-settings-alert-events",
                    general.default_alerts.events,
                    offsets(EVENT_ALERT_OFFSETS),
                    |general, offset| general.default_alerts.events = offset,
                    cx,
                ),
            ))
            .child(labelled_row(
                "All Day Events:",
                self.popup(
                    "calendar-settings-alert-allday",
                    general.default_alerts.all_day_events,
                    offsets(ALL_DAY_ALERT_OFFSETS),
                    |general, offset| general.default_alerts.all_day_events = offset,
                    cx,
                ),
            ))
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
                .child(if cfg!(windows) {
                    "Accounts aren't available on Windows yet."
                } else {
                    "Add one in System Settings ▸ Internet Accounts."
                })
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
            Tab::Alerts => self.render_alerts(cx).into_any_element(),
            Tab::Accounts => self.render_accounts(cx).into_any_element(),
            Tab::Advanced => self.render_advanced(cx).into_any_element(),
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
                    .children(
                        [Tab::General, Tab::Accounts, Tab::Alerts, Tab::Advanced]
                            .into_iter()
                            .map(|tab| {
                                Button::new(
                                    format!("calendar-settings-{}", tab.label()),
                                    tab.label(),
                                )
                                .small()
                                .selected(self.tab == tab)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.tab = tab;
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
            )
            .child(div().flex_1().px_4().py_4().child(body))
    }
}

fn labelled_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    // Right-aligned labels before their controls, as the Mac's General pane.
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(220.0))
                .text_right()
                .text_color(mac::text())
                .text_size(px(13.0))
                .child(label),
        )
        .child(control)
}

/// "15 minutes", "1 hour", "1.5 hours" for Default event duration.
fn duration_label(minutes: u32) -> String {
    match minutes {
        60 => "1 hour".to_owned(),
        90 => "1.5 hours".to_owned(),
        120 => "2 hours".to_owned(),
        minutes => format!("{minutes} minutes"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn duration_labels_read_like_the_mac() {
        assert_eq!(super::duration_label(15), "15 minutes");
        assert_eq!(super::duration_label(60), "1 hour");
        assert_eq!(super::duration_label(90), "1.5 hours");
    }
}
