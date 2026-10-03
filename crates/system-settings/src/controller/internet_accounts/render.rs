use super::*;

fn provider_monogram(mark: &'static str) -> AnyElement {
    div()
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(rmac_ui::mac::radius_control()))
        .bg(rmac_ui::mac::control_fill())
        .text_size(rmac_ui::text_px(13.0))
        .font_weight(rmac_ui::mac::BOLD)
        .text_color(label())
        .child(mark)
        .into_any_element()
}

fn choice_mark(choice: Choice) -> &'static str {
    match choice {
        Choice::ICloud => "i",
        Choice::Microsoft => "M",
        Choice::Google => "G",
        Choice::Yahoo => "Y",
        Choice::OtherMail => "@",
        Choice::OtherCalendar => "C",
    }
}

fn account_mark(label: &str) -> &'static str {
    match label {
        "iCloud" => "i",
        "Microsoft" => "M",
        "Google" => "G",
        "Yahoo" => "Y",
        "Calendar Account" => "C",
        _ => "@",
    }
}

fn field(
    label_text: &'static str,
    editor: &Entity<InputState>,
    busy: bool,
    cx: &Context<Settings>,
) -> impl IntoElement {
    let field = div()
        .id(format!("internet-account-field-{label_text}"))
        .role(Role::TextInput)
        .aria_label(label_text)
        .v_flex()
        .gap_1()
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(secondary())
                .child(label_text),
        )
        .child(TextField::new(editor).disabled(busy).w_full());
    if label_text == "Password" {
        field
    } else {
        field.accessible_text_input(editor, cx)
    }
}

impl Settings {
    pub(in crate::controller) fn render_internet_accounts(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards: Vec<Div> = Vec::new();
        let account_rows = rmac_accounts_ui::account_rows(&self.internet_accounts);
        if let Some(path) = &self.internet_account_selected {
            if let Some(account) = account_rows
                .iter()
                .find(|account| account.paths.contains(path))
            {
                let account = account.clone();
                let mut rows = Vec::new();
                rows.push(value_row(
                    "icons/globe.svg",
                    accent(),
                    "Account".into(),
                    account.identity.clone().into(),
                ));
                for (service, title) in [
                    (Service::Mail, "Mail"),
                    (Service::Calendar, "Calendars"),
                    (Service::Contacts, "Contacts"),
                ] {
                    let Some(path) = account.service_path(service).map(str::to_owned) else {
                        continue;
                    };
                    let view = view.clone();
                    rows.push(switch_row(
                        format!("account-{}-{title}", account.paths[0]),
                        title,
                        None,
                        account.services.enabled(service),
                        !self.internet_accounts_busy,
                        move |enabled, _, cx| {
                            view.update(cx, |settings, cx| {
                                settings.set_account_service(path.clone(), service, enabled, cx)
                            });
                        },
                    ));
                }
                cards.push(card(rows));
                let back = view.clone();
                let delete = view.clone();
                cards.push(footer_buttons(vec![
                    push_button("internet-account-back", "All Accounts")
                        .on_click(move |_, _, cx| {
                            back.update(cx, |settings, cx| {
                                settings.internet_account_selected = None;
                                cx.notify();
                            })
                        })
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "internet-account-delete",
                        "Delete Account…",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(self.internet_accounts_busy)
                    .on_click(move |_, _, cx| {
                        delete.update(cx, |settings, cx| {
                            settings.internet_account_delete = true;
                            cx.notify();
                        })
                    })
                    .into_any_element(),
                ]));
            }
        } else if self.internet_accounts.is_empty() {
            cards.push(if self.internet_accounts_loading {
                note_card("Loading internet accounts…")
            } else if self.internet_accounts_error.is_some() {
                note_card("Internet Accounts is unavailable. Try refreshing when the account service is running.")
            } else {
                note_card("No internet accounts are set up yet.")
            });
        } else {
            let mut rows = Vec::new();
            for account in &account_rows {
                let path = account.paths[0].clone();
                let view = view.clone();
                rows.push(large_nav_row(
                    format!("internet-account-{}", account.paths[0]),
                    provider_monogram(account_mark(account.label)),
                    account.label,
                    Some(subtitle_text(account.identity.clone())),
                    Some(account.summary().into()),
                    move |_, cx| {
                        view.update(cx, |settings, cx| {
                            settings.internet_account_selected = Some(path.clone());
                            cx.notify();
                        })
                    },
                ));
            }
            cards.push(card(rows));
        }
        let add = view.clone();
        let refresh = view.clone();
        cards.push(footer_buttons(vec![
            push_button("internet-accounts-refresh", "Refresh")
                .disabled(self.internet_accounts_loading)
                .on_click(move |_, _, cx| {
                    refresh.update(cx, |settings, cx| settings.refresh_internet_accounts(cx))
                })
                .into_any_element(),
            push_button("internet-accounts-add", "Add Account…")
                .on_click(move |_, window, cx| {
                    add.update(cx, |settings, cx| settings.open_account_sheet(window, cx))
                })
                .into_any_element(),
        ]));
        if let Some(error) = &self.internet_accounts_error {
            cards.push(note_card(error.clone()));
        }
        self.pane(cards)
    }

    pub(in crate::controller) fn render_internet_account_delete(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.internet_account_delete.then(|| {
            rmac_ui::alert(
                "Delete this account?",
                "This removes the account from Mail, Calendar and other apps on this computer.",
                vec![
                    rmac_ui::dialog_button(
                        "account-delete-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.internet_account_delete = false;
                        cx.notify();
                    }))
                    .into_any_element(),
                    rmac_ui::dialog_button(
                        "account-delete-confirm",
                        "Delete Account",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.remove_account(cx)))
                    .into_any_element(),
                ],
            )
            .into_any_element()
        })
    }

    pub(in crate::controller) fn render_internet_account_sheet(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let sheet = self.internet_account_sheet.as_ref()?;
        let view = cx.entity();
        let busy = self.internet_accounts_busy;
        let mut body = div()
            .w(px(460.0))
            .v_flex()
            .gap_3()
            .p_5()
            .rounded(px(rmac_ui::mac::radius_large_surface()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(match sheet.model.step {
                        Step::Choose => "Add an Internet Account".to_owned(),
                        Step::Discovering => "Finding account settings".to_owned(),
                        Step::Credentials => format!(
                            "Sign in to {}",
                            sheet.model.choice.map(Choice::label).unwrap_or("Account")
                        ),
                        Step::Browser => format!(
                            "Sign in to {}",
                            sheet.model.choice.map(Choice::label).unwrap_or("Account")
                        ),
                        Step::Services => "Use this account with:".to_owned(),
                    }),
            );
        body = match sheet.model.step {
            Step::Choose => {
                let mut body = body
                    .child(div().text_size(rmac_ui::text_px(12.0)).text_color(secondary()).child("Enter your email address, or choose a provider."))
                    .child(field("Email address", &sheet.address, busy, cx));
                for choice in Choice::ALL {
                    let selected = sheet.model.choice == Some(choice);
                    let view = view.clone();
                    body = body.child(
                        ListRow::new(
                            format!("account-provider-{choice:?}"),
                            div().flex().items_center().gap_2()
                                .child(provider_monogram(choice_mark(choice)))
                                .child(div().flex_1().text_color(label()).child(choice.label()))
                                .child(div().text_size(rmac_ui::text_px(11.0)).text_color(secondary()).child(choice.hint())),
                        )
                        .aria_label(format!("{} · {}", choice.label(), choice.hint()))
                        .selected(selected)
                        .h(px(40.0))
                        .on_activate(move |_, _, cx| view.update(cx, |settings, cx| {
                            if let Some(sheet) = settings.internet_account_sheet.as_mut() {
                                sheet.model.choice = Some(choice);
                                sheet.model.error = None;
                                cx.notify();
                            }
                        })),
                    );
                }
                body
            }
            Step::Discovering => body.child(Progress::indeterminate().label("Looking up mail servers…")),
            Step::Credentials => {
                let mut body = body;
                if matches!(sheet.model.choice, Some(Choice::ICloud | Choice::Yahoo)) {
                    body = body.child(div().text_size(rmac_ui::text_px(12.0)).text_color(secondary()).child("Use an app-specific password from your account provider."));
                    if let Some(choice) = sheet.model.choice {
                        let help = view.clone();
                        body = body.child(push_button("account-password-help", "Get an App-Specific Password…")
                            .on_click(move |_, _, cx| help.update(cx, |settings, cx| settings.open_account_password_help(choice, cx))));
                    }
                }
                body = body.child(field("Name", &sheet.name, busy, cx))
                    .child(field("Email address", &sheet.address, busy, cx))
                    .child(field("Password", &sheet.password, busy, cx));
                if sheet.model.choice == Some(Choice::OtherCalendar) {
                    body = body.child(field("CalDAV URL", &sheet.caldav, busy, cx));
                } else if sheet.manual {
                    body = body.child(field("IMAP server", &sheet.imap, busy, cx))
                        .child(field("SMTP server", &sheet.smtp, busy, cx));
                }
                body
            }
            Step::Browser => body.child(div().text_size(rmac_ui::text_px(12.0)).text_color(secondary())
                .child("Finish signing in in your web browser. Lulo will continue when the provider sends you back here."))
                .child(Progress::indeterminate().label("Waiting for your browser…")),
            Step::Services => {
                let mut body = body.child(div().text_size(rmac_ui::text_px(12.0)).text_color(secondary()).child(sheet.model.address.clone()));
                for (service, title) in [(Service::Mail, "Mail"), (Service::Calendar, "Calendars"), (Service::Contacts, "Contacts")] {
                    if sheet.model.choice == Some(Choice::OtherCalendar) && service != Service::Calendar { continue; }
                    if sheet.model.choice == Some(Choice::OtherMail) && service != Service::Mail { continue; }
                    let view = view.clone();
                    body = body.child(switch_row(
                        format!("account-new-{title}"), title, None,
                        sheet.model.services.enabled(service), !busy && !(service == Service::Contacts && matches!(sheet.model.choice, Some(Choice::ICloud | Choice::Yahoo))),
                        move |enabled, _, cx| view.update(cx, |settings, cx| {
                            if let Some(sheet) = settings.internet_account_sheet.as_mut() {
                                sheet.model.services.set(service, enabled);
                                cx.notify();
                            }
                        }),
                    ));
                }
                body
            }
        };
        if let Some(error) = sheet.model.error {
            body = body.child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::danger())
                    .child(error),
            );
        }
        let cancel = view.clone();
        let action = view.clone();
        let label = match sheet.model.step {
            Step::Choose => "Continue",
            Step::Discovering => "",
            Step::Credentials => "Sign In",
            Step::Browser => "",
            Step::Services => "Done",
        };
        body = body.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    rmac_ui::dialog_button(
                        "internet-account-cancel",
                        "Cancel",
                        rmac_ui::DialogButtonKind::Normal,
                    )
                    .disabled(busy && sheet.model.step == Step::Services)
                    .on_click(move |_, _, cx| {
                        cancel.update(cx, |settings, cx| settings.close_account_sheet(cx))
                    }),
                )
                .when(
                    !matches!(sheet.model.step, Step::Browser | Step::Discovering),
                    |buttons| {
                        buttons.child(
                            rmac_ui::dialog_button(
                                "internet-account-next",
                                label,
                                rmac_ui::DialogButtonKind::Primary,
                            )
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                action.update(cx, |settings, cx| {
                                    match settings
                                        .internet_account_sheet
                                        .as_ref()
                                        .map(|sheet| sheet.model.step)
                                    {
                                        Some(Step::Choose) => settings.continue_account_sheet(cx),
                                        Some(Step::Credentials) => settings.credentials_next(cx),
                                        Some(Step::Services) => settings.save_account(cx),
                                        _ => {}
                                    }
                                })
                            }),
                        )
                    },
                ),
        );
        Some(
            rmac_ui::dialog("internet-account-sheet", body)
                .attached()
                .aria_label("Add an Internet Account")
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.close_account_sheet(cx);
                        }
                        "enter"
                            if !this.internet_accounts_busy
                                && this.internet_account_sheet.as_ref().is_some_and(|sheet| {
                                    matches!(sheet.model.step, Step::Credentials | Step::Services)
                                }) =>
                        {
                            cx.stop_propagation();
                            match this
                                .internet_account_sheet
                                .as_ref()
                                .map(|sheet| sheet.model.step)
                            {
                                Some(Step::Credentials) => this.credentials_next(cx),
                                Some(Step::Services) => this.save_account(cx),
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
