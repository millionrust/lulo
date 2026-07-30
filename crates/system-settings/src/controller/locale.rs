//! Language, region, and system keyboard transaction lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_locale_update(
        &mut self,
        result: std::result::Result<rmac_locale::Snapshot, rmac_locale::Error>,
    ) {
        self.locale_loading = false;
        self.locale_busy = false;
        match result {
            Ok(snapshot) => {
                self.locale = Some(snapshot);
                self.locale_error = None;
                self.locale_stream_error = None;
            }
            Err(error) => {
                self.locale_error =
                    Some(format!("Could not update language and region: {error}").into());
            }
        }
    }

    pub(super) fn queue_locale_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.locale_loading || self.locale_busy || self.locale_stream_refreshing {
            self.locale_refresh_pending = true;
            return;
        }
        self.locale_refresh_pending = false;
        self.locale_stream_refreshing = true;
        let generation = self.locale_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.locale_stream_refreshing = false;
                if locale_stream_snapshot_is_current(
                    generation,
                    this.locale_generation,
                    this.locale_loading,
                    this.locale_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.locale = Some(snapshot);
                            this.locale_error = None;
                            this.locale_stream_error = None;
                        }
                        Err(_) => {
                            this.locale_stream_error =
                                Some("Could not refresh changed language and region state".into());
                        }
                    }
                } else {
                    this.locale_refresh_pending = true;
                }
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_locale_refresh(&mut self, cx: &mut Context<Self>) {
        if self.locale_refresh_pending
            && !self.locale_loading
            && !self.locale_busy
            && !self.locale_stream_refreshing
        {
            self.queue_locale_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_loading || self.locale_busy {
            return;
        }
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_locale_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.locale_editor.is_some()
            || self.region_editor.is_some()
            || self.x11_layout_editor.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let language = snapshot.language().to_owned();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(language)
                .placeholder("en_US.UTF-8")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.locale_editor = Some(editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_locale_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.locale_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.locale_editor, &self.locale) else {
            return;
        };
        let language = editor.read(cx).value().trim().to_owned();
        let next = match snapshot.preview_language(&language) {
            Ok(assignments) => assignments
                .iter()
                .map(rmac_locale::Assignment::encoded)
                .collect::<Vec<_>>(),
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.apply_locale_assignments(next, snapshot.clone(), cx);
    }

    pub(super) fn start_region_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.region_editor.is_some()
            || self.locale_editor.is_some()
            || self.x11_layout_editor.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let region = snapshot.region_locale().to_owned();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(region)
                .placeholder("en_IN.UTF-8")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.region_editor = Some(editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_region_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.region_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_region(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.region_editor, &self.locale) else {
            return;
        };
        let region = editor.read(cx).value().trim().to_owned();
        let next = match snapshot.preview_region(&region) {
            Ok(assignments) => assignments
                .iter()
                .map(rmac_locale::Assignment::encoded)
                .collect::<Vec<_>>(),
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.apply_locale_assignments(next, snapshot.clone(), cx);
    }

    pub(super) fn apply_locale_assignments(
        &mut self,
        next: Vec<String>,
        previous: rmac_locale::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_locale(&next) })
                .await;
            let rollback = result
                .as_ref()
                .ok()
                .map(|applied| rmac_locale::LocaleRollback::new(&previous, applied));
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(rollback) = rollback {
                    this.locale_editor = None;
                    this.region_editor = None;
                    this.locale_revert = Some(rollback);
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_locale(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(rollback) = self.locale_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::restore_locale(&rollback) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.locale_revert = None;
                    this.locale_editor = None;
                    this.region_editor = None;
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_x11_keyboard_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locale_busy
            || self.x11_layout_editor.is_some()
            || self.locale_editor.is_some()
            || self.region_editor.is_some()
            || self.input.keyboard_layout_authority
                != rmac_input::KeyboardLayoutAuthority::SystemLocaled
        {
            return;
        }
        let Some(snapshot) = &self.locale else {
            return;
        };
        let layout = snapshot.x11_layout.clone();
        let variant = snapshot.x11_variant.clone();
        let options = snapshot.x11_options.clone();
        let layout_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(layout)
                .placeholder("us,de")
        });
        let variant_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(variant)
                .placeholder(",nodeadkeys")
        });
        let options_editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(options)
                .placeholder("grp:ctrl_space_toggle")
        });
        let focus = layout_editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.x11_layout_editor = Some(layout_editor);
        self.x11_variant_editor = Some(variant_editor);
        self.x11_options_editor = Some(options_editor);
        self.locale_error = None;
        cx.notify();
    }

    pub(super) fn cancel_x11_keyboard_edit(&mut self, cx: &mut Context<Self>) {
        if !self.locale_busy {
            self.x11_layout_editor = None;
            self.x11_variant_editor = None;
            self.x11_options_editor = None;
            self.locale_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let (Some(layout_editor), Some(variant_editor), Some(options_editor), Some(snapshot)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
            &self.locale,
        ) else {
            return;
        };
        let layout = layout_editor.read(cx).value().trim().to_owned();
        let variant = variant_editor.read(cx).value().trim().to_owned();
        let options = options_editor.read(cx).value().trim().to_owned();
        let keyboard = match snapshot.preview_x11_keyboard(&layout, &variant, &options) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                self.locale_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let previous = snapshot.clone();
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::set_x11_keyboard(&keyboard) })
                .await;
            let rollback = result
                .as_ref()
                .ok()
                .map(|applied| rmac_locale::KeyboardRollback::new(&previous, applied));
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(rollback) = rollback {
                    this.x11_layout_editor = None;
                    this.x11_variant_editor = None;
                    this.x11_options_editor = None;
                    this.x11_keyboard_revert = Some(rollback);
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_x11_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.locale_busy {
            return;
        }
        let Some(rollback) = self.x11_keyboard_revert.clone() else {
            return;
        };
        self.locale_busy = true;
        self.locale_generation = self.locale_generation.wrapping_add(1);
        self.locale_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_locale_linux::restore_x11_keyboard(&rollback) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.x11_keyboard_revert = None;
                }
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_language_region(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-language-region", "Refresh")
            .busy(self.locale_busy || self.locale_stream_refreshing)
            .disabled(self.locale_loading || self.locale_busy || self.locale_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_locale(cx));
            });
        let Some(snapshot) = &self.locale else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/languages.svg", secondary(), 22.0))
                    .child(text_block(
                        "System language and formats".into(),
                        Some("systemd-localed".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.locale_loading {
                    "Reading authoritative locale state and installed locales…"
                } else {
                    "The system locale service is unavailable. No local fallback controls are shown."
                }),
            ]);
        };

        let language_row = if let Some(editor) = &self.locale_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), 22.0))
                .child(text_block(
                    "Language".into(),
                    Some("Enter an exact locale installed on this computer".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("locale-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_locale_edit(cx));
                        }),
                )
                .child(
                    Button::new("locale-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_locale(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            row_base()
                .child(tile("icons/languages.svg", accent(), 22.0))
                .child(text_block(
                    "Language".into(),
                    Some("Validated against the system's installed locales".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(snapshot.language().to_owned()),
                )
                .child(
                    Button::new("locale-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.region_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_locale_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let region_row = if let Some(editor) = &self.region_editor {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Region".into(),
                    Some("Sets date, number, currency, and regional formats".into()),
                ))
                .child(div().w(px(180.0)).child(TextField::new(editor).small()))
                .child(
                    Button::new("region-cancel", "Cancel")
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_region_edit(cx));
                        }),
                )
                .child(
                    Button::new("region-apply", "Apply")
                        .primary()
                        .busy(self.locale_busy)
                        .disabled(self.locale_busy)
                        .on_click(move |_, _, cx| {
                            apply_view.update(cx, |settings, cx| settings.submit_region(cx));
                        }),
                )
                .into_any_element()
        } else {
            let edit_view = view.clone();
            let value = if snapshot.formats_are_mixed() {
                format!("Mixed · {}", snapshot.region_locale())
            } else {
                snapshot.region_locale().to_owned()
            };
            row_base()
                .child(tile("icons/globe.svg", accent(), 22.0))
                .child(text_block(
                    "Region".into(),
                    Some("System-wide formats, independent of display language".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(value),
                )
                .child(
                    Button::new("region-edit", "Edit")
                        .disabled(
                            self.locale_busy
                                || self.locale_editor.is_some()
                                || self.x11_layout_editor.is_some(),
                        )
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_region_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()
        };

        let mut cards = vec![card(vec![language_row, region_row])];
        if let Some(editor) = &self.locale_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_language(value.trim()) {
                Ok(preview) => {
                    cards.push(section_header("Assignments applied to the system"));
                    cards.push(card(
                        preview
                            .iter()
                            .map(|assignment| {
                                value_row(
                                    "icons/settings.svg",
                                    secondary(),
                                    assignment.key.clone().into(),
                                    assignment.value.clone().into(),
                                )
                            })
                            .collect(),
                    ));
                    if preview
                        .iter()
                        .any(|assignment| assignment.key.starts_with("LC_"))
                    {
                        cards.push(note_card(
                            "Existing LC_* format overrides are preserved. Applying changes LANG only.",
                        ));
                    }
                }
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }
        if let Some(editor) = &self.region_editor {
            let value = editor.read(cx).value();
            match snapshot.preview_region(value.trim()) {
                Ok(_) => cards.push(note_card(
                    "Applying changes regional date, number, currency, paper, address, telephone, and measurement formats without changing the display language or message locale.",
                )),
                Err(error) => cards.push(note_card(error.to_string())),
            }
        }

        cards.push(section_header("Format examples"));
        let preview = snapshot.format_preview.as_ref();
        cards.push(card(vec![
            locale_preview_row(
                "icons/clock.svg",
                "Dates and times",
                locale_format(snapshot, "LC_TIME"),
                preview.map(|preview| preview.date_time.as_str()),
            ),
            locale_preview_row(
                "icons/info.svg",
                "Numbers",
                locale_format(snapshot, "LC_NUMERIC"),
                preview.map(|preview| preview.number.as_str()),
            ),
            locale_preview_row(
                "icons/database.svg",
                "Currency",
                locale_format(snapshot, "LC_MONETARY"),
                preview.map(|preview| preview.currency.as_str()),
            ),
            locale_preview_row(
                "icons/settings.svg",
                "Measurement",
                locale_format(snapshot, "LC_MEASUREMENT"),
                None,
            ),
        ]));
        if let Some(error) = &snapshot.format_preview_error {
            cards.push(note_card(format!(
                "Format examples are unavailable: {error}. Locale assignments remain authoritative."
            )));
        }

        let keyboard = if snapshot.x11_layout.is_empty() {
            "Not reported by systemd-localed".to_owned()
        } else {
            let mut value = snapshot.x11_layout.clone();
            if !snapshot.x11_variant.is_empty() {
                value.push_str(" · ");
                value.push_str(&snapshot.x11_variant);
            }
            value
        };
        let layout_authority = self.input.keyboard_layout_authority;
        let can_edit_layout = layout_authority
            == rmac_input::KeyboardLayoutAuthority::SystemLocaled
            && snapshot.x11_layouts_error.is_none()
            && !snapshot.installed_x11_layouts.is_empty();
        cards.push(section_header("Input sources"));
        let editing_keyboard = self.x11_layout_editor.is_some();
        let mut keyboard_rows = if let (Some(layout), Some(variant), Some(options)) = (
            &self.x11_layout_editor,
            &self.x11_variant_editor,
            &self.x11_options_editor,
        ) {
            let cancel_view = view.clone();
            let apply_view = view.clone();
            vec![
                row_base()
                    .child(tile("icons/keyboard.svg", accent(), 22.0))
                    .child(text_block(
                        "XKB layouts".into(),
                        Some("Comma-separated installed names, in switch order".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(layout).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), 22.0))
                    .child(text_block(
                        "Variants".into(),
                        Some("One entry per layout; empty entries are allowed".into()),
                    ))
                    .child(div().w(px(190.0)).child(TextField::new(variant).small()))
                    .into_any_element(),
                row_base()
                    .child(tile("icons/settings.svg", secondary(), 22.0))
                    .child(text_block(
                        "Switching options".into(),
                        Some("For example grp:ctrl_space_toggle".into()),
                    ))
                    .child(div().w(px(150.0)).child(TextField::new(options).small()))
                    .child(
                        Button::new("x11-keyboard-cancel", "Cancel")
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                cancel_view.update(cx, |settings, cx| {
                                    settings.cancel_x11_keyboard_edit(cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("x11-keyboard-apply", "Apply")
                            .primary()
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                apply_view.update(cx, |settings, cx| {
                                    settings.submit_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            ]
        } else {
            let edit_view = view.clone();
            vec![row_base()
                .child(tile("icons/keyboard.svg", accent(), 22.0))
                .child(text_block(
                    "Keyboard layouts".into(),
                    Some("systemd-localed default and switch order".into()),
                ))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(keyboard),
                )
                .child(
                    Button::new("x11-keyboard-edit", "Edit")
                        .disabled(self.locale_busy || !can_edit_layout)
                        .on_click(move |_, window, cx| {
                            edit_view.update(cx, |settings, cx| {
                                settings.start_x11_keyboard_edit(window, cx);
                            });
                        }),
                )
                .into_any_element()]
        };
        if !editing_keyboard {
            keyboard_rows.push(value_row(
                "icons/settings.svg",
                secondary(),
                "Switching options".into(),
                if snapshot.x11_options.is_empty() {
                    "Not configured".into()
                } else {
                    snapshot.x11_options.clone().into()
                },
            ));
        }
        keyboard_rows.push(value_row(
            "icons/keyboard.svg",
            secondary(),
            "Console keymap".into(),
            if snapshot.console_keymap.is_empty() {
                "Not configured".into()
            } else {
                snapshot.console_keymap.clone().into()
            },
        ));
        if self.x11_keyboard_revert.is_some() {
            let revert_view = view.clone();
            keyboard_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Previous keyboard layout".into(),
                        Some("Exact model, layouts, variants, and options".into()),
                    ))
                    .child(
                        Button::new("x11-keyboard-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| {
                                    settings.revert_x11_keyboard(cx);
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(keyboard_rows));
        cards.push(note_card(match layout_authority {
            rmac_input::KeyboardLayoutAuthority::SystemLocaled => {
                "niri follows this systemd-localed layout because no explicit XKB override is present. Use the configured XKB switching option to move between multiple layouts."
            }
            rmac_input::KeyboardLayoutAuthority::NiriConfig => {
                "The niri config has an explicit XKB block, so it—not systemd-localed—owns this session's keyboard layout. The system default is read-only here to avoid overriding that choice."
            }
            rmac_input::KeyboardLayoutAuthority::IncludedConfig => {
                "A traversed niri include owns explicit XKB settings, so the systemd-localed default remains read-only here instead of competing with that configuration."
            }
            rmac_input::KeyboardLayoutAuthority::Unavailable => {
                "The active niri keyboard-layout authority could not be verified. The system default remains read-only."
            }
        }));
        if let Some(error) = &snapshot.x11_layouts_error {
            cards.push(note_card(format!(
                "Keyboard layout editing is unavailable: {error}."
            )));
        }
        if snapshot.installed_x11_layouts_truncated {
            cards.push(note_card(
                "The installed XKB layout inventory exceeded the bounded validation list.",
            ));
        }

        let mut authority_rows = vec![row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Authoritative state".into(),
                Some(
                    format!(
                        "Live localed changes · {} installed locales",
                        snapshot.installed_locales.len()
                    )
                    .into(),
                ),
            ))
            .child(refresh)
            .into_any_element()];
        if self.locale_revert.is_some() {
            let revert_view = view.clone();
            authority_rows.push(
                row_base()
                    .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
                    .child(text_block(
                        "Previous locale assignments".into(),
                        Some("Reverts only if the complete applied state is still current".into()),
                    ))
                    .child(
                        Button::new("locale-revert", "Revert")
                            .busy(self.locale_busy)
                            .disabled(self.locale_busy)
                            .on_click(move |_, _, cx| {
                                revert_view.update(cx, |settings, cx| settings.revert_locale(cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        cards.push(card(authority_rows));
        if snapshot.installed_locales_truncated {
            cards.push(note_card(
                "The installed locale inventory exceeded the bounded validation list.",
            ));
        }
        cards.push(note_card(
            "New applications and services use an applied locale immediately. Sign out and back in before judging the current desktop session.",
        ));
        self.pane(cards)
    }
}
