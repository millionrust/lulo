use super::*;

const PRINTER_TILE: f32 = 32.0;

fn printer_tile() -> AnyElement {
    tile("icons/printer.svg", secondary(), PRINTER_TILE).into_any_element()
}

impl Settings {
    pub(in crate::controller) fn render_printers(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let state = &self.printers;
        let busy = state.busy;
        let mut cards: Vec<Div> = Vec::new();

        // Default printer and paper size.
        let mut default_choices: Vec<PopupChoice> = Vec::new();
        for printer in &state.list {
            let name = printer.name.clone();
            let set = view.clone();
            default_choices.push(choice(
                printer.display_name().to_owned(),
                state.default.as_deref() == Some(printer.name.as_str()),
                move |_, cx| {
                    let name = name.clone();
                    set.update(cx, |settings, cx| settings.set_default_printer(name, cx))
                },
            ));
        }
        let default_value = popup_value(
            &default_choices,
            if state.list.is_empty() {
                "No Printers"
            } else {
                "None"
            },
        );
        let has_printers = !state.list.is_empty();
        let paper_choices: Vec<PopupChoice> = PaperSize::ALL
            .into_iter()
            .map(|size| {
                let set = view.clone();
                choice(size.label(), state.paper == Some(size), move |_, cx| {
                    set.update(cx, |settings, cx| settings.set_paper_size(size, cx))
                })
            })
            .collect();
        let paper_value = popup_value(&paper_choices, "A4");
        cards.push(card(vec![
            popup_row(
                "printers-default-printer",
                "Default printer",
                None,
                default_value,
                default_choices,
                has_printers && !busy,
            ),
            popup_row(
                "printers-default-paper",
                "Default paper size",
                None,
                paper_value,
                paper_choices,
                state.loaded && !busy,
            ),
        ]));

        cards.push(section_header("Printers"));
        if state.list.is_empty() {
            cards.push(card(vec![group_placeholder(
                if state.loading || !state.loaded {
                    "Loading printers…"
                } else {
                    "No Printers"
                },
            )
            .into_any_element()]));
        } else {
            let rows = state
                .list
                .iter()
                .map(|printer| {
                    let name = printer.name.clone();
                    let open = view.clone();
                    let is_default = state.default.as_deref() == Some(printer.name.as_str());
                    let status = if is_default {
                        format!("{}, Default", printer.status_label())
                    } else {
                        printer.status_label().to_owned()
                    };
                    large_nav_row(
                        SharedString::from(format!("printer-{}", printer.name)),
                        printer_tile(),
                        printer.display_name().to_owned(),
                        Some(subtitle_text(status)),
                        None,
                        move |_, cx| {
                            let name = name.clone();
                            open.update(cx, |settings, cx| {
                                settings.printers.info = Some(name);
                                cx.notify();
                            })
                        },
                    )
                })
                .collect();
            cards.push(card(rows));
        }
        let add = view.clone();
        cards.push(footer_buttons(vec![push_button(
            "printers-add",
            "Add Printer, Scanner or Fax…",
        )
        .disabled(busy || !state.loaded || (state.error.is_some() && state.list.is_empty()))
        .on_click(move |_, window, cx| {
            add.update(cx, |settings, cx| settings.open_add_printer(window, cx))
        })
        .into_any_element()]));
        if let Some(error) = &state.error {
            cards.push(note_card(error.clone()));
        }
        self.pane(cards)
    }

    pub(in crate::controller) fn render_printers_overlay(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let state = &self.printers;
        let dialog = if let Some(name) = &state.remove {
            self.render_remove_printer(name, cx)
        } else if let Some(sheet) = &state.add {
            self.render_add_printer(sheet, cx)
        } else if let Some(queue) = &state.queue {
            self.render_print_queue(queue, cx)
        } else if let Some(name) = &state.info {
            self.render_printer_info(name, cx)?
        } else {
            return None;
        };
        Some(
            dialog
                .restore_focus_to(self.content_focus.clone())
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.close_printer_sheets(cx);
                    }
                }))
                .into_any_element(),
        )
    }

    fn render_printer_info(&self, name: &str, cx: &Context<Self>) -> Option<rmac_ui::Dialog> {
        let view = cx.entity();
        let state = &self.printers;
        let printer = state.printer(name)?;
        let busy = state.busy;
        let mut rows = vec![fact_row("Name", printer.name.clone())];
        if !printer.location.is_empty() {
            rows.push(fact_row("Location", printer.location.clone()));
        }
        if !printer.make_and_model.is_empty() {
            rows.push(fact_row("Kind", printer.make_and_model.clone()));
        }
        rows.push(fact_row("Status", printer.status_label()));
        let remove = view.clone();
        let remove_name = printer.name.clone();
        let queue = view.clone();
        let queue_name = printer.name.clone();
        let ok = view.clone();
        Some(form_sheet(
            "printer-info-sheet",
            printer.display_name().to_owned(),
            None,
            Some(printer_tile()),
            rows,
            None,
            Some(sheet_button(
                "printer-info-remove",
                "Remove Printer…",
                rmac_ui::DialogButtonKind::Destructive,
                !busy,
                move |_, cx| {
                    let name = remove_name.clone();
                    remove.update(cx, |settings, cx| {
                        settings.printers.remove = Some(name);
                        settings.printers.info = None;
                        cx.notify();
                    })
                },
            )),
            vec![
                sheet_button(
                    "printer-info-queue",
                    "Printer Queue…",
                    rmac_ui::DialogButtonKind::Normal,
                    true,
                    move |_, cx| {
                        let name = queue_name.clone();
                        queue.update(cx, |settings, cx| settings.open_print_queue(name, cx))
                    },
                ),
                sheet_button(
                    "printer-info-ok",
                    "OK",
                    rmac_ui::DialogButtonKind::Primary,
                    true,
                    move |_, cx| ok.update(cx, |settings, cx| settings.close_printer_sheets(cx)),
                ),
            ],
        ))
    }

    fn render_remove_printer(&self, name: &str, cx: &Context<Self>) -> rmac_ui::Dialog {
        let display = self
            .printers
            .printer(name)
            .map(|printer| printer.display_name().to_owned())
            .unwrap_or_else(|| name.to_owned());
        let busy = self.printers.busy;
        rmac_ui::alert_cancel_default(
            format!("Are you sure you want to delete printer “{display}”?"),
            "Documents waiting to print on it are cancelled.",
            vec![
                rmac_ui::dialog_button(
                    "printer-remove-cancel",
                    "Cancel",
                    rmac_ui::DialogButtonKind::Normal,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.printers.remove = None;
                    cx.notify();
                }))
                .into_any_element(),
                rmac_ui::dialog_button(
                    "printer-remove-confirm",
                    "Delete Printer",
                    rmac_ui::DialogButtonKind::Destructive,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.confirm_remove_printer(cx)))
                .into_any_element(),
            ],
        )
    }

    fn render_add_printer(&self, sheet: &AddSheet, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.printers.busy;
        let list: AnyElement = if sheet.searching {
            Progress::indeterminate()
                .label("Looking for printers…")
                .into_any_element()
        } else if sheet.devices.is_empty() {
            group_placeholder("No printers were found on the network. Check that the printer is on and connected.")
                .into_any_element()
        } else {
            let mut list = group();
            for (index, device) in sheet.devices.iter().enumerate() {
                let pick = view.clone();
                list = list.child(
                    ListRow::new(
                        SharedString::from(format!("add-printer-device-{index}")),
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(label())
                                    .child(device.display_name().to_owned()),
                            )
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child(device.kind()),
                            ),
                    )
                    .aria_label(format!("{}, {}", device.display_name(), device.kind()))
                    .selected(sheet.selected == Some(index))
                    .h(px(style::NAV_ROW_HEIGHT))
                    .on_activate(move |_, window, cx| {
                        pick.update(cx, |settings, cx| {
                            settings.select_new_printer(index, window, cx)
                        })
                    }),
                );
            }
            list.into_any_element()
        };
        let rows = if sheet.selected.is_some() {
            vec![
                sheet_field_row("add-printer-name", "Name", &sheet.name, false, !busy, cx),
                sheet_field_row(
                    "add-printer-location",
                    "Location",
                    &sheet.location,
                    false,
                    !busy,
                    cx,
                ),
                fact_row("Use", "Driverless printing (IPP Everywhere)"),
            ]
        } else {
            Vec::new()
        };
        let cancel = view.clone();
        let add = view.clone();
        form_sheet(
            "add-printer-sheet",
            "Add Printer",
            Some("Lulo OS sets up printers that print without a driver: AirPrint, IPP Everywhere and Mopria printers on the network or USB.".into()),
            Some(list),
            rows,
            sheet.error.clone(),
            busy.then(|| Spinner::small().into_any_element()),
            vec![
                sheet_button(
                    "add-printer-cancel",
                    "Cancel",
                    rmac_ui::DialogButtonKind::Normal,
                    !busy,
                    move |_, cx| cancel.update(cx, |settings, cx| settings.close_printer_sheets(cx)),
                ),
                sheet_button(
                    "add-printer-add",
                    "Add",
                    rmac_ui::DialogButtonKind::Primary,
                    !busy && sheet.selected.is_some(),
                    move |_, cx| add.update(cx, |settings, cx| settings.submit_add_printer(cx)),
                ),
            ],
        )
    }

    fn render_print_queue(&self, queue: &QueueSheet, cx: &Context<Self>) -> rmac_ui::Dialog {
        let view = cx.entity();
        let busy = self.printers.busy;
        let printer = self.printers.printer(&queue.printer);
        let title = printer
            .map(|printer| printer.display_name().to_owned())
            .unwrap_or_else(|| queue.printer.clone());
        let paused = printer.is_some_and(Printer::is_paused);
        let status = printer.map(Printer::status_label).unwrap_or("Unavailable");
        let mut rows: Vec<AnyElement> = Vec::new();
        if queue.loading {
            rows.push(group_placeholder("Loading…").into_any_element());
        } else if queue.jobs.is_empty() {
            rows.push(group_placeholder("No Jobs").into_any_element());
        } else {
            for job in &queue.jobs {
                let id = job.id;
                let cancel = view.clone();
                let detail = if job.size_kb > 0 {
                    format!("{} · {} · {} KB", job.owner, job.state.label(), job.size_kb)
                } else {
                    format!("{} · {}", job.owner, job.state.label())
                };
                rows.push(
                    row_base()
                        .child(text_block(job.name.clone().into(), Some(detail.into())))
                        .child(
                            push_button(
                                SharedString::from(format!("print-job-cancel-{id}")),
                                "Cancel Job",
                            )
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |settings, cx| settings.cancel_print_job(id, cx))
                            }),
                        )
                        .into_any_element(),
                );
            }
        }
        let pause = view.clone();
        let name = queue.printer.clone();
        let close = view.clone();
        form_sheet(
            "print-queue-sheet",
            title,
            Some(SharedString::from(status)),
            None,
            rows,
            queue.error.clone(),
            printer.map(|_| {
                sheet_button(
                    "print-queue-pause",
                    if paused { "Resume" } else { "Pause" },
                    rmac_ui::DialogButtonKind::Normal,
                    !busy,
                    move |_, cx| {
                        let name = name.clone();
                        pause.update(cx, |settings, cx| {
                            settings.set_printer_paused(name, !paused, cx)
                        })
                    },
                )
            }),
            vec![sheet_button(
                "print-queue-close",
                "Close",
                rmac_ui::DialogButtonKind::Primary,
                true,
                move |_, cx| close.update(cx, |settings, cx| settings.close_printer_sheets(cx)),
            )],
        )
    }
}
