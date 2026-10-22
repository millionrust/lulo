//! Message ▸ Remind Me: Remind Me in 1 Hour, Tonight, Tomorrow, Later… and
//! Clear Reminder. The schedule itself is `crate::scheduler`; when a
//! reminder falls due the message comes back unread at the top of the list.

use gpui::{
    div, px, AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::schedule::{format_when, parse_when, Preset, Reminder};
use rmac_ui::{mac, AccessibleTextInput as _, DialogButtonKind, InputEvent, TextField};

use super::MailView;
use crate::{
    ClearReminder, RemindInOneHour, RemindLater, RemindSheetCancel, RemindSheetConfirm,
    RemindTomorrow, RemindTonight,
};

impl MailView {
    fn selected_id(&self) -> Option<String> {
        self.state
            .selected_message()
            .map(|message| message.id.clone())
    }

    fn remind_at(&mut self, at: i64, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id() else {
            return;
        };
        crate::scheduler::change(cx, move |schedule| schedule.remind(&id, at));
    }

    fn remind(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.remind_at(preset.at(crate::scheduler::now()), cx);
    }

    fn clear_reminder(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_id() {
            crate::scheduler::change(cx, move |schedule| {
                schedule.clear_reminder(&id);
            });
        }
    }

    /// Remind Me Later…: a date and time, starting at 8:00 tomorrow.
    fn open_remind_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_id().is_none() {
            return;
        }
        let start = format_when(Preset::TomorrowMorning.at(crate::scheduler::now()));
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("23 Oct 2026 08:00")
                .default_value(start)
        });
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_remind_sheet(window, cx);
            }
        })
        .detach();
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            let end = state.value().len();
            state.set_selected_range(0..end, cx);
        });
        self.remind_sheet = Some(input);
        cx.notify();
    }

    fn close_remind_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remind_sheet = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn confirm_remind_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.remind_sheet.clone() else {
            return;
        };
        let Some(at) = parse_when(&input.read(cx).value()) else {
            // An unreadable date keeps the sheet open, as a Mac date
            // picker never accepts one.
            return;
        };
        self.close_remind_sheet(window, cx);
        self.remind_at(at, cx);
    }

    /// Called by the scheduler for the reminders that just fell due.
    pub fn reminders_due(&mut self, due: &[Reminder], cx: &mut Context<Self>) {
        let mut changed = false;
        for reminder in due {
            changed |= self.state.bring_back(&reminder.message_id);
        }
        if changed {
            self.dispatch_persist(cx);
            self.sync_menu(cx);
            cx.notify();
        } else if self.active {
            // A reminder set or cleared changes Clear Reminder.
            self.sync_remind_menu(cx);
        }
    }

    /// Message ▸ Remind Me's live state.
    pub(super) fn sync_remind_menu(&self, cx: &mut Context<Self>) {
        let selected = self.selected_id();
        for action in [
            "mail::RemindInOneHour",
            "mail::RemindTonight",
            "mail::RemindTomorrow",
            "mail::RemindLater",
        ] {
            rmac_ui::set_menu_enabled(action, selected.is_some(), cx);
        }
        let has = selected
            .as_deref()
            .is_some_and(|id| crate::scheduler::has_reminder(id, cx));
        rmac_ui::set_menu_enabled("mail::ClearReminder", has, cx);
    }

    pub(super) fn on_remind_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        root.on_action(
            cx.listener(|this, _: &RemindInOneHour, _, cx| this.remind(Preset::InOneHour, cx)),
        )
        .on_action(cx.listener(|this, _: &RemindTonight, _, cx| this.remind(Preset::Tonight, cx)))
        .on_action(
            cx.listener(|this, _: &RemindTomorrow, _, cx| this.remind(Preset::TomorrowMorning, cx)),
        )
        .on_action(
            cx.listener(|this, _: &RemindLater, window, cx| this.open_remind_sheet(window, cx)),
        )
        .on_action(cx.listener(|this, _: &ClearReminder, _, cx| this.clear_reminder(cx)))
        .on_action(cx.listener(|this, _: &RemindSheetCancel, window, cx| {
            this.close_remind_sheet(window, cx)
        }))
        .on_action(cx.listener(|this, _: &RemindSheetConfirm, window, cx| {
            this.confirm_remind_sheet(window, cx)
        }))
    }

    pub(super) fn remind_sheet_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.remind_sheet.as_ref()?;
        Some(
            div()
                .id("mail-remind-sheet")
                .key_context("MailRemindSheet")
                .role(Role::Dialog)
                .aria_label("Remind Me Later")
                .absolute()
                .top(px(super::TOOLBAR))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(340.0))
                        .p(px(20.0))
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .rounded(px(mac::radius_card()))
                        .bg(mac::raised())
                        .border_1()
                        .border_color(mac::separator())
                        .shadow_lg()
                        .text_size(px(13.0))
                        .text_color(mac::text())
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Remind me about this message on:"),
                        )
                        .child(
                            div()
                                .id("mail-remind-when")
                                .role(Role::TextInput)
                                .aria_label("Date and time")
                                .accessible_text_input(input, cx)
                                .h(px(24.0))
                                .rounded(px(mac::radius_control()))
                                .bg(mac::control_fill())
                                .child(TextField::new(input).appearance(false).small()),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(8.0))
                                .child(
                                    rmac_ui::dialog_button(
                                        "mail-remind-cancel",
                                        "Cancel",
                                        DialogButtonKind::Normal,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.close_remind_sheet(window, cx),
                                    )),
                                )
                                .child(
                                    rmac_ui::dialog_button(
                                        "mail-remind-ok",
                                        "Remind Me",
                                        DialogButtonKind::Primary,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.confirm_remind_sheet(window, cx),
                                    )),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
