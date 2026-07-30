//! Login-item and background-service transaction lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_login_items_update(
        &mut self,
        result: std::result::Result<rmac_login_items::Snapshot, rmac_login_items::Error>,
    ) {
        self.login_items_loading = false;
        self.login_item_busy = None;
        match result {
            Ok(snapshot) => {
                self.login_items = Some(snapshot);
                self.login_items_error = None;
                self.login_items_stream_error = None;
            }
            Err(error) => {
                self.login_items_error =
                    Some(format!("Could not update login items: {error}").into());
            }
        }
    }

    pub(super) fn queue_login_items_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.login_items_loading
            || self.login_item_busy.is_some()
            || self.login_items_stream_refreshing
        {
            self.login_items_refresh_pending = true;
            return;
        }
        self.login_items_refresh_pending = false;
        self.login_items_stream_refreshing = true;
        let generation = self.login_items_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_items_stream_refreshing = false;
                if login_items_stream_snapshot_is_current(
                    generation,
                    this.login_items_generation,
                    this.login_items_loading,
                    this.login_item_busy.is_some(),
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.login_items = Some(snapshot);
                            this.login_items_error = None;
                            this.login_items_stream_error = None;
                        }
                        Err(_) => {
                            this.login_items_stream_error =
                                Some("Could not refresh changed Login Items state".into());
                        }
                    }
                } else {
                    this.login_items_refresh_pending = true;
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_login_items_refresh(&mut self, cx: &mut Context<Self>) {
        if self.login_items_refresh_pending
            && !self.login_items_loading
            && self.login_item_busy.is_none()
            && !self.login_items_stream_refreshing
        {
            self.queue_login_items_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_login_items(&mut self, cx: &mut Context<Self>) {
        if self.login_items_loading
            || self.login_item_busy.is_some()
            || self.login_items_stream_refreshing
        {
            return;
        }
        self.login_item_busy = Some("refresh".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_login_item_enabled(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(id.clone());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_background_service_enabled(
        &mut self,
        id: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("systemd:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::set_background_enabled(&id, enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn reveal_login_item(
        &mut self,
        id: String,
        background: bool,
        cx: &mut Context<Self>,
    ) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("reveal:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    if background {
                        rmac_login_items_linux::background_service_source(&id)
                    } else {
                        rmac_login_items_linux::autostart_source(&id)
                    }
                })
                .await;
            let result = match path {
                Ok(path) => rmac_app_launch::reveal_item(path)
                    .await
                    .map_err(|_| "the file manager could not reveal this login item".to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                this.login_items_error = result
                    .err()
                    .map(|error| format!("Could not reveal login item: {error}").into());
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn choose_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some("choose".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_desktop_entry().await;
            let preview = match choice {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move { rmac_login_items_linux::prepare_add_source(&path) })
                        .await,
                ),
                Ok(None) => None,
                Err(_) => Some(Err(rmac_login_items::Error::new(
                    rmac_login_items::ErrorKind::Unavailable,
                    "the desktop-entry chooser is unavailable",
                ))),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                match preview {
                    Some(Ok(preview)) => {
                        this.login_item_add = Some(preview);
                        this.login_items_error = None;
                    }
                    Some(Err(error)) => {
                        this.login_items_error =
                            Some(format!("Could not add login item: {error}").into());
                    }
                    None => {}
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_add_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some(preview) = self.login_item_add.clone() else {
            return;
        };
        self.login_item_busy = Some("add".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::add_source(&preview) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_add = None;
                }
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_remove_login_item(&mut self, id: String, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        self.login_item_busy = Some(format!("prepare-remove:{id}"));
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::prepare_remove_autostart(&id) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.login_item_busy = None;
                match result {
                    Ok(preview) => {
                        this.login_item_remove = Some(preview);
                        this.login_items_error = None;
                    }
                    Err(error) => {
                        this.login_items_error =
                            Some(format!("Could not prepare login item removal: {error}").into());
                    }
                }
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remove_login_item(&mut self, cx: &mut Context<Self>) {
        if self.login_item_busy.is_some() {
            return;
        }
        let Some(preview) = self.login_item_remove.clone() else {
            return;
        };
        self.login_item_busy = Some("remove".into());
        self.login_items_generation = self.login_items_generation.wrapping_add(1);
        self.login_items_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_login_items_linux::remove_autostart(&preview) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.login_item_remove = None;
                }
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_login_items(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-login-items", "Refresh")
            .busy(
                self.login_item_busy.as_deref() == Some("refresh")
                    || self.login_items_stream_refreshing,
            )
            .disabled(
                self.login_items_loading
                    || self.login_item_busy.is_some()
                    || self.login_items_stream_refreshing,
            )
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_login_items(cx));
            });
        let Some(snapshot) = &self.login_items else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/app-window.svg", secondary(), 22.0))
                    .child(text_block(
                        "Open at login".into(),
                        Some("XDG autostart directories".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.login_items_loading {
                    "Reading effective XDG autostart entries…"
                } else {
                    "Autostart entries are unavailable. No private fallback toggles are shown."
                }),
            ]);
        };

        let choose_view = view.clone();
        let mut cards = vec![section_header("Open at login")];
        cards.push(card(vec![row_base()
            .child(tile("icons/app-window.svg", accent(), 22.0))
            .child(text_block(
                "Add application entry".into(),
                Some("Choose a local .desktop file to review".into()),
            ))
            .child(
                Button::new("choose-login-item", "Add…")
                    .busy(self.login_item_busy.as_deref() == Some("choose"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        choose_view.update(cx, |settings, cx| settings.choose_login_item(cx));
                    }),
            )
            .into_any_element()]));
        if let Some(preview) = &self.login_item_add {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Review “{}” ({}). This command will be allowed to run at sign-in. {}",
                preview.name,
                preview.id,
                if preview.replacing {
                    "A user entry with this filename exists and will be replaced only after confirmation."
                } else {
                    "The validated entry will be copied into your user autostart directory."
                }
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Command at sign-in".into(),
                    Some(preview.command.clone().into()),
                ))
                .into_any_element()]));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", secondary(), 22.0))
                .child(text_block(
                    if preview.replacing {
                        "Replace existing login item"
                    } else {
                        "Add login item"
                    }
                    .into(),
                    Some("The installed copy will start enabled".into()),
                ))
                .child(
                    Button::new("cancel-add-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_add = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new(
                        "confirm-add-login-item",
                        if preview.replacing { "Replace" } else { "Add" },
                    )
                    .primary()
                    .busy(self.login_item_busy.as_deref() == Some("add"))
                    .disabled(self.login_item_busy.is_some())
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_add_login_item(cx));
                    }),
                )
                .into_any_element()]));
        }
        if snapshot.items.is_empty() {
            cards.push(note_card("No effective XDG autostart entries were found."));
        } else {
            let rows = snapshot
                .items
                .iter()
                .map(|item| {
                    let id = item.id.clone();
                    let reveal_id = item.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let remove_view = view.clone();
                    let remove_id = item.id.clone();
                    let busy = self.login_item_busy.as_deref() == Some(item.id.as_str());
                    let remove_key = format!("prepare-remove:{}", item.id);
                    let preparing_remove =
                        self.login_item_busy.as_deref() == Some(remove_key.as_str());
                    let reveal_key = format!("reveal:{}", item.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = item.session_detail.clone().unwrap_or_else(|| {
                        if item.user_owned {
                            "User autostart entry".into()
                        } else {
                            "System autostart entry".into()
                        }
                    });
                    row_base()
                        .child(tile("icons/app-window.svg", accent(), 22.0))
                        .child(text_block(item.name.clone().into(), Some(subtitle.into())))
                        .when(item.user_owned && !item.managed_override, |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "remove-login-item-{}",
                                        item.id
                                    ))),
                                    "Remove…",
                                )
                                .busy(preparing_remove)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    remove_view.update(cx, |settings, cx| {
                                        settings.request_remove_login_item(remove_id.clone(), cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Button::new(
                                ElementId::from(SharedString::from(format!(
                                    "reveal-login-item-{}",
                                    item.id
                                ))),
                                "Show in Files",
                            )
                            .busy(revealing)
                            .disabled(self.login_item_busy.is_some())
                            .on_click(move |_, _, cx| {
                                reveal_view.update(cx, |settings, cx| {
                                    settings.reveal_login_item(reveal_id.clone(), false, cx);
                                });
                            }),
                        )
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "login-item-{}",
                                item.id
                            ))))
                            .checked(item.enabled)
                            .disabled(self.login_item_busy.is_some() || !item.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_login_item_enabled(id.clone(), *enabled, cx);
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(preview) = &self.login_item_remove {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Remove “{}”? Its user-owned desktop entry will be moved to Trash. If a system entry with the same filename exists, it will remain visible but disabled.",
                preview.name
            )));
            cards.push(card(vec![row_base()
                .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                .child(text_block(
                    "Confirm removal".into(),
                    Some("This does not delete the application itself".into()),
                ))
                .child(
                    Button::new("cancel-remove-login-item", "Cancel")
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| {
                                settings.login_item_remove = None;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Button::new("confirm-remove-login-item", "Move to Trash")
                        .primary()
                        .busy(self.login_item_busy.as_deref() == Some("remove"))
                        .disabled(self.login_item_busy.is_some())
                        .on_click(move |_, _, cx| {
                            confirm_view.update(cx, |settings, cx| {
                                settings.confirm_remove_login_item(cx);
                            });
                        }),
                )
                .into_any_element()]));
        }

        cards.push(section_header("Allow in background"));
        if snapshot.background_services.is_empty() {
            cards.push(note_card(if snapshot.background_services_error.is_some() {
                "The systemd user manager is unavailable. XDG application login items remain usable."
            } else {
                "No enabled or user-installed systemd background services were found."
            }));
        } else {
            let rows = snapshot
                .background_services
                .iter()
                .map(|service| {
                    let id = service.id.clone();
                    let reveal_id = service.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let busy_key = format!("systemd:{}", service.id);
                    let busy = self.login_item_busy.as_deref() == Some(busy_key.as_str());
                    let reveal_key = format!("reveal:{}", service.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = format!("{} · {}", service.detail, service.state.label());
                    row_base()
                        .child(tile("icons/settings.svg", secondary(), 22.0))
                        .child(text_block(
                            service.name.clone().into(),
                            Some(subtitle.into()),
                        ))
                        .when(service.source.is_some(), |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "reveal-background-service-{}",
                                        service.id
                                    ))),
                                    "Show in Files",
                                )
                                .busy(revealing)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    reveal_view.update(cx, |settings, cx| {
                                        settings.reveal_login_item(reveal_id.clone(), true, cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "background-service-{}",
                                service.id
                            ))))
                            .checked(service.enabled)
                            .disabled(self.login_item_busy.is_some() || !service.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_background_service_enabled(
                                        id.clone(),
                                        *enabled,
                                        cx,
                                    );
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(error) = &snapshot.background_services_error {
            cards.push(note_card(format!(
                "Background service status is unavailable: {error}"
            )));
        }
        if snapshot.background_services_truncated {
            cards.push(note_card(
                "The systemd user service inventory exceeded the bounded display limit.",
            ));
        }

        if !snapshot.issues.is_empty() {
            cards.push(section_header("Entries needing attention"));
            cards.push(card(
                snapshot
                    .issues
                    .iter()
                    .map(|issue| {
                        row_base()
                            .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                            .child(text_block(
                                issue.file.clone().into(),
                                Some(issue.detail.clone().into()),
                            ))
                            .into_any_element()
                    })
                    .collect(),
            ));
        }
        let refresh_row = row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Authoritative state".into(),
                Some("Live XDG files · systemd user unit changes".into()),
            ))
            .child(refresh)
            .into_any_element();
        cards.push(card(vec![refresh_row]));
        if snapshot.truncated {
            cards.push(note_card(
                "The autostart inventory exceeded the bounded display limit.",
            ));
        }
        cards.push(note_card(
            "Changes to systemd user services take effect at the next sign-in; this pane does not start or stop running services. Adding or removing systemd unit files remains an administrator workflow.",
        ));
        self.pane(cards)
    }
}
