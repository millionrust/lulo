use super::*;

const LIST_PICTURE: f32 = 32.0;
const SHEET_PICTURE: f32 = 64.0;
const FACE_TILE: f32 = 56.0;

fn user_subtitle(user: &User, current_uid: u64) -> Option<SharedString> {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(badge) = user.account_type.badge() {
        parts.push(badge);
    }
    if user.uid == current_uid {
        parts.push("Signed in");
    }
    (!parts.is_empty()).then(|| parts.join(", ").into())
}

impl Settings {
    pub(in crate::controller) fn render_users_groups(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let state = &self.users;
        let mut cards: Vec<Div> = Vec::new();
        if state.list.is_empty() {
            cards.push(if state.loading || !state.loaded {
                note_card("Loading users…")
            } else {
                note_card(
                    state
                        .error
                        .clone()
                        .unwrap_or_else(|| "No users are available.".into()),
                )
            });
        } else {
            let mut rows = Vec::new();
            for user in &state.list {
                let path = user.path.clone();
                let open = view.clone();
                let subtitle = user_subtitle(user, state.current_uid);
                let mut row = large_row(
                    account_picture(user.icon_file.as_ref(), &user.initials(), LIST_PICTURE),
                    user.display_name().to_owned(),
                    subtitle.map(subtitle_text),
                );
                row = row.child(info_button(
                    SharedString::from(format!("user-info-{}", user.uid)),
                    "User details",
                    move |window, cx| {
                        let path = path.clone();
                        open.update(cx, |settings, cx| settings.open_user_info(path, window, cx))
                    },
                ));
                rows.push(
                    row.id(SharedString::from(format!("user-row-{}", user.uid)))
                        .role(Role::Group)
                        .aria_label(SharedString::from(match user.account_type.badge() {
                            Some(badge) => format!("{}, {badge}", user.display_name()),
                            None => user.display_name().to_owned(),
                        }))
                        .into_any_element(),
                );
            }
            cards.push(card(rows));
        }
        let add = view.clone();
        cards.push(footer_buttons(vec![push_button("users-add", "Add User…")
            .disabled(state.busy || state.list.is_empty())
            .on_click(move |_, window, cx| {
                add.update(cx, |settings, cx| settings.open_new_user(window, cx))
            })
            .into_any_element()]));

        // Login Options.
        if !state.list.is_empty() {
            let current_auto = state.list.iter().find(|user| user.automatic_login);
            let off = view.clone();
            let mut choices = vec![choice("Off", current_auto.is_none(), move |_, cx| {
                off.update(cx, |settings, cx| settings.set_automatic_login(None, cx))
            })];
            for user in &state.list {
                let path = user.path.clone();
                let set = view.clone();
                choices.push(choice(
                    user.display_name().to_owned(),
                    user.automatic_login,
                    move |_, cx| {
                        let path = path.clone();
                        set.update(cx, |settings, cx| {
                            settings.set_automatic_login(Some(path), cx)
                        })
                    },
                ));
            }
            let current = popup_value(&choices, "Off");
            cards.push(card(vec![popup_row(
                "users-automatic-login",
                "Automatically log in as",
                None,
                current,
                choices,
                !state.busy,
            )]));
        }
        if let (Some(error), false) = (&state.error, state.list.is_empty()) {
            cards.push(note_card(error.clone()));
        }
        self.pane(cards)
    }

    /// Login Password (the name macOS 26 gives Touch ID & Password on a Mac
    /// without Touch ID): the password row and its Change… button.
    pub(in crate::controller) fn render_login_password(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let state = &self.users;
        let available = state.current().is_some();
        let change = view.clone();
        let mut cards = vec![
            card(vec![value_button_row(
                "Login password",
                Some("The password you use to log in and to unlock the screen.".into()),
                None,
                Some(
                    push_button("login-password-change", "Change…")
                        .disabled(!available || state.busy)
                        .on_click(move |_, window, cx| {
                            change.update(cx, |settings, cx| {
                                settings.open_change_password(window, cx)
                            })
                        })
                        .into_any_element(),
                ),
            )]),
            footnote("Lulo OS has no fingerprint unlock on this computer, so this pane covers the login password only."),
        ];
        if let Some(user) = state.current() {
            if !user.password_hint.is_empty() {
                cards.insert(
                    1,
                    card(vec![fact_row("Password hint", user.password_hint.clone())]),
                );
            }
        } else if state.loaded {
            cards.push(note_card(
                state
                    .error
                    .clone()
                    .unwrap_or_else(|| "Your account couldn’t be read.".into()),
            ));
        }
        self.pane(cards)
    }

    pub(in crate::controller) fn render_users_overlay(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.users;
        let dialog = if let Some(confirm) = &state.delete {
            self.render_delete_user(confirm, cx)
        } else if let Some(sheet) = &state.new_user {
            self.render_new_user(sheet, cx)
        } else if let Some(sheet) = &state.password {
            self.render_change_password(sheet, cx)
        } else if let Some(sheet) = &state.picture {
            self.render_picture_sheet(sheet, cx)
        } else if let Some(sheet) = &state.info {
            self.render_user_info(sheet, cx)?
        } else {
            return None;
        };
        Some(
            dialog
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.close_user_sheets(window, cx);
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_user_info(&self, sheet: &InfoSheet, cx: &Context<Self>) -> Option<rmac_ui::Dialog> {
        let view = cx.entity();
        let state = &self.users;
        let user = state.user(&sheet.path)?;
        let own = user.uid == state.current_uid;
        let busy = state.busy;
        let picture = {
            let path = user.path.clone();
            let open = view.clone();
            div()
                .id("user-info-picture")
                .role(Role::Button)
                .aria_label(if own { "Edit picture" } else { "Picture" })
                .when(own, |picture| {
                    picture.cursor_pointer().on_click(move |_, _, cx| {
                        let path = path.clone();
                        open.update(cx, |settings, cx| settings.open_picture_sheet(path, cx))
                    })
                })
                .child(account_picture(
                    user.icon_file.as_ref(),
                    &user.initials(),
                    SHEET_PICTURE,
                ))
        };
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(picture)
            .child(large_text(
                user.display_name().to_owned(),
                Some(subtitle_text(format!(
                    "{} · {}",
                    user.user_name,
                    user.account_type.label()
                ))),
            ))
            .into_any_element();
        let mut rows = vec![if own {
            sheet_field_row(
                "user-info-full-name",
                "Full name",
                &sheet.full_name,
                false,
                !busy,
                cx,
            )
        } else {
            fact_row("Full name", user.real_name.clone())
        }];
        rows.push(fact_row("Account name", user.user_name.clone()));
        if own {
            let change = view.clone();
            rows.push(value_button_row(
                "Password",
                None,
                None,
                Some(
                    push_button("user-info-change-password", "Change Password…")
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            change.update(cx, |settings, cx| {
                                settings.open_change_password(window, cx)
                            })
                        })
                        .into_any_element(),
                ),
            ));
        }
        let leading = (!own).then(|| {
            let path = user.path.clone();
            let delete = view.clone();
            sheet_button(
                "user-info-delete",
                "Delete User…",
                rmac_ui::DialogButtonKind::Destructive,
                !busy && state.current_is_admin(),
                move |_, cx| {
                    let path = path.clone();
                    delete.update(cx, |settings, cx| settings.request_delete_user(&path, cx))
                },
            )
        });
        let ok = view.clone();
        Some(form_sheet(
            "user-info-sheet",
            user.display_name().to_owned(),
            None,
            Some(header),
            rows,
            sheet.error.clone(),
            leading,
            vec![sheet_button(
                "user-info-ok",
                "OK",
                rmac_ui::DialogButtonKind::Primary,
                !busy,
                move |_, cx| ok.update(cx, |settings, cx| settings.save_user_info(cx)),
            )],
        ))
    }

    fn render_new_user(&self, sheet: &NewUserSheet, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.users.busy;
        let types: Vec<PopupChoice> = [AccountType::Standard, AccountType::Administrator]
            .into_iter()
            .map(|kind| {
                let set = view.clone();
                choice(kind.label(), sheet.account_type == kind, move |_, cx| {
                    set.update(cx, |settings, cx| {
                        if let Some(sheet) = settings.users.new_user.as_mut() {
                            sheet.account_type = kind;
                            cx.notify();
                        }
                    })
                })
            })
            .collect();
        let rows = vec![
            sheet_value_row(
                "New User",
                form_popup(
                    "new-user-type",
                    sheet.account_type.label().into(),
                    types,
                    !busy,
                )
                .into_any_element(),
            ),
            sheet_field_row(
                "new-user-full-name",
                "Full Name",
                &sheet.full_name,
                false,
                !busy,
                cx,
            ),
            sheet_field_row(
                "new-user-account-name",
                "Account Name",
                &sheet.user_name,
                false,
                !busy,
                cx,
            ),
            sheet_field_row(
                "new-user-password",
                "Password",
                &sheet.password,
                true,
                !busy,
                cx,
            ),
            sheet_field_row("new-user-verify", "Verify", &sheet.verify, true, !busy, cx),
            sheet_field_row(
                "new-user-hint",
                "Password Hint",
                &sheet.hint,
                false,
                !busy,
                cx,
            ),
        ];
        let cancel = view.clone();
        let create = view.clone();
        form_sheet(
            "new-user-sheet",
            "New User",
            Some("The account name is used for the home folder and can’t be changed later.".into()),
            None,
            rows,
            sheet.error.clone(),
            None,
            vec![
                sheet_button(
                    "new-user-cancel",
                    "Cancel",
                    rmac_ui::DialogButtonKind::Normal,
                    !busy,
                    move |window, cx| {
                        cancel.update(cx, |settings, cx| settings.close_user_sheets(window, cx))
                    },
                ),
                sheet_button(
                    "new-user-create",
                    "Create User",
                    rmac_ui::DialogButtonKind::Primary,
                    !busy,
                    move |window, cx| {
                        create.update(cx, |settings, cx| settings.submit_new_user(window, cx))
                    },
                ),
            ],
        )
    }

    fn render_change_password(&self, sheet: &PasswordSheet, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.users.busy;
        let rows = vec![
            sheet_field_row(
                "change-password-old",
                "Old Password",
                &sheet.old,
                true,
                !busy,
                cx,
            ),
            sheet_field_row(
                "change-password-new",
                "New Password",
                &sheet.new,
                true,
                !busy,
                cx,
            ),
            sheet_field_row(
                "change-password-verify",
                "Verify",
                &sheet.verify,
                true,
                !busy,
                cx,
            ),
            sheet_field_row(
                "change-password-hint",
                "Password Hint",
                &sheet.hint,
                false,
                !busy,
                cx,
            ),
        ];
        let cancel = view.clone();
        let change = view.clone();
        form_sheet(
            "change-password-sheet",
            "Change Password",
            None,
            None,
            rows,
            sheet.error.clone(),
            busy.then(|| Spinner::small().into_any_element()),
            vec![
                sheet_button(
                    "change-password-cancel",
                    "Cancel",
                    rmac_ui::DialogButtonKind::Normal,
                    !busy,
                    move |window, cx| {
                        cancel.update(cx, |settings, cx| settings.close_user_sheets(window, cx))
                    },
                ),
                sheet_button(
                    "change-password-submit",
                    "Change Password",
                    rmac_ui::DialogButtonKind::Primary,
                    !busy,
                    move |window, cx| {
                        change.update(cx, |settings, cx| {
                            settings.submit_change_password(window, cx)
                        })
                    },
                ),
            ],
        )
    }

    fn render_picture_sheet(&self, sheet: &PictureSheet, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.users.busy;
        let mut grid = div().flex().flex_wrap().gap_2();
        for (index, face) in sheet.choices.iter().enumerate() {
            let face = face.clone();
            let pick = view.clone();
            let name = face
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Picture")
                .to_owned();
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("user-picture-{index}")))
                    .role(Role::Button)
                    .aria_label(name)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let face = face.clone();
                        pick.update(cx, |settings, cx| settings.set_user_picture(face, cx))
                    })
                    .child(account_picture(sheet.choices.get(index), "", FACE_TILE)),
            );
        }
        let header = if sheet.choices.is_empty() {
            None
        } else {
            Some(grid.into_any_element())
        };
        let choose = view.clone();
        let cancel = view.clone();
        form_sheet(
            "user-picture-sheet",
            "Edit Picture",
            Some("Choose a picture for your account. It appears on the login screen and in Settings.".into()),
            header,
            Vec::new(),
            sheet.error.clone(),
            Some(sheet_button(
                "user-picture-choose",
                "Choose File…",
                rmac_ui::DialogButtonKind::Normal,
                !busy,
                move |_, cx| choose.update(cx, |settings, cx| settings.choose_user_picture_file(cx)),
            )),
            vec![sheet_button(
                "user-picture-cancel",
                "Cancel",
                rmac_ui::DialogButtonKind::Normal,
                !busy,
                move |window, cx| {
                    cancel.update(cx, |settings, cx| settings.close_user_sheets(window, cx))
                },
            )],
        )
    }

    /// The Mac's alert: "Are you sure you want to delete the user account
    /// “Name”?" with the home-folder choice and Delete User.
    fn render_delete_user(&self, confirm: &DeleteConfirm, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.users.busy;
        let options = ["Don’t change the home folder", "Delete the home folder"];
        let choose = view.clone();
        let radios = RadioGroup::new("delete-user-home", options)
            .label("Home folder")
            .selected(usize::from(confirm.delete_home))
            .disabled(busy)
            .on_change(move |index, _, cx| {
                choose.update(cx, |settings, cx| {
                    if let Some(confirm) = settings.users.delete.as_mut() {
                        confirm.delete_home = index == 1;
                        cx.notify();
                    }
                })
            });
        let cancel = view.clone();
        let delete = view.clone();
        form_sheet(
            "delete-user-alert",
            format!(
                "Are you sure you want to delete the user account “{}”?",
                confirm.name
            ),
            Some(
                "Deleting this account can’t be undone. Choose what happens to its home folder."
                    .into(),
            ),
            Some(radios.into_any_element()),
            Vec::new(),
            None,
            None,
            vec![
                sheet_button(
                    "delete-user-cancel",
                    "Cancel",
                    rmac_ui::DialogButtonKind::Normal,
                    !busy,
                    move |window, cx| {
                        cancel.update(cx, |settings, cx| settings.close_user_sheets(window, cx))
                    },
                ),
                sheet_button(
                    "delete-user-confirm",
                    "Delete User",
                    rmac_ui::DialogButtonKind::Destructive,
                    !busy,
                    move |_, cx| delete.update(cx, |settings, cx| settings.confirm_delete_user(cx)),
                ),
            ],
        )
    }
}
