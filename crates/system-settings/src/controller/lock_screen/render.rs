//! System Settings Lock Screen pane presentation, in the Mac's grammar: the
//! inactivity pop-ups in one group, the pane's actions right-aligned under
//! it (design-lab/settings.html, "Lock Screen").

use super::*;

const LOCK_TIMEOUTS: [(Option<u32>, &str); 5] = [
    (None, "Never"),
    (Some(60), "After 1 minute"),
    (Some(300), "After 5 minutes"),
    (Some(900), "After 15 minutes"),
    (Some(3_600), "After 1 hour"),
];

const SUSPEND_TIMEOUTS: [(Option<u32>, &str); 5] = [
    (None, "Never"),
    (Some(15 * 60), "After 15 minutes"),
    (Some(30 * 60), "After 30 minutes"),
    (Some(60 * 60), "After 1 hour"),
    (Some(3 * 60 * 60), "After 3 hours"),
];

/// The Mac's "Turn display off … when inactive" choices.
const DISPLAY_OFF_TIMEOUTS: [(Option<u32>, &str); 10] = [
    (None, "Never"),
    (Some(60), "For 1 minute"),
    (Some(2 * 60), "For 2 minutes"),
    (Some(5 * 60), "For 5 minutes"),
    (Some(10 * 60), "For 10 minutes"),
    (Some(20 * 60), "For 20 minutes"),
    (Some(30 * 60), "For 30 minutes"),
    (Some(60 * 60), "For 1 hour"),
    (Some(2 * 60 * 60), "For 2 hours"),
    (Some(3 * 60 * 60), "For 3 hours"),
];

/// The Mac's "Require password after … display is turned off" choices.
const REQUIRE_PASSWORD_DELAYS: [(Option<u32>, &str); 8] = [
    (Some(0), "Immediately"),
    (Some(5), "After 5 seconds"),
    (Some(60), "After 1 minute"),
    (Some(5 * 60), "After 5 minutes"),
    (Some(15 * 60), "After 15 minutes"),
    (Some(60 * 60), "After 1 hour"),
    (Some(8 * 60 * 60), "After 8 hours"),
    (None, "Never"),
];

impl Settings {
    fn require_password_row(
        &self,
        view: &Entity<Self>,
        current: Option<u32>,
        enabled: bool,
    ) -> AnyElement {
        let choices: Vec<PopupChoice> = REQUIRE_PASSWORD_DELAYS
            .into_iter()
            .map(|(delay, title)| {
                let option_view = view.clone();
                let selected = current == delay;
                choice(title, selected, move |_, cx| {
                    if !selected {
                        option_view
                            .update(cx, |settings, cx| settings.set_require_password(delay, cx));
                    }
                })
            })
            .collect();
        let fallback = current
            .map(u64::from)
            .map(format_power_duration)
            .unwrap_or_else(|| "Never".into());
        let value = popup_value(&choices, &fallback);
        popup_row(
            "require-password",
            "Require password after display is turned off",
            None,
            value,
            choices,
            enabled,
        )
    }

    fn display_off_row(
        &self,
        view: &Entity<Self>,
        on_battery: bool,
        current: Option<u32>,
        enabled: bool,
    ) -> AnyElement {
        let choices: Vec<PopupChoice> = DISPLAY_OFF_TIMEOUTS
            .into_iter()
            .map(|(timeout, title)| {
                let option_view = view.clone();
                let selected = current == timeout;
                choice(title, selected, move |_, cx| {
                    if !selected {
                        option_view.update(cx, |settings, cx| {
                            settings.set_display_off(on_battery, timeout, cx)
                        });
                    }
                })
            })
            .collect();
        let fallback = current
            .map(u64::from)
            .map(format_power_duration)
            .unwrap_or_else(|| "Never".into());
        let value = popup_value(&choices, &fallback);
        // The Mac warns under the row when the display never turns off.
        let warning: Option<SharedString> = current.is_none().then(|| {
            if on_battery {
                "⚠ Energy usage may be higher when this computer is inactive for longer periods."
                    .into()
            } else {
                "⚠ Never letting your display turn off may shorten its lifespan and increase energy usage."
                    .into()
            }
        });
        let (id, title) = if on_battery {
            (
                "display-off-battery",
                "Turn display off on battery when inactive",
            )
        } else {
            (
                "display-off-adapter",
                "Turn display off on power adapter when inactive",
            )
        };
        popup_row(id, title, warning, value, choices, enabled)
    }

    pub(in crate::controller) fn render_lock_screen(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if self.lock_policy_loading || self.lock_policy_busy {
            cards.push(div().mb_3().child(Progress::indeterminate().label(
                if self.lock_policy_busy {
                    "Applying Lock Screen timeout…"
                } else {
                    "Loading Lock Screen…"
                },
            )));
        }
        for error in [
            &self.lock_policy_error,
            &self.lock_policy_stream_error,
            &self.lock_request_error,
        ]
        .into_iter()
        .flatten()
        {
            cards.push(note_card(error.clone()));
        }
        if let Some(policy) = self.lock_policy {
            let enabled = !self.lock_policy_busy;
            let lock_choices: Vec<PopupChoice> = LOCK_TIMEOUTS
                .into_iter()
                .map(|(timeout, title)| {
                    let option_view = view.clone();
                    let selected = policy.lock_after_seconds == timeout;
                    choice(title, selected, move |_, cx| {
                        if !selected {
                            option_view
                                .update(cx, |settings, cx| settings.set_lock_after(timeout, cx));
                        }
                    })
                })
                .collect();
            let lock_fallback = policy
                .lock_after_seconds
                .map(u64::from)
                .map(format_power_duration)
                .unwrap_or_else(|| "Never".into());
            let lock_current = popup_value(&lock_choices, &lock_fallback);

            let authorized = policy.suspend_capability
                == rmac_shortcuts::lock_settings::SuspendCapability::Authorized;
            let suspend_choices: Vec<PopupChoice> = SUSPEND_TIMEOUTS
                .into_iter()
                .filter(|(timeout, _)| timeout.is_none() || authorized)
                .map(|(timeout, title)| {
                    let option_view = view.clone();
                    let selected = policy.suspend_after_seconds == timeout;
                    choice(title, selected, move |_, cx| {
                        if !selected {
                            option_view
                                .update(cx, |settings, cx| settings.set_suspend_after(timeout, cx));
                        }
                    })
                })
                .collect();
            let suspend_fallback = policy
                .suspend_after_seconds
                .map(u64::from)
                .map(format_power_duration)
                .unwrap_or_else(|| "Never".into());
            let suspend_current = popup_value(&suspend_choices, &suspend_fallback);
            // The Mac puts its warnings under the row title ("⚠ Energy usage
            // may be higher…"); the suspend authority's limits go there too.
            let suspend_warning: Option<SharedString> = match policy.suspend_capability {
                rmac_shortcuts::lock_settings::SuspendCapability::Authorized => None,
                rmac_shortcuts::lock_settings::SuspendCapability::RequiresAuthentication => Some(
                    "⚠ Needs administrator authorisation on this computer. You can still sleep it manually."
                        .into(),
                ),
                rmac_shortcuts::lock_settings::SuspendCapability::Denied => {
                    Some("⚠ Turned off by this computer’s authorisation policy.".into())
                }
                rmac_shortcuts::lock_settings::SuspendCapability::Unavailable => {
                    Some("⚠ Not supported by this computer’s system service.".into())
                }
            };
            let mut rows = Vec::new();
            if policy.display_off_supported {
                if self.hardware.has_battery {
                    rows.push(self.display_off_row(
                        &view,
                        true,
                        policy.display_off_on_battery_seconds,
                        enabled,
                    ));
                }
                rows.push(self.display_off_row(
                    &view,
                    false,
                    policy.display_off_on_adapter_seconds,
                    enabled,
                ));
                if policy.options_supported {
                    rows.push(self.require_password_row(
                        &view,
                        policy.require_password_after_seconds,
                        enabled,
                    ));
                }
            }
            if !rows.is_empty() {
                cards.push(card(rows));
            }
            cards.push(card(vec![
                popup_row(
                    "lock-after",
                    "Lock screen when inactive",
                    None,
                    lock_current,
                    lock_choices,
                    enabled,
                ),
                popup_row(
                    "suspend-after",
                    "Put the computer to sleep when inactive",
                    suspend_warning,
                    suspend_current,
                    suspend_choices,
                    enabled,
                ),
            ]));
        }
        if self.hardware.fprintd_supported {
            cards.push(section_header("Fingerprint"));
            cards.push(card(vec![value_button_row(
                "Fingerprint reader",
                None,
                Some("Supported by fprintd".into()),
                None,
            )]));
        }
        let lock_view = view.clone();
        let refresh_view = view.clone();
        cards.push(footer_buttons(vec![
            push_button(
                "lock-screen-now",
                if self.lock_request_busy {
                    "Locking…"
                } else {
                    "Lock Now"
                },
            )
            .disabled(self.lock_request_busy)
            .on_click(move |_, _, cx| {
                lock_view.update(cx, |settings, cx| settings.request_lock_screen(cx));
            })
            .into_any_element(),
            push_button("lock-policy-refresh", "Refresh")
                .disabled(self.lock_policy_loading || self.lock_policy_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_lock_policy(cx));
                })
                .into_any_element(),
        ]));
        self.pane(cards)
    }
}
