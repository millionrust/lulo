use super::*;

impl FinderView {
    pub(in crate::view) fn render_open_with(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let picker = self.open_with.as_ref()?;
        let name = picker
            .path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| "this file".into());
        let busy = picker.busy;
        let browsing = picker.browse.is_some();

        let mut body = div().v_flex().gap_2().px_5().py_4();
        let mut can_open = false;
        let mut selected_is_default = false;

        if let Some(browse) = &picker.browse {
            match browse {
                OpenWithBrowse::Loading => {
                    body = body.child(
                        div()
                            .h(px(180.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .v_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(Spinner::new())
                                    .child(
                                        div()
                                            .text_size(rmac_ui::text_px(13.0))
                                            .text_color(secondary())
                                            .child("Loading the application catalog…"),
                                    ),
                            ),
                    );
                }
                OpenWithBrowse::Ready(applications) => {
                    body = body.child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(format!("Choose an application to open “{name}” with")),
                    );
                    if applications.is_empty() {
                        body = body.child(
                            div()
                                .h(px(120.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(secondary())
                                .child("No applications are installed."),
                        );
                    } else {
                        can_open = true;
                        let mut rows = Vec::with_capacity(applications.len());
                        for (index, application) in applications.iter().enumerate() {
                            let selected = index == picker.selected;
                            let application_name = sanitize_dialog_name(&application.name);
                            let row = Button::new(("open-with-browse", index), application_name)
                                .selected(selected)
                                .disabled(busy)
                                .w_full()
                                .h(px(34.0))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.choose_browse_application(index, cx)
                                }));
                            rows.push(if selected {
                                row.primary().into_any_element()
                            } else {
                                row.ghost().into_any_element()
                            });
                        }
                        body = body.child(
                            div()
                                .id("open-with-browse-list")
                                .max_h(px(260.0))
                                .overflow_y_scroll()
                                .v_flex()
                                .gap_0p5()
                                .children(rows),
                        );
                        body = body.child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(secondary())
                                .child(
                                    "This only opens the file once; it can't be set as the default \
                                     application for this file type.",
                                ),
                        );
                    }
                }
            }
        } else if let Some(association) = &picker.association {
            if association.handlers.is_empty() {
                body = body.child(
                    div()
                        .v_flex()
                        .gap_3()
                        .items_center()
                        .py_4()
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(label())
                                .text_center()
                                .child(format!(
                                    "There is no application set to open the document \
                                     “{name}”."
                                )),
                        )
                        .child(
                            Button::new("open-with-choose-application", "Choose Application…")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.request_choose_application(cx)
                                })),
                        ),
                );
            } else {
                can_open = true;
                body = body.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(secondary())
                        .child(format!(
                            "Choose an application for “{name}” ({})",
                            association.mime_type
                        )),
                );
                let mut rows = Vec::with_capacity(association.handlers.len());
                for (index, application) in association.handlers.iter().enumerate() {
                    let selected = index == picker.selected;
                    let is_default = association.default_application_id.as_deref()
                        == Some(application.id.as_str());
                    if selected {
                        selected_is_default = is_default;
                    }
                    let application_name = sanitize_dialog_name(&application.name);
                    let row_label = if is_default {
                        format!("{application_name} — Default")
                    } else {
                        application_name
                    };
                    let row = Button::new(("open-with-handler", index), row_label)
                        .selected(selected)
                        .disabled(busy)
                        .w_full()
                        .h(px(34.0))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.choose_open_with(index, cx)),
                        );
                    rows.push(if selected {
                        row.primary().into_any_element()
                    } else {
                        row.ghost().into_any_element()
                    });
                }
                body = body.child(
                    div()
                        .id("open-with-list")
                        .max_h(px(260.0))
                        .overflow_y_scroll()
                        .v_flex()
                        .gap_0p5()
                        .children(rows),
                );

                let checked = selected_is_default || picker.make_default;
                body = body.child(
                    Toggle::new("open-with-default")
                        .checked(checked)
                        .label(if selected_is_default {
                            "This application is already the default"
                        } else {
                            "Always open this file type with this application"
                        })
                        .disabled(selected_is_default || busy)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_open_with_default(cx))),
                );
            }
        } else {
            body = body.child(
                div()
                    .h(px(150.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .v_flex()
                            .items_center()
                            .gap_3()
                            .child(Spinner::new())
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(secondary())
                                    .child("Finding compatible applications…"),
                            ),
                    ),
            );
        }
        if let Some(error) = &picker.error {
            body = body.child(
                div()
                    .id("open-with-error")
                    .rounded(px(rmac_ui::mac::radius_menu_item()))
                    .border_1()
                    .border_color(rmac_ui::mac::error_border())
                    .bg(rmac_ui::mac::error_background())
                    .px_3()
                    .py_2()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::danger())
                    .child(rmac_ui::user_error_message(
                        rmac_ui::ErrorSurface::Files,
                        error.as_ref(),
                        false,
                    )),
            );
        }

        let cancel_label = if browsing {
            "Back"
        } else if can_open {
            "Cancel"
        } else {
            "Close"
        };
        let buttons = div()
            .h(px(54.0))
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .px_5()
            .border_t_1()
            .border_color(sep())
            .child(
                rmac_ui::dialog_button(
                    "open-with-cancel",
                    cancel_label,
                    rmac_ui::DialogButtonKind::Normal,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this
                        .open_with
                        .as_ref()
                        .is_some_and(|picker| picker.browse.is_some())
                    {
                        this.cancel_choose_application(cx);
                    } else {
                        this.close_open_with(cx);
                    }
                })),
            )
            .child(
                rmac_ui::dialog_button(
                    "open-with-confirm",
                    if busy { "Opening…" } else { "Open" },
                    rmac_ui::DialogButtonKind::Primary,
                )
                .busy(busy)
                .disabled(busy || !can_open)
                .on_click(cx.listener(|this, _, _, cx| this.confirm_open_with(cx))),
            );

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rmac_ui::mac::scrim())
                .child(
                    div()
                        .w(px(440.0))
                        .max_h(px(520.0))
                        .v_flex()
                        .rounded(px(rmac_ui::mac::radius_card()))
                        .bg(rmac_ui::mac::raised())
                        .border_1()
                        .border_color(sep())
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(48.0))
                                .flex()
                                .items_center()
                                .px_5()
                                .border_b_1()
                                .border_color(sep())
                                .text_size(rmac_ui::text_px(15.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(label())
                                .child("Open With"),
                        )
                        .child(body)
                        .child(buttons),
                )
                .into_any_element(),
        )
    }
}
