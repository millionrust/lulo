use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use gpui::{
    accesskit, div, prelude::FluentBuilder as _, px, AnyElement, AppContext as _, ClickEvent,
    Context, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Render, Role, ScrollDelta,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Toggled, Window,
};
use rmac_calendar::{
    current_date,
    editing::{self, Mutation},
    events_on_day, invitations, is_weekend, month_grid_start, search_events, store,
    subscription_default_name, unique_calendar_name, validate_calendar_name,
    validate_subscription_url, Calendar, CalendarColor, Navigator, SearchResult, View,
    WeekSnapshot,
};
use rmac_calendar_store::{TimeValue, Zone};
use rmac_ui::{
    dialog, dialog_button, mac, AccessibleTextInput as _, ContextMenu, ContextMenuState,
    DialogButtonKind, InputEvent, InputState, StyledExt as _, TextField,
};

use crate::{
    CloseWindow, DeleteCalendar, DeleteEvent, DismissInspector, GoToday, NewCalendar,
    NewCalendarSubscription, NewEvent, NextPeriod, PreviousPeriod, RedoEvent, RenameCalendar,
    SaveEvent, Search, SelectNextDay, SelectNextWeek, SelectPreviousDay, SelectPreviousWeek,
    ShowDay, ShowInspector, ShowInvitations, ShowMonth, ShowSettings, ShowWeek, ShowYear,
    ToggleSidebar, UndoEvent,
};

const SIDEBAR: f32 = 220.0;
const TOOLBAR: f32 = 52.0;
const GUTTER: f32 = 52.0;
const HOUR: f32 = 48.0;
const START_HOUR: f32 = 8.0;

/// Which stack a submitted [`Mutation`] belongs to. A fresh edit (`Do`) starts a new
/// branch of history and clears any pending redos; undoing moves one step onto redo;
/// redoing moves one step back onto undo without disturbing the rest of either stack.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Do,
    Undo,
    Redo,
}

/// Which CAL-6 calendar-list sheet is currently shown, if any.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Sheet {
    RenameCalendar,
    DeleteCalendar,
    NewCalendarSubscription,
}

struct Editor {
    original: Option<rmac_calendar::Event>,
    draft: rmac_calendar_store::Event,
    calendar: usize,
    title: Entity<InputState>,
    location: Entity<InputState>,
    start: Entity<InputState>,
    end: Entity<InputState>,
    notes: Entity<InputState>,
    url: Entity<InputState>,
    invitees: Entity<InputState>,
    all_day: bool,
    repeat: usize,
    alert: usize,
    scope: usize,
    confirm_delete: bool,
}

pub struct CalendarView {
    pub focus: FocusHandle,
    nav: Navigator,
    sidebar_visible: bool,
    scroll_accumulated: f32,
    /// What renders: the loaded snapshot whose `calendars` are
    /// `store::overlay(&base_calendars, &settings)`. `Event::calendar`
    /// indexes the base part; local calendars and subscriptions follow it.
    snapshot: WeekSnapshot,
    /// The calendars exactly as EDS (or the fixture fallback) reported them.
    base_calendars: Vec<Calendar>,
    /// CAL-6's saved prefs (`~/.config/lulo/calendar.json`).
    settings: store::Settings,
    /// UIA-04: Settings ▸ Date & Time's 12/24-hour preference, read once at
    /// startup the same way `settings` above is. Defaults to 12-hour (the
    /// `Locale` default's own fallback) until that load completes.
    twenty_four_hour: bool,
    /// Settings ▸ Accounts lists these (ADR 0022 §1: GOA is the only
    /// account store). Always empty until ACC-2/ACC-3 land.
    accounts: Vec<rmac_accounts::model::Account>,
    menu_at: Option<ContextMenuState>,
    /// Set when a calendar row was right-clicked; `None` means the "+" menu
    /// (New Calendar / New Calendar Subscription…) is open instead.
    context_calendar: Option<usize>,
    sheet: Option<Sheet>,
    sheet_error: Option<SharedString>,
    rename_input: Entity<InputState>,
    rename_color: CalendarColor,
    subscription_name_input: Entity<InputState>,
    subscription_url_input: Entity<InputState>,
    subscription_color: CalendarColor,
    search_open: bool,
    search_input: Entity<InputState>,
    search_results: Vec<SearchResult>,
    search_selected: usize,
    selected: Option<String>,
    editor: Option<Editor>,
    undo: Vec<Mutation>,
    redo: Vec<Mutation>,
    busy: bool,
    error: Option<String>,
    /// CAL-8: the calendar-enabled signed-in accounts' addresses, for
    /// matching this computer's own ATTENDEE line on an invitation.
    self_emails: Vec<String>,
    invitations_open: bool,
    responding: Option<String>,
    /// A `--event`/`--date` launch argument (the CAL-7 reminder's default
    /// action, or the Notification Centre Calendar widget), resolved once
    /// the first EDS snapshot loads.
    deep_link: Option<crate::DeepLink>,
}

impl CalendarView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        deep_link: Option<crate::DeepLink>,
    ) -> Self {
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let subscription_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Team Releases"));
        let subscription_url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("webcal:// or https://…"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.subscribe(
            &search_input,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.update_search(cx),
                InputEvent::PressEnter { .. } => this.jump_to_search_result(cx),
                _ => {}
            },
        )
        .detach();
        let view = Self {
            focus: cx.focus_handle(),
            nav: Navigator::new(current_date()),
            sidebar_visible: true,
            scroll_accumulated: 0.0,
            snapshot: WeekSnapshot::empty(),
            base_calendars: Vec::new(),
            settings: store::Settings::default(),
            twenty_four_hour: false,
            accounts: Vec::new(),
            menu_at: None,
            context_calendar: None,
            sheet: None,
            sheet_error: None,
            rename_input,
            rename_color: CalendarColor::Blue,
            subscription_name_input,
            subscription_url_input,
            subscription_color: CalendarColor::Blue,
            search_open: false,
            search_input,
            search_results: Vec::new(),
            search_selected: 0,
            selected: None,
            editor: None,
            undo: Vec::new(),
            redo: Vec::new(),
            busy: false,
            error: None,
            self_emails: Vec::new(),
            invitations_open: false,
            responding: None,
            deep_link,
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // Saved prefs first, so the first EDS snapshot already renders
            // with the user's colours and visibility.
            let settings = blocking::unblock(|| store::load_settings().unwrap_or_default()).await;
            // UIA-04: Settings ▸ Date & Time's 12/24-hour preference, off
            // the UI thread like everything else `rmac_shell_settings`
            // reads from disk.
            let twenty_four_hour = blocking::unblock(store::twenty_four_hour_preference).await;
            let _ = this.update(cx, |this: &mut CalendarView, cx| {
                this.settings = settings;
                this.twenty_four_hour = twenty_four_hour;
                this.apply_overlay();
                cx.notify();
            });
            let result = blocking::unblock(editing::load).await;
            let _ = this.update(cx, |this: &mut CalendarView, cx| {
                this.accept_load(result, cx)
            });
        })
        .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // CAL-8: who "me" is, for matching an ATTENDEE line on an
            // invitation. Independent of the snapshot load above so a slow
            // or unavailable GOA bus never delays the calendar grid.
            let self_emails = blocking::unblock(editing::self_emails).await;
            let _ = this.update(cx, |this: &mut CalendarView, cx| {
                this.self_emails = self_emails;
                cx.notify();
            });
        })
        .detach();
        view
    }

    fn accept_load(&mut self, result: Result<WeekSnapshot, String>, cx: &mut Context<Self>) {
        match result {
            Ok(snapshot) => {
                self.set_snapshot(snapshot);
                self.error = None;
            }
            Err(error) => {
                // No EDS (or it failed before the first load): show an
                // empty calendar with the error banner. Never show sample
                // events to a real user.
                if self.base_calendars.is_empty() {
                    self.set_snapshot(WeekSnapshot::empty());
                }
                self.error = Some(error);
            }
        }
        self.busy = false;
        self.update_search(cx);
        self.resolve_deep_link();
        cx.notify();
    }

    /// CAL-8: the first (and only -- `load` is one-shot, not a live
    /// subscription) snapshot has just landed, so this is the one chance to
    /// jump to a `--event`/`--date` launch argument. Matches a `--event`
    /// link within a minute of its occurrence to tolerate the small clock
    /// skew between this read and the one that posted the reminder.
    fn resolve_deep_link(&mut self) {
        let Some(link) = self.deep_link.take() else {
            return;
        };
        match link {
            crate::DeepLink::Date(date) => {
                self.nav.view = View::Day;
                self.nav.selected = date;
            }
            crate::DeepLink::Event {
                calendar_uid,
                event_uid,
                occurrence_start,
            } => {
                let found = self.snapshot.events.iter().find(|event| {
                    self.snapshot
                        .calendars
                        .get(event.calendar)
                        .and_then(|calendar| calendar.source_uid.as_deref())
                        == Some(calendar_uid.as_str())
                        && event
                            .ical
                            .as_ref()
                            .is_some_and(|ical| ical.uid == event_uid)
                        && (event.start - occurrence_start).num_seconds().abs() <= 60
                });
                if let Some(event) = found {
                    self.nav.view = View::Day;
                    self.nav.selected = event.start.date_naive();
                    self.selected = Some(event.id.clone());
                }
            }
        }
    }

    fn set_snapshot(&mut self, mut snapshot: WeekSnapshot) {
        self.base_calendars = std::mem::take(&mut snapshot.calendars);
        self.snapshot = snapshot;
        self.apply_overlay();
    }

    /// Rebuilds the rendered calendar list from the base list and prefs.
    fn apply_overlay(&mut self) {
        self.snapshot.calendars = store::overlay(&self.base_calendars, &self.settings);
    }

    /// Applies a prefs change, re-overlays and saves off the UI thread.
    fn update_settings(
        &mut self,
        change: impl FnOnce(&mut store::Settings),
        cx: &mut Context<Self>,
    ) {
        change(&mut self.settings);
        self.settings = std::mem::take(&mut self.settings).normalized();
        self.apply_overlay();
        let settings = self.settings.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = store::save_settings(&settings) {
                    eprintln!("rmac-calendar: could not save settings: {error}");
                }
            })
            .detach();
        self.update_search(cx);
        cx.notify();
    }

    /// The flags `events_on_day`/`search_events` expect: a calendar is
    /// shown exactly when it is ticked and not deleted.
    fn visible_flags(&self) -> Vec<bool> {
        self.snapshot
            .calendars
            .iter()
            .map(|calendar| calendar.visible && !calendar.removed)
            .collect()
    }

    fn is_visible(&self, calendar: usize) -> bool {
        self.snapshot
            .calendars
            .get(calendar)
            .is_some_and(|calendar| calendar.visible && !calendar.removed)
    }

    /// UIA-04: event and "now" times follow `self.twenty_four_hour`
    /// (Settings ▸ Date & Time) instead of always reading 24-hour, the way
    /// the Mac's Calendar follows the system clock format.
    fn time_format_str(&self) -> &'static str {
        if self.twenty_four_hour {
            "%-H:%M"
        } else {
            "%-I:%M %p"
        }
    }

    /// Where a new event goes: Settings' default calendar when it's
    /// writable, else the first writable calendar still in the list.
    fn default_writable_calendar(&self) -> Option<usize> {
        let candidates = || {
            self.snapshot
                .calendars
                .iter()
                .enumerate()
                .filter(|(_, calendar)| calendar.writable && !calendar.removed)
        };
        let default = &self.settings.general.default_calendar;
        candidates()
            .find(|(_, calendar)| !default.is_empty() && &calendar.name == default)
            .or_else(|| candidates().next())
            .map(|(index, _)| index)
    }

    pub(crate) fn calendar_names(&self) -> Vec<SharedString> {
        self.snapshot
            .calendars
            .iter()
            .filter(|calendar| !calendar.removed)
            .map(|calendar| SharedString::from(calendar.name.clone()))
            .collect()
    }

    pub(crate) fn general(&self) -> store::GeneralSettings {
        self.settings.general.clone()
    }

    pub(crate) fn accounts(&self) -> &[rmac_accounts::model::Account] {
        &self.accounts
    }

    pub(crate) fn set_general(&mut self, general: store::GeneralSettings, cx: &mut Context<Self>) {
        self.update_settings(|settings| settings.general = general, cx);
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

    fn toggle_calendar(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(calendar) = self.snapshot.calendars.get(index) else {
            return;
        };
        let (id, visible) = (calendar.id.clone(), !calendar.visible);
        self.update_settings(
            |settings| settings.prefs_mut(&id).visible = Some(visible),
            cx,
        );
    }

    fn create_new_calendar(&mut self, cx: &mut Context<Self>) {
        let name = unique_calendar_name(&self.snapshot.calendars, "New Calendar");
        let color = Self::next_color(&self.snapshot.calendars);
        self.update_settings(
            |settings| {
                settings.add_calendar(name, "On My Mac", color, None);
            },
            cx,
        );
    }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(calendar) = self
            .context_calendar
            .and_then(|index| self.snapshot.calendars.get(index))
        else {
            return;
        };
        let (name, color) = (calendar.name.clone(), calendar.color);
        self.rename_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.rename_color = color;
        self.sheet_error = None;
        self.sheet = Some(Sheet::RenameCalendar);
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(calendar) = self
            .context_calendar
            .and_then(|index| self.snapshot.calendars.get(index))
        else {
            return;
        };
        let id = calendar.id.clone();
        let value = self.rename_input.read(cx).value().to_string();
        match validate_calendar_name(&value) {
            Ok(name) => {
                let color = self.rename_color;
                self.sheet = None;
                self.sheet_error = None;
                self.update_settings(
                    |settings| {
                        let prefs = settings.prefs_mut(&id);
                        prefs.name = Some(name);
                        prefs.color = Some(color);
                    },
                    cx,
                );
            }
            Err(_) => {
                self.sheet_error = Some("Give this calendar a name.".into());
                cx.notify();
            }
        }
    }

    fn start_delete(&mut self, cx: &mut Context<Self>) {
        if self.context_calendar.is_none() {
            return;
        }
        self.sheet = Some(Sheet::DeleteCalendar);
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        let Some(id) = self
            .context_calendar
            .take()
            .and_then(|index| self.snapshot.calendars.get(index))
            .map(|calendar| calendar.id.clone())
        else {
            cx.notify();
            return;
        };
        self.update_settings(|settings| settings.remove_calendar(&id), cx);
    }

    fn start_new_calendar_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription_name_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.subscription_url_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.subscription_color = Self::next_color(&self.snapshot.calendars);
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
            unique_calendar_name(&self.snapshot.calendars, &subscription_default_name(&url))
        });
        let color = self.subscription_color;
        self.sheet = None;
        self.sheet_error = None;
        self.update_settings(
            |settings| {
                settings.add_calendar(name, "Subscribed", color, Some(url));
            },
            cx,
        );
    }

    /// Escape: closes the inspector, then a CAL-6 sheet, then search, then
    /// the CAL-8 invitations popover. Whichever it closes, keyboard focus
    /// moves back to the main view (ACC-32): closing one of these used to
    /// leave focus on the FocusHandle of a field that had just been
    /// dropped, which Orca reports as nothing focused at all.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            self.editor = None;
        } else if self.sheet.is_some() {
            self.sheet = None;
            self.sheet_error = None;
        } else if self.search_open {
            self.search_open = false;
        } else if self.invitations_open {
            self.invitations_open = false;
        } else {
            return;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn toggle_invitations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invitations_open = !self.invitations_open;
        if !self.invitations_open {
            // ACC-32: closing the popover (Escape goes through `dismiss`;
            // this is the toggle button and the app-menu item) must not
            // leave focus on a row inside it that is about to stop showing.
            window.focus(&self.focus, cx);
        }
        self.sync_menu(cx);
        cx.notify();
    }

    fn pending_invitations(&self) -> Vec<invitations::PendingInvitation> {
        invitations::pending(&self.snapshot, &self.self_emails)
    }

    /// Accept/Maybe/Decline (CAL-8): optimistically rewrites the local
    /// PARTSTAT so the popover drops the invitation immediately, then
    /// writes through the EDS adapter on a worker. A write failure shows
    /// the error banner and restores the invitation to the pending list.
    fn respond_to_invitation(
        &mut self,
        event_id: String,
        response: invitations::Response,
        cx: &mut Context<Self>,
    ) {
        if self.responding.is_some() {
            return;
        }
        let Some(index) = self
            .snapshot
            .events
            .iter()
            .position(|event| event.id == event_id)
        else {
            return;
        };
        let Some(ical) = self.snapshot.events[index].ical.clone() else {
            return;
        };
        let Some(source) = self
            .snapshot
            .calendars
            .get(self.snapshot.events[index].calendar)
            .and_then(|calendar| calendar.source_uid.clone())
        else {
            return;
        };
        let Some(self_email) =
            invitations::my_attendee(&ical, &self.self_emails).map(|attendee| attendee.email)
        else {
            return;
        };
        let Some(updated) = invitations::apply_response(&ical, &self_email, response) else {
            return;
        };
        self.snapshot.events[index].ical = Some(updated.clone());
        self.responding = Some(event_id.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(move || editing::respond_to_invitation(&source, &updated)).await;
            let _ = this.update(cx, |this, cx| {
                this.responding = None;
                if let Err(error) = result {
                    this.error = Some(error);
                    if let Some(event) = this
                        .snapshot
                        .events
                        .iter_mut()
                        .find(|event| event.id == event_id)
                    {
                        event.ical = Some(ical);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = !self.search_open;
        if self.search_open {
            self.search_input
                .update(cx, |state, cx| state.focus(window, cx));
            self.update_search(cx);
        } else {
            self.search_results.clear();
        }
        cx.notify();
    }

    /// Searches every loaded event (EDS loads two years back and three
    /// ahead) in shown calendars.
    fn update_search(&mut self, cx: &mut Context<Self>) {
        if !self.search_open {
            return;
        }
        let query = self.search_input.read(cx).value().to_string();
        self.search_results = search_events(&self.snapshot, &self.visible_flags(), &query);
        self.search_selected = 0;
        cx.notify();
    }

    fn jump_to_search_result(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.search_results.get(self.search_selected).cloned() else {
            return;
        };
        self.nav.view = View::Day;
        self.nav.selected = result.start.date_naive();
        self.selected = Some(result.event_id);
        self.search_open = false;
        self.sync_menu(cx);
        cx.notify();
    }

    fn field(window: &mut Window, cx: &mut Context<Self>, value: String) -> Entity<InputState> {
        cx.new(|cx| InputState::new(window, cx).default_value(value))
    }

    /// The Alert field's options: an all-day event offers "N days/weeks
    /// before" (fired at 9 AM), a timed event offers minute/hour offsets
    /// (CAL-5's inspector and CAL-7's Default Alerts share this list).
    fn alert_offsets(all_day: bool) -> &'static [editing::AlertOffset] {
        if all_day {
            editing::ALL_DAY_ALERT_OFFSETS
        } else {
            editing::EVENT_ALERT_OFFSETS
        }
    }

    fn open_editor(
        &mut self,
        event: rmac_calendar::Event,
        fresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = event.ical.clone() else {
            return;
        };
        let title = Self::field(window, cx, event.title.clone());
        cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_editor(cx);
            }
        })
        .detach();
        let location = Self::field(window, cx, event.location.clone());
        let start = Self::field(window, cx, event.start.format("%Y-%m-%d %H:%M").to_string());
        let end = Self::field(window, cx, event.end.format("%Y-%m-%d %H:%M").to_string());
        let notes = Self::field(window, cx, editing::property(&draft, "DESCRIPTION"));
        let url = Self::field(window, cx, editing::property(&draft, "URL"));
        let invitees = Self::field(
            window,
            cx,
            draft
                .other_properties
                .iter()
                .filter_map(|line| line.strip_prefix("ATTENDEE:mailto:"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        let repeat = match draft.rrules.first().map(String::as_str) {
            Some(rule) if rule.starts_with("FREQ=DAILY") => 1,
            Some(rule) if rule.starts_with("FREQ=WEEKLY") => 2,
            Some(rule) if rule.starts_with("FREQ=MONTHLY") => 3,
            _ => 0,
        };
        let alert_offsets = Self::alert_offsets(event.all_day);
        let alert_offset =
            editing::AlertOffset::from_properties(&draft.other_properties, event.all_day);
        let alert = alert_offsets
            .iter()
            .position(|option| *option == alert_offset)
            .unwrap_or(0);
        self.selected = Some(event.id.clone());
        self.editor = Some(Editor {
            original: (!fresh).then_some(event.clone()),
            draft,
            calendar: event.calendar,
            title: title.clone(),
            location,
            start,
            end,
            notes,
            url,
            invitees,
            all_day: event.all_day,
            repeat,
            alert,
            scope: 2,
            confirm_delete: false,
        });
        let focus = title.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn new_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Utc::now();
        let hour = (now.hour() + 1).clamp(8, 19);
        self.create_event_at(self.nav.selected, hour, window, cx);
    }

    /// Double-clicking an empty time slot (Day/Week grid) creates a one-hour event there.
    fn create_event_at(
        &mut self,
        date: NaiveDate,
        hour: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(calendar) = self.default_writable_calendar() else {
            self.error = Some("No writable calendar is available".into());
            cx.notify();
            return;
        };
        let Some((start, end)) = editing::at_day(date, hour) else {
            return;
        };
        let mut draft = editing::new_event(start, end, false);
        self.settings
            .general
            .default_alerts
            .events
            .apply(&mut draft, false);
        let event = rmac_calendar::Event {
            id: draft.uid.clone(),
            title: draft.summary.clone(),
            location: String::new(),
            calendar,
            start,
            end,
            all_day: false,
            ical: Some(draft),
        };
        self.open_editor(event, true, window, cx);
    }

    /// Double-clicking an all-day strip cell or a Month view day creates an all-day event.
    fn create_all_day_event(
        &mut self,
        date: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(calendar) = self.default_writable_calendar() else {
            self.error = Some("No writable calendar is available".into());
            cx.notify();
            return;
        };
        let Some(start) = date.and_hms_opt(0, 0, 0).map(|naive| naive.and_utc()) else {
            return;
        };
        let Some(end) = (date + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .map(|naive| naive.and_utc())
        else {
            return;
        };
        let mut draft = editing::new_event(start, end, true);
        self.settings
            .general
            .default_alerts
            .all_day_events
            .apply(&mut draft, true);
        let event = rmac_calendar::Event {
            id: draft.uid.clone(),
            title: draft.summary.clone(),
            location: String::new(),
            calendar,
            start,
            end,
            all_day: true,
            ical: Some(draft),
        };
        self.open_editor(event, true, window, cx);
    }

    fn inspect_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(event) = self
            .selected
            .as_ref()
            .and_then(|id| self.snapshot.events.iter().find(|event| &event.id == id))
            .cloned()
        else {
            return;
        };
        self.open_editor(event, false, window, cx);
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        if self.busy {
            return;
        }
        let title = editor.title.read(cx).value().to_string();
        let location = editor.location.read(cx).value().to_string();
        let start = editor.start.read(cx).value().to_string();
        let end = editor.end.read(cx).value().to_string();
        let notes = editor.notes.read(cx).value().to_string();
        let url = editor.url.read(cx).value().to_string();
        let invitees = editor.invitees.read(cx).value().to_string();
        let parse = |text: &str| NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M");
        let (Ok(start), Ok(end)) = (parse(&start), parse(&end)) else {
            self.error = Some("Enter dates as YYYY-MM-DD HH:MM".into());
            cx.notify();
            return;
        };
        if end <= start || title.trim().is_empty() {
            self.error = Some("Give the event a title and an end after its start".into());
            cx.notify();
            return;
        }
        let mut after = editor.draft.clone();
        after.summary = title.trim().to_owned();
        after.start = TimeValue {
            local: start,
            zone: if editor.all_day {
                Zone::Date
            } else {
                Zone::Utc
            },
        };
        after.end = TimeValue {
            local: end,
            zone: if editor.all_day {
                Zone::Date
            } else {
                Zone::Utc
            },
        };
        after.rrules = match editor.repeat {
            1 => vec!["FREQ=DAILY".into()],
            2 => vec!["FREQ=WEEKLY".into()],
            3 => vec!["FREQ=MONTHLY".into()],
            _ => Vec::new(),
        };
        editing::set_property(&mut after, "LOCATION", &location);
        editing::set_property(&mut after, "DESCRIPTION", &notes);
        editing::set_property(&mut after, "URL", &url);
        // The Alert field's own offset (just below) owns VALARM/TRIGGER;
        // this only drops the invitee lines the fields below rebuild.
        after
            .other_properties
            .retain(|line| !line.starts_with("ATTENDEE:"));
        for address in invitees
            .split(',')
            .map(str::trim)
            .filter(|address| !address.is_empty())
        {
            if address.contains('@') && !address.contains(['\n', '\r']) {
                after
                    .other_properties
                    .push(format!("ATTENDEE:mailto:{address}"));
            }
        }
        Self::alert_offsets(editor.all_day)
            .get(editor.alert)
            .copied()
            .unwrap_or(editing::AlertOffset::None)
            .apply(&mut after, editor.all_day);
        let Some(source) = self
            .snapshot
            .calendars
            .get(editor.calendar)
            .and_then(|calendar| calendar.source_uid.clone())
        else {
            return;
        };
        let change = if let Some(original) = &editor.original {
            let Some(before) = original.ical.clone() else {
                return;
            };
            let Some(old_source) = self
                .snapshot
                .calendars
                .get(original.calendar)
                .and_then(|calendar| calendar.source_uid.clone())
            else {
                return;
            };
            if old_source != source {
                Mutation::MoveSource {
                    from: old_source,
                    to: source,
                    before,
                    after,
                }
            } else {
                Mutation::Modify {
                    source,
                    before,
                    after,
                    scope: ["this", "this-and-future", "all"][editor.scope],
                }
            }
        } else {
            Mutation::Create {
                source,
                event: after,
            }
        };
        self.editor = None;
        self.submit(change, Direction::Do, cx);
    }

    fn submit(&mut self, change: Mutation, direction: Direction, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock({
                let change = change.clone();
                move || editing::apply(&change).and_then(|_| editing::load())
            })
            .await;
            let _ = this.update(cx, |this: &mut CalendarView, cx| match result {
                Ok(snapshot) => {
                    // Undo moves this change onto the redo stack; redo moves it back onto
                    // undo without touching the rest of the redo stack still queued behind
                    // it. Only a genuinely new edit (Direction::Do) clears redo: it is a
                    // fresh branch of history, not a continuation of the one being replayed.
                    match direction {
                        Direction::Undo => this.redo.push(change.inverse()),
                        Direction::Redo => this.undo.push(change.inverse()),
                        Direction::Do => {
                            this.undo.push(change.inverse());
                            this.redo.clear();
                        }
                    }
                    this.accept_load(Ok(snapshot), cx);
                }
                Err(error) => {
                    this.busy = false;
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(event) = self
            .selected
            .as_ref()
            .and_then(|id| self.snapshot.events.iter().find(|event| &event.id == id))
            .cloned()
        else {
            return;
        };
        let Some(ical) = event.ical else {
            return;
        };
        let Some(source) = self
            .snapshot
            .calendars
            .get(event.calendar)
            .and_then(|calendar| calendar.source_uid.clone())
        else {
            return;
        };
        if !ical.rrules.is_empty()
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| !editor.confirm_delete)
        {
            if let Some(editor) = &mut self.editor {
                editor.confirm_delete = true;
            }
            cx.notify();
            return;
        }
        let scope = self.editor.as_ref().map_or("all", |editor| {
            ["this", "this-and-future", "all"][editor.scope]
        });
        // ACC-32: Delete/Backspace can close the editor sheet along with
        // removing the event; keep focus on the main view rather than on
        // the sheet field that just stopped showing.
        self.editor = None;
        self.selected = None;
        window.focus(&self.focus, cx);
        self.submit(
            Mutation::Delete {
                source,
                event: ical,
                scope,
            },
            Direction::Do,
            cx,
        );
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(change) = self.undo.pop() {
            self.submit(change, Direction::Undo, cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(change) = self.redo.pop() {
            self.submit(change, Direction::Redo, cx);
        }
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
        rmac_ui::set_menu_checked("calendar::ShowInvitations", self.invitations_open, cx);
        rmac_ui::set_menu_enabled("calendar::UndoEvent", !self.undo.is_empty(), cx);
        rmac_ui::set_menu_enabled("calendar::RedoEvent", !self.redo.is_empty(), cx);
        rmac_ui::set_menu_enabled("calendar::ShowInspector", self.selected.is_some(), cx);
        rmac_ui::set_menu_enabled("calendar::DeleteEvent", self.selected.is_some(), cx);
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
        self.control_sized(id, label, accessible, active, px(12.0), click, cx)
    }

    /// UIA-04: the Mac's toolbar draws its symbol glyphs (☷ ▢ + ⌕) at 16 pt
    /// inside the 28 pt buttons; this crate's text labels ("Today", "All
    /// Day", "Daily"…) share [`Self::control`]'s 12 pt, so the icon-only
    /// buttons go through this sized variant instead of bumping every
    /// label.
    fn control_sized(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        accessible: &'static str,
        active: bool,
        text_size: gpui::Pixels,
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
            .text_size(text_size)
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
        bar = bar.child(self.control_sized(
            "calendar-sidebar",
            "☷",
            "Sidebar",
            false,
            px(16.0),
            |this, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                this.sync_menu(cx);
                cx.notify();
            },
            cx,
        ));
        let pending_count = self.pending_invitations().len();
        bar = bar.child(
            div()
                .id("calendar-inbox")
                .role(Role::Button)
                .aria_label(if pending_count > 0 {
                    format!("Invitations, {pending_count} pending")
                } else {
                    "Invitations".to_owned()
                })
                .aria_selected(self.invitations_open)
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(mac::radius_pill()))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .bg(if self.invitations_open {
                    mac::control_fill()
                } else {
                    mac::material_clear()
                })
                .text_color(mac::text())
                .text_size(px(16.0))
                .child("▢")
                .when(pending_count > 0, |control| {
                    control.child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(mac::system_red())
                            .child(pending_count.to_string()),
                    )
                })
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_invitations(window, cx)
                })),
        );
        bar = bar.child(
            div()
                .id("calendar-new")
                .role(Role::Button)
                .aria_label("New Event")
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(mac::radius_pill()))
                .flex()
                .items_center()
                .justify_center()
                .bg(mac::material_clear())
                .text_color(mac::text())
                .text_size(px(16.0))
                .child("+")
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.new_event(window, cx)),
                ),
        );
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
                .text_size(px(16.0))
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
        let mut grid = div()
            .id("calendar-mini-month")
            .flex()
            .flex_wrap()
            .w_full()
            .role(Role::Table)
            .aria_label(self.nav.selected.format("%B %Y").to_string());
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
            let selected = day == self.nav.selected;
            grid = grid.child(
                div()
                    .id(format!("mini-{}", offset))
                    .role(Role::Button)
                    .aria_label(day.format("%A %-d %B %Y").to_string())
                    .aria_selected(selected)
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
            .id("calendar-list")
            .role(Role::List)
            .aria_label("Calendars")
            .absolute()
            .top(px(48.0))
            .left(px(10.0))
            .right(px(10.0))
            .flex()
            .flex_col();
        let mut account = String::new();
        for (index, calendar) in self.snapshot.calendars.iter().enumerate() {
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
                    // ACC-32: this row toggles whether the calendar's
                    // events show, exactly like a checkbox (and like real
                    // Calendar's own coloured checkbox) — not a selectable
                    // row, which told Orca nothing about the checked state.
                    .role(Role::CheckBox)
                    .aria_label(calendar.name.clone())
                    .aria_toggled(if visible {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
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
                        this.toggle_calendar(index, cx);
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
        let spoken_range = if subtitle.is_empty() {
            title.clone()
        } else {
            format!("{title} {subtitle}")
        };
        div()
            .h(px(44.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .child(
                // ACC-32: a named, polite live region so Orca announces the
                // new period (Next/Previous Period, Go Today, Day/Week/
                // Month/Year) even though this text never takes keyboard
                // focus. `Role::Status` plus a name is not enough on its
                // own — confirmed live on the laptop: Orca only announces
                // an unfocused name change when the node also carries
                // AccessKit's `live`/`live_atomic` (the same fix Calculator's
                // display and Spotlight's result announcer use).
                div()
                    .id("calendar-heading")
                    .role(Role::Status)
                    .aria_label(spoken_range)
                    .a11y_synthetic_children(|builder| {
                        builder.parent_node().set_live(accesskit::Live::Polite);
                        builder.parent_node().set_live_atomic();
                    })
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
                    ),
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

    #[allow(clippy::too_many_arguments)]
    fn time_grid(
        &self,
        snapshot: &WeekSnapshot,
        width: f32,
        height: f32,
        first: NaiveDate,
        days: usize,
        origin_x: f32,
        origin_y: f32,
        cx: &mut Context<Self>,
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
            })
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if e.click_count() >= 2 {
                    let position = e.position();
                    let local_x = f32::from(position.x) - origin_x;
                    let local_y = f32::from(position.y) - origin_y;
                    if local_y < grid_top || local_x < GUTTER {
                        return;
                    }
                    let day_index = ((local_x - GUTTER) / day_width)
                        .floor()
                        .clamp(0.0, (days as f32 - 1.0).max(0.0))
                        as i64;
                    let day = first + Duration::days(day_index);
                    let hour_frac = START_HOUR + (local_y - grid_top) / HOUR;
                    let hour = (hour_frac.round() as i64).clamp(0, 23) as u32;
                    this.create_event_at(day, hour, window, cx);
                } else {
                    this.selected = None;
                    cx.notify();
                }
            }));
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
            if (0..days as i64).contains(&offset) && self.is_visible(event.calendar) {
                let days = (event.end.date_naive() - day)
                    .num_days()
                    .clamp(1, days as i64 - offset);
                let event_id = event.id.clone();
                week = week.child(
                    div()
                        .id(format!("calendar-allday-event-{}", event.id))
                        .absolute()
                        .left(px(GUTTER + offset as f32 * day_width + 2.0))
                        .top(px(37.0))
                        .w(px(days as f32 * day_width - 4.0))
                        .h(px(19.0))
                        .rounded(px(mac::radius_control()))
                        .px(px(5.0))
                        .overflow_hidden()
                        .bg(Self::color(snapshot.calendars[event.calendar].color))
                        .text_color(mac::white())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(event.title.clone())
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.selected = Some(event_id.clone());
                            if e.click_count() >= 2 {
                                this.inspect_selected(window, cx);
                            }
                            cx.notify();
                            cx.stop_propagation();
                        })),
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
                // UIA-04: this always read "9:00" in 24-hour even with a
                // 12-hour preference; the on-the-hour label now follows
                // `self.twenty_four_hour` like the event times below.
                let label = if self.twenty_four_hour {
                    format!("{hour}:00")
                } else {
                    NaiveTime::from_hms_opt(hour as u32, 0, 0)
                        .map(|time| time.format("%-I %p").to_string())
                        .unwrap_or_else(|| format!("{hour}:00"))
                };
                week = week.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(y - 7.0))
                        .w(px(44.0))
                        .text_right()
                        .text_size(px(10.0))
                        .text_color(mac::text_secondary())
                        .child(label),
                );
            }
        }
        for slot in &snapshot.slots_for(first) {
            let Some(event) = snapshot.events.iter().find(|event| event.id == slot.id) else {
                continue;
            };
            if !self.is_visible(event.calendar) {
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
            let color = Self::color(snapshot.calendars[event.calendar].color);
            let event_id = event.id.clone();
            let accessible_name = if event.location.is_empty() {
                format!(
                    "{}, {}",
                    event.title,
                    event.start.format(self.time_format_str())
                )
            } else {
                format!(
                    "{}, {}, {}",
                    event.title,
                    event.start.format(self.time_format_str()),
                    event.location
                )
            };
            week = week.child(
                div()
                    .id(format!("calendar-event-{}", event.id))
                    // ACC-32: a selectable, openable event used to have no
                    // role or name at all — Orca had nothing to say about
                    // it, focused or not.
                    .role(Role::Button)
                    .aria_label(accessible_name)
                    .aria_selected(self.selected.as_deref() == Some(event.id.as_str()))
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
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(event.title.clone()),
                    )
                    .child(div().text_color(mac::text_secondary()).child(format!(
                        "{}{}",
                        event.start.format(self.time_format_str()),
                        if event.location.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", event.location)
                        }
                    )))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.selected = Some(event_id.clone());
                        if e.click_count() >= 2 {
                            this.inspect_selected(window, cx);
                        }
                        cx.notify();
                        cx.stop_propagation();
                    })),
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
                        .child(now.format(self.time_format_str()).to_string()),
                );
            }
        }
        week.into_any_element()
    }

    fn day(
        &self,
        snapshot: &WeekSnapshot,
        width: f32,
        height: f32,
        origin_x: f32,
        origin_y: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            let color = Self::color(snapshot.calendars[event.calendar].color);
            let event_id = event.id.clone();
            let accessible_name = if event.all_day {
                format!("{}, all day", event.title)
            } else {
                format!(
                    "{}, {}–{}, {}",
                    event.title,
                    event.start.format(self.time_format_str()),
                    event.end.format(self.time_format_str()),
                    snapshot.calendars[event.calendar].name
                )
            };
            detail = detail.child(
                div()
                    .id(format!("calendar-day-detail-{}", event.id))
                    // ACC-32: same fix as the grid's event blocks above.
                    .role(Role::Button)
                    .aria_label(accessible_name)
                    .aria_selected(self.selected.as_deref() == Some(event.id.as_str()))
                    .border_l_3()
                    .border_color(color)
                    .pl(px(9.0))
                    .py(px(5.0))
                    .rounded(px(mac::radius_control()))
                    .bg(color.opacity(0.18))
                    .flex()
                    .flex_col()
                    .text_size(px(12.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(event.title.clone()),
                    )
                    .child(
                        div()
                            .text_color(mac::text_secondary())
                            .child(if event.all_day {
                                "All day".to_string()
                            } else {
                                format!(
                                    "{}–{} · {}",
                                    event.start.format(self.time_format_str()),
                                    event.end.format(self.time_format_str()),
                                    snapshot.calendars[event.calendar].name
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.selected = Some(event_id.clone());
                        if e.click_count() >= 2 {
                            this.inspect_selected(window, cx);
                        }
                        cx.notify();
                    })),
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
                    .child(self.time_grid(
                        snapshot,
                        grid_width,
                        height,
                        self.nav.selected,
                        1,
                        origin_x,
                        origin_y,
                        cx,
                    )),
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
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if e.click_count() >= 2 {
                        this.create_all_day_event(day, window, cx);
                    } else {
                        this.nav.selected = day;
                        cx.notify();
                    }
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
                let color = Self::color(snapshot.calendars[event.calendar].color);
                let event_id = event.id.clone();
                let mut item = div()
                    .id(format!("calendar-month-event-{}", event.id))
                    // ACC-32: same fix as the week/day grids' event blocks.
                    .role(Role::Button)
                    .aria_label(event.title.clone())
                    .aria_selected(self.selected.as_deref() == Some(event.id.as_str()))
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
                let item = item.child(event.title.clone()).on_click(cx.listener(
                    move |this, e: &ClickEvent, window, cx| {
                        this.selected = Some(event_id.clone());
                        if e.click_count() >= 2 {
                            this.inspect_selected(window, cx);
                        }
                        cx.notify();
                        cx.stop_propagation();
                    },
                ));
                cell = cell.child(item);
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
                // UIA-03: a trailing `.relative()` here overwrote the
                // `.absolute()` position set above (last call wins), so
                // each card fell back into normal flow with its left/top
                // added as a relative offset on top of that flow
                // position -- a diagonal staircase that compounded with
                // every month. `.absolute()` alone already gives this div's
                // own absolute-positioned children (the weekday labels and
                // day numbers below) a containing block, matching the
                // month-view day cells above.
                .child(
                    div()
                        .id(format!("calendar-year-month-{}", month_index + 1))
                        // ACC-32: this heading opens the month in Month
                        // view but had no role or name of its own.
                        .role(Role::Button)
                        .aria_label(first.format("%B %Y").to_string())
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

    /// The event inspector: a modal sheet (consistent with Clock's alarm editor and the
    /// shared `rmac_ui::dialog` sheets elsewhere) rather than a Mac-style arrow-pointing
    /// popover (verify on Mac; CAL-5 ships the simplified form first).
    /// The toolbar search field and its results popover (CAL-6).
    fn render_search(&self, cx: &mut Context<Self>) -> AnyElement {
        let field = div()
            .absolute()
            .right(px(9.0))
            .top(px(8.0))
            .w(px(260.0))
            .child(
                // ACC-32: bare `SearchField` publishes no accessible name
                // (confirmed live: Orca read it as "(unnamed), entry").
                // The same fix as Terminal's and Notes' Find fields: a
                // named wrapper folds the field's own text node into it.
                div()
                    .id("calendar-search-field")
                    .role(Role::SearchInput)
                    .aria_label("Search")
                    .accessible_text_input(&self.search_input, cx)
                    .child(rmac_ui::SearchField::new(&self.search_input).small()),
            );
        let mut popover = div()
            .id("calendar-search-results")
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
                        "Type to search events."
                    } else {
                        "No matching events."
                    }),
            );
        }
        for (index, result) in self.search_results.iter().enumerate() {
            let selected = index == self.search_selected;
            let color = Self::color(
                self.snapshot
                    .calendars
                    .get(result.calendar)
                    .map_or(CalendarColor::Blue, |calendar| calendar.color),
            );
            let title = self
                .snapshot
                .events
                .iter()
                .find(|event| event.id == result.event_id)
                .map(|event| event.title.clone())
                .unwrap_or_default();
            popover = popover.child(
                div()
                    .id(format!("calendar-search-result-{index}"))
                    .role(Role::ListBoxOption)
                    .aria_label(title.clone())
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

    /// The toolbar inbox's invitations popover (CAL-8): every event this
    /// computer has an outstanding RSVP for, with Accept/Maybe/Decline.
    /// Pointed at the inbox icon the same way the search results are
    /// pointed at the search field.
    fn render_invitations(&self, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.pending_invitations();
        let mut popover = div()
            .id("calendar-invitations")
            .role(Role::Dialog)
            .aria_label("Invitations")
            .absolute()
            .right(px(9.0))
            .top(px(44.0))
            .w(px(300.0))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0));
        if pending.is_empty() {
            popover = popover.child(
                div()
                    .p(px(8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child("No pending invitations."),
            );
        }
        for invitation in &pending {
            let busy = self.responding.as_deref() == Some(invitation.event_id.as_str());
            let color = Self::color(
                self.snapshot
                    .calendars
                    .get(invitation.calendar)
                    .map_or(CalendarColor::Blue, |calendar| calendar.color),
            );
            let when = if invitation.all_day {
                invitation.start.format("%a %-d %b").to_string()
            } else {
                invitation.start.format("%a %-d %b, %-I:%M %p").to_string()
            };
            let mut row = div()
                .id(SharedString::from(format!(
                    "calendar-invitation-{}",
                    invitation.event_id
                )))
                .v_flex()
                .gap(px(4.0))
                .p(px(8.0))
                .rounded(px(mac::radius_control()))
                .bg(mac::material_clear())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().size(px(7.0)).rounded_full().bg(color))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(mac::text())
                                .child(invitation.title.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(match &invitation.organizer {
                            Some(organizer) => format!("{when} · {organizer}"),
                            None => when,
                        }),
                );
            let mut actions = div().flex().gap(px(6.0));
            for response in invitations::Response::ALL {
                let event_id = invitation.event_id.clone();
                actions = actions.child(
                    div()
                        .id(SharedString::from(format!(
                            "calendar-invitation-{}-{}",
                            invitation.event_id,
                            response.label()
                        )))
                        .role(Role::Button)
                        .aria_label(if busy {
                            format!("{}, working", response.label())
                        } else {
                            response.label().to_owned()
                        })
                        .flex_1()
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(mac::radius_control()))
                        .bg(mac::control_fill())
                        .text_size(px(11.0))
                        .text_color(if busy {
                            mac::text_secondary()
                        } else {
                            mac::text()
                        })
                        .child(response.label())
                        .when(!busy, |button| {
                            button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.respond_to_invitation(event_id.clone(), response, cx);
                            }))
                        }),
                );
            }
            row = row.child(actions);
            popover = popover.child(row);
        }
        div()
            .id("calendar-invitations-overlay")
            .absolute()
            .inset_0()
            .child(popover)
            .into_any_element()
    }

    fn color_swatches(
        &self,
        id_prefix: &'static str,
        current: CalendarColor,
        pick: fn(&mut Self, CalendarColor),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut swatches = div().flex().gap(px(7.0));
        for color in CalendarColor::ALL {
            swatches = swatches.child(
                div()
                    .id(format!("{id_prefix}-{color:?}"))
                    .role(Role::RadioButton)
                    .aria_label(format!("{color:?}"))
                    .aria_selected(current == color)
                    .size(px(18.0))
                    .rounded_full()
                    .bg(Self::color(color))
                    .when(current == color, |el| {
                        el.border_2().border_color(mac::text())
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        pick(this, color);
                        cx.notify();
                    })),
            );
        }
        swatches
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
                card = card
                    .child(rmac_ui::TextField::new(&self.rename_input).w_full())
                    .child(self.color_swatches(
                        "calendar-rename-color",
                        self.rename_color,
                        |this, color| this.rename_color = color,
                        cx,
                    ));
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
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
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
                    .and_then(|index| self.snapshot.calendars.get(index))
                    .map(|calendar| calendar.name.clone())
                    .unwrap_or_default();
                return Some(
                    rmac_ui::alert_cancel_default(
                        format!("Remove “{name}” from Calendar?"),
                        "It disappears from this list. Its events stay in the account it belongs to.",
                        vec![
                            rmac_ui::dialog_button(
                                "calendar-delete-cancel",
                                "Cancel",
                                DialogButtonKind::Normal,
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx)))
                            .into_any_element(),
                            rmac_ui::dialog_button(
                                "calendar-delete-confirm",
                                "Remove",
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
                card = card.child(self.color_swatches(
                    "calendar-subscription-color",
                    self.subscription_color,
                    |this, color| this.subscription_color = color,
                    cx,
                ));
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
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
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
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.dismiss(window, cx);
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

    fn inspector(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        let is_new = editor.original.is_none();
        let recurring = !editor.draft.rrules.is_empty();

        let mut card = div()
            .id("calendar-inspector")
            .w(px(420.0))
            .max_h(px(620.0))
            .overflow_y_scroll()
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::sheet())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(mac::text())
                    .child(if is_new { "New Event" } else { "Edit Event" }),
            )
            .child(labelled_field("Title", TextField::new(&editor.title)))
            .child(labelled_field("Location", TextField::new(&editor.location)))
            .child(self.control(
                "calendar-editor-allday",
                "All Day",
                "All Day",
                editor.all_day,
                |this, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        editor.all_day = !editor.all_day;
                        // The two Alert lists mean different things at the
                        // same index (CAL-7): switching All Day resets the
                        // selection instead of silently reinterpreting it.
                        editor.alert = 0;
                    }
                    cx.notify();
                },
                cx,
            ))
            .child(labelled_field("Starts", TextField::new(&editor.start)))
            .child(labelled_field("Ends", TextField::new(&editor.end)));

        let mut repeat_row = div().flex().flex_wrap().gap(px(6.0));
        for (index, label) in ["None", "Daily", "Weekly", "Monthly"]
            .into_iter()
            .enumerate()
        {
            repeat_row = repeat_row.child(self.control(
                format!("calendar-editor-repeat-{index}"),
                label,
                label,
                editor.repeat == index,
                move |this, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        editor.repeat = index;
                    }
                    cx.notify();
                },
                cx,
            ));
        }
        card = card.child(labelled_field("Repeat", repeat_row));

        let mut alert_row = div().flex().flex_wrap().gap(px(6.0));
        for (index, offset) in Self::alert_offsets(editor.all_day).iter().enumerate() {
            let label = offset.label();
            alert_row = alert_row.child(self.control(
                format!("calendar-editor-alert-{index}"),
                label,
                label,
                editor.alert == index,
                move |this, cx| {
                    if let Some(editor) = this.editor.as_mut() {
                        editor.alert = index;
                    }
                    cx.notify();
                },
                cx,
            ));
        }
        card = card.child(labelled_field("Alert", alert_row));

        let writable: Vec<(usize, String)> = self
            .snapshot
            .calendars
            .iter()
            .enumerate()
            .filter(|(_, calendar)| calendar.writable && !calendar.removed)
            .map(|(index, calendar)| (index, calendar.name.clone()))
            .collect();
        if writable.len() > 1 {
            let mut calendar_row = div().flex().flex_wrap().gap(px(6.0));
            for (index, name) in writable {
                let active = editor.calendar == index;
                calendar_row = calendar_row.child(
                    div()
                        .id(format!("calendar-editor-calendar-{index}"))
                        .role(Role::Button)
                        .aria_label(name.clone())
                        .h(px(26.0))
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
                        .child(name)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(editor) = this.editor.as_mut() {
                                editor.calendar = index;
                            }
                            cx.notify();
                        })),
                );
            }
            card = card.child(labelled_field("Calendar", calendar_row));
        }

        card = card
            .child(labelled_field("Invitees", TextField::new(&editor.invitees)))
            .child(labelled_field("Notes", TextField::new(&editor.notes)))
            .child(labelled_field("URL", TextField::new(&editor.url)));

        if !is_new && recurring {
            let mut scope_row = div().flex().flex_wrap().gap(px(6.0));
            for (index, label) in ["This Event", "This & Future", "All Events"]
                .into_iter()
                .enumerate()
            {
                scope_row = scope_row.child(self.control(
                    format!("calendar-editor-scope-{index}"),
                    label,
                    label,
                    editor.scope == index,
                    move |this, cx| {
                        if let Some(editor) = this.editor.as_mut() {
                            editor.scope = index;
                        }
                        cx.notify();
                    },
                    cx,
                ));
            }
            card = card.child(labelled_field("Apply to", scope_row));
        }

        let delete_label = if recurring { "Delete…" } else { "Delete" };
        let mut buttons = div().flex().items_center().gap(px(8.0));
        if !is_new {
            buttons =
                buttons.child(
                    dialog_button(
                        "calendar-editor-delete",
                        delete_label,
                        DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.delete_selected(window, cx),
                    )),
                );
        }
        buttons = buttons
            .child(div().flex_1())
            .child(
                dialog_button("calendar-editor-cancel", "Cancel", DialogButtonKind::Normal)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.editor = None;
                        cx.notify();
                    })),
            )
            .child(
                dialog_button("calendar-editor-save", "Save", DialogButtonKind::Primary)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_editor(cx))),
            );
        card = card.child(buttons);

        Some(dialog("calendar-inspector-dialog", card).into_any_element())
    }
}

fn labelled_field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(mac::text_secondary())
                .child(label),
        )
        .child(control)
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
        let origin_x = side;
        let origin_y = TOOLBAR + 44.0;
        let week_start = self.nav.week_start();
        let snapshot = self.snapshot.clone();
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
                View::Day => self.day(
                    &snapshot,
                    content_width,
                    content_height,
                    origin_x,
                    origin_y,
                    cx,
                ),
                View::Week => self.time_grid(
                    &snapshot,
                    content_width,
                    content_height,
                    week_start,
                    7,
                    origin_x,
                    origin_y,
                    cx,
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
            .on_action(cx.listener(|this, _: &NewEvent, window, cx| this.new_event(window, cx)))
            .on_action(cx.listener(|this, _: &SaveEvent, _, cx| this.save_editor(cx)))
            .on_action(
                cx.listener(|this, _: &DeleteEvent, window, cx| this.delete_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &UndoEvent, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &RedoEvent, _, cx| this.redo(cx)))
            .on_action(
                cx.listener(|this, _: &ShowInspector, window, cx| {
                    this.inspect_selected(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &DismissInspector, window, cx| this.dismiss(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Search, window, cx| this.toggle_search(window, cx)))
            .on_action(cx.listener(|this, _: &ShowInvitations, window, cx| {
                this.toggle_invitations(window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewCalendar, _, cx| this.create_new_calendar(cx)))
            .on_action(
                cx.listener(|this, _: &RenameCalendar, window, cx| this.start_rename(window, cx)),
            )
            .on_action(cx.listener(|this, _: &DeleteCalendar, _, cx| this.start_delete(cx)))
            .on_action(
                cx.listener(|this, _: &NewCalendarSubscription, window, cx| {
                    this.start_new_calendar_subscription(window, cx)
                }),
            )
            .on_action(cx.listener(|_, _: &ShowSettings, _, cx| {
                let main = cx.entity();
                cx.defer(move |cx| crate::settings_window::show(main, cx));
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, window, cx| {
                ContextMenuState::dismiss(&mut this.menu_at, window, cx);
            }))
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
        if self.invitations_open {
            root = root.child(self.render_invitations(cx));
        }
        if let Some(menu_at) = self.menu_at.clone() {
            root = root.child(
                self.build_calendar_menu(menu_at.position())
                    .render(&menu_at),
            );
        }
        if let Some(error) = self.error.clone() {
            root = root.child(
                div()
                    .id("calendar-error")
                    .absolute()
                    .left(px(side + 16.0))
                    // UIA-04: this used to sit at `TOOLBAR + 8.0`, inside the
                    // 44 px heading row (`self.heading` below, and
                    // `content_height`'s own `- 44.0`), so the banner was
                    // drawn over "October 2026" instead of under it, the
                    // way the Mac's title stands alone above the grid.
                    .top(px(TOOLBAR + 44.0 + 8.0))
                    .right(px(16.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .bg(mac::system_red().opacity(0.15))
                    .border_1()
                    .border_color(mac::system_red())
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .child(error),
            );
        }
        if let Some(inspector) = self.inspector(cx) {
            root = root.child(inspector);
        }
        if let Some(sheet) = self.render_sheet(cx) {
            root = root.child(sheet);
        }
        root
    }
}
