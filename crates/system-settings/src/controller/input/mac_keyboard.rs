//! Keyboard › PC Keyboard: Mac shortcuts in all apps and where ⌘, ⌥ and
//! Caps Lock sit on a PC keyboard (design-lab/settings.html, Keyboard;
//! docs/decisions/0017-mac-keyboard.md). Every change goes to keyd or
//! localed through `rmac_keyboard::apply`; nothing here is local state.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_mac_keyboard_update(
        &mut self,
        result: std::result::Result<rmac_keyboard::Status, rmac_keyboard::Error>,
    ) {
        self.mac_keyboard_busy = false;
        match result {
            Ok(status) => {
                self.mac_keyboard = Some(status);
                self.mac_keyboard_error = None;
            }
            Err(error) => {
                self.mac_keyboard_error =
                    Some(format!("Could not update the PC keyboard settings: {error}").into());
            }
        }
    }

    pub(in crate::controller) fn refresh_mac_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.mac_keyboard_busy {
            return;
        }
        self.mac_keyboard_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_keyboard::status() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_mac_keyboard_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_mac_keyboard(
        &mut self,
        change: impl FnOnce(&mut rmac_keyboard::MacKeyboard),
        cx: &mut Context<Self>,
    ) {
        let Some(status) = &self.mac_keyboard else {
            return;
        };
        if self.mac_keyboard_busy {
            return;
        }
        let mut target = status.state;
        change(&mut target);
        if target == status.state {
            return;
        }
        self.mac_keyboard_busy = true;
        self.mac_keyboard_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_keyboard::apply(&target) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_mac_keyboard_update(result);
                // localed's layout and options changed too.
                this.refresh_locale(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The PC Keyboard section: a header, one group and its footnotes.
    pub(in crate::controller) fn mac_keyboard_cards(&self, cx: &Context<Self>) -> Vec<Div> {
        let mut cards = vec![section_header("PC Keyboard")];
        let Some(status) = &self.mac_keyboard else {
            cards.push(note_card(match &self.mac_keyboard_error {
                Some(error) => error.clone(),
                None => "Loading keyboard settings…".into(),
            }));
            return cards;
        };
        let view = cx.entity();
        let state = status.state;
        let idle = !self.mac_keyboard_busy;
        let can_use_keyd = status.keyd_installed
            && status.helper_installed
            && status.foreign_keyd_configs.is_empty();

        let shortcuts_view = view.clone();
        let swap_view = view.clone();
        let option_view = view.clone();
        let caps_choices = rmac_keyboard::CapsLockAction::ALL
            .into_iter()
            .map(|action| {
                let view = view.clone();
                choice(
                    action.label(),
                    state.layout.caps_lock == action,
                    move |_, cx| {
                        view.update(cx, |settings, cx| {
                            settings
                                .apply_mac_keyboard(|target| target.layout.caps_lock = action, cx);
                        });
                    },
                )
            })
            .collect::<Vec<_>>();
        let caps_value = popup_value(&caps_choices, state.layout.caps_lock.label());

        cards.push(card(vec![
            switch_row(
                "mac-keyboard-shortcuts",
                "Use Mac shortcuts in all apps",
                Some(
                    "⌘C, ⌘V and the other ⌘ shortcuts work in apps made for PC keyboards. \
                     Terminals keep ⌃C for interrupting."
                        .into(),
                ),
                state.shortcuts_in_all_apps,
                idle && (can_use_keyd || state.shortcuts_in_all_apps),
                move |on, _, cx| {
                    shortcuts_view.update(cx, |settings, cx| {
                        settings.apply_mac_keyboard(|target| target.shortcuts_in_all_apps = on, cx);
                    });
                },
            ),
            switch_row(
                "mac-keyboard-swap",
                "⌘ Command next to the space bar",
                Some("Alt works as ⌘ Command and the Windows key as ⌥ Option".into()),
                state.layout.swap_command_option,
                idle,
                move |on, _, cx| {
                    swap_view.update(cx, |settings, cx| {
                        settings.apply_mac_keyboard(
                            |target| target.layout.swap_command_option = on,
                            cx,
                        );
                    });
                },
            ),
            popup_row(
                "mac-keyboard-caps",
                "Caps Lock key",
                None,
                caps_value,
                caps_choices,
                idle,
            ),
            switch_row(
                "mac-keyboard-option",
                "⌥ Option types special characters",
                Some("Uses the Mac version of your layout: ⌥E then E types é".into()),
                state.layout.option_characters,
                idle && (status.option_characters_available || state.layout.option_characters),
                move |on, _, cx| {
                    option_view.update(cx, |settings, cx| {
                        settings
                            .apply_mac_keyboard(|target| target.layout.option_characters = on, cx);
                    });
                },
            ),
        ]));

        if let Some(error) = &self.mac_keyboard_error {
            cards.push(note_card(error.clone()));
        }
        if state.shortcuts_in_all_apps && !status.session_can_bind {
            cards.push(footnote(
                "Log out and log back in to finish turning on Mac shortcuts in all apps.",
            ));
        } else if !status.keyd_installed && !state.shortcuts_in_all_apps {
            cards.push(footnote(
                "Mac shortcuts in all apps need the keyd package. Install keyd, then return here.",
            ));
        } else if !status.foreign_keyd_configs.is_empty() && !state.shortcuts_in_all_apps {
            cards.push(footnote(format!(
                "keyd already has another configuration ({}), so Lulo OS leaves it alone.",
                status.foreign_keyd_configs.join(", ")
            )));
        } else if !status.helper_installed && !state.shortcuts_in_all_apps {
            cards.push(footnote(
                "Mac shortcuts in all apps are available when Lulo OS is installed from its package.",
            ));
        }
        if !status.option_characters_available && !state.layout.option_characters {
            cards.push(footnote(
                "Your keyboard layout has no Mac version, so ⌥ Option cannot type special characters.",
            ));
        }
        cards
    }
}
