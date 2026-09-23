//! System Settings Lock Screen pane presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_lock_screen(&self, cx: &Context<Self>) -> Div {
        const TIMEOUTS: [(Option<u32>, &str, &str); 5] = [
            (
                None,
                "Never",
                "Keep the session unlocked until a manual or system lock",
            ),
            (
                Some(60),
                "After 1 Minute",
                "Lock after one minute without input",
            ),
            (Some(300), "After 5 Minutes", "Recommended default"),
            (
                Some(900),
                "After 15 Minutes",
                "Lock after fifteen minutes without input",
            ),
            (
                Some(3_600),
                "After 1 Hour",
                "Lock after one hour without input",
            ),
        ];

        let view = cx.entity();
        let refresh_view = view.clone();
        let mut cards = vec![div().flex().justify_end().mb_2().child(
            Button::new("lock-policy-refresh", "Refresh")
                .ghost()
                .disabled(self.lock_policy_loading || self.lock_policy_busy)
                .busy(self.lock_policy_loading || self.lock_policy_busy)
                .on_click(move |_, _, cx| {
                    refresh_view.update(cx, |settings, cx| settings.refresh_lock_policy(cx));
                }),
        )];
        if self.lock_policy_loading || self.lock_policy_busy {
            cards.push(div().mb_3().child(Progress::indeterminate().label(
                if self.lock_policy_busy {
                    "Applying Lock Screen timeout…"
                } else {
                    "Loading Lock Screen…"
                },
            )));
        }
        if let Some(error) = &self.lock_policy_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.lock_policy_stream_error {
            cards.push(note_card(error.clone()));
        }
        if let Some(error) = &self.lock_request_error {
            cards.push(note_card(error.clone()));
        }
        let lock_view = view.clone();
        cards.push(card(vec![row_base()
            .child(tile("icons/lock.svg", accent(), style::ROW_ICON))
            .child(text_block(
                "Test Lock Screen".into(),
                Some(
                    "Securely hide this session now, then authenticate to return to Settings"
                        .into(),
                ),
            ))
            .child(
                Button::new(
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
                }),
            )
            .into_any_element()]));
        if let Some(policy) = self.lock_policy {
            cards.push(section_header("Lock After Inactivity"));
            let mut timeout_rows = TIMEOUTS
                .into_iter()
                .map(|(timeout, title, detail)| {
                    let selected = policy.lock_after_seconds == timeout;
                    let option_view = view.clone();
                    lock_policy_choice_row(
                        SharedString::from(format!("lock-timeout-{}", timeout.unwrap_or(0))),
                        "icons/lock.svg",
                        title,
                        detail,
                        selected,
                        self.lock_policy_busy,
                    )
                    .on_activate(move |_, _, cx| {
                        if !selected {
                            option_view
                                .update(cx, |settings, cx| settings.set_lock_after(timeout, cx));
                        }
                    })
                    .into_any_element()
                })
                .collect::<Vec<_>>();
            if !TIMEOUTS
                .iter()
                .any(|(timeout, _, _)| *timeout == policy.lock_after_seconds)
            {
                let value = policy
                    .lock_after_seconds
                    .map(u64::from)
                    .map(format_power_duration)
                    .unwrap_or_else(|| "Never".into());
                timeout_rows.insert(
                    0,
                    value_row(
                        "icons/history.svg",
                        secondary(),
                        "Current Custom Timeout".into(),
                        value.into(),
                    ),
                );
            }
            cards.push(card(timeout_rows));
            cards.push(section_header("Automatic Suspend"));
            let suspend_authorized = policy.suspend_capability
                == rmac_shortcuts::lock_settings::SuspendCapability::Authorized;
            let suspend_options = [
                (None, "Never", "Do not suspend automatically"),
                (
                    Some(15 * 60),
                    "After 15 Minutes",
                    "Suspend after fifteen minutes without input",
                ),
                (
                    Some(30 * 60),
                    "After 30 Minutes",
                    "Suspend after thirty minutes without input",
                ),
                (
                    Some(60 * 60),
                    "After 1 Hour",
                    "Suspend after one hour without input",
                ),
                (
                    Some(3 * 60 * 60),
                    "After 3 Hours",
                    "Suspend after three hours without input",
                ),
            ];
            let mut suspend_rows = suspend_options
                .into_iter()
                .filter(|(timeout, _, _)| timeout.is_none() || suspend_authorized)
                .map(|(timeout, title, detail)| {
                    let selected = policy.suspend_after_seconds == timeout;
                    let option_view = view.clone();
                    lock_policy_choice_row(
                        SharedString::from(format!("suspend-timeout-{}", timeout.unwrap_or(0))),
                        "icons/power.svg",
                        title,
                        detail,
                        selected,
                        self.lock_policy_busy,
                    )
                    .on_activate(move |_, _, cx| {
                        if !selected {
                            option_view
                                .update(cx, |settings, cx| settings.set_suspend_after(timeout, cx));
                        }
                    })
                    .into_any_element()
                })
                .collect::<Vec<_>>();
            if let Some(seconds) = policy.suspend_after_seconds {
                let is_visible_choice = suspend_authorized
                    && suspend_options
                        .iter()
                        .any(|(timeout, _, _)| *timeout == Some(seconds));
                if !is_visible_choice {
                    suspend_rows.insert(
                        0,
                        value_row(
                            "icons/history.svg",
                            secondary(),
                            "Current Suspend Timeout".into(),
                            format_power_duration(u64::from(seconds)).into(),
                        ),
                    );
                }
            }
            cards.push(card(suspend_rows));
            match policy.suspend_capability {
                rmac_shortcuts::lock_settings::SuspendCapability::Authorized => {
                    cards.push(note_card(
                        "Automatic suspend uses the system login manager, respects active inhibitors, and always passes through the pre-sleep lock boundary.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::RequiresAuthentication => {
                    cards.push(note_card(
                        "Automatic suspend is unavailable because this computer requires interactive authorization. You can still suspend manually and approve the system prompt.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::Denied => {
                    cards.push(note_card(
                        "Automatic suspend is disabled by this computer’s authorization policy.",
                    ));
                }
                rmac_shortcuts::lock_settings::SuspendCapability::Unavailable => {
                    cards.push(note_card(
                        "Automatic suspend is not supported by this computer or its current system service.",
                    ));
                }
            }
            cards.push(section_header("Security"));
            cards.push(card(vec![
                value_row(
                    "icons/shield.svg",
                    hsl(0x34c759),
                    "Before Sleep".into(),
                    "Always Lock".into(),
                ),
                value_row(
                    "icons/key.svg",
                    secondary(),
                    "Authentication".into(),
                    "Password Required".into(),
                ),
                value_row(
                    "icons/bell.svg",
                    secondary(),
                    "Notification Previews".into(),
                    "Hidden".into(),
                ),
            ]));
            cards.push(note_card(
                "The current PAM-enabled swaylock provider cannot render notification content, so previews stay hidden even when an application policy would allow them. Lid close and suspend follow the system’s supported logind policy and always pass through the pre-sleep lock boundary.",
            ));
        }
        self.pane(cards)
    }
}
