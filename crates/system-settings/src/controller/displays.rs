//! Display configuration, confirmation, and presentation authority.

use super::*;

impl Settings {
    pub(super) fn finish_display_update(
        &mut self,
        result: std::result::Result<rmac_display::Snapshot, rmac_display::Error>,
    ) {
        self.display_loading = false;
        self.display_busy = false;
        match result {
            Ok(snapshot) => {
                self.display = snapshot;
                self.display_error = None;
            }
            Err(error) => {
                self.display_error = Some(format!("Could not update Displays: {error}").into());
            }
        }
    }

    pub(super) fn request_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            self.display_refresh_pending = true;
            return;
        }
        self.display_refresh_pending = false;
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if this.display_generation == generation
                    && !this.display_loading
                    && !this.display_busy
                    && this.display_confirmation.is_none()
                {
                    this.finish_display_update(result);
                    cx.notify();
                } else {
                    this.display_refresh_pending = true;
                }
            });
        })
        .detach();
    }

    pub(super) fn flush_display_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.display_refresh_pending
            && !self.display_loading
            && !self.display_busy
            && self.display_confirmation.is_none()
        {
            self.request_display_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_displays(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || self.display_confirmation.is_some() {
            return;
        }
        self.display_refresh_pending = false;
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                if succeeded {
                    this.display_generation = this.display_generation.wrapping_add(1);
                    this.display_confirmation = None;
                }
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_display_change(&mut self, change: DisplayChange, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_configure
        {
            return;
        }
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { change.apply() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(applied) => {
                        let confirmed = applied.snapshot.clone();
                        this.finish_display_update(Ok(applied.snapshot));
                        this.begin_display_confirmation(applied.baseline, confirmed, cx);
                    }
                    Err(error) => {
                        this.finish_display_update(Err(error));
                        this.flush_display_stream_refresh(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn begin_display_confirmation(
        &mut self,
        baseline: rmac_display::Snapshot,
        applied: rmac_display::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        self.display_confirmation = Some(DisplayConfirmation {
            baseline,
            applied,
            generation,
            seconds_remaining: DISPLAY_CONFIRMATION_SECONDS,
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            for remaining in (0..DISPLAY_CONFIRMATION_SECONDS).rev() {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let update = this.update(cx, |this: &mut Settings, cx| {
                    let current = this.display_confirmation.as_ref().is_some_and(|pending| {
                        pending.generation == generation && this.display_generation == generation
                    });
                    if !current {
                        return false;
                    }
                    if remaining == 0 {
                        this.revert_display_change(cx);
                    } else if let Some(pending) = &mut this.display_confirmation {
                        pending.seconds_remaining = remaining;
                        cx.notify();
                    }
                    true
                });
                if !matches!(update, Ok(true)) || remaining == 0 {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn keep_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_persist {
            return;
        }
        let Some(pending) = self.display_confirmation.clone() else {
            return;
        };
        let Some(primary) = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .or_else(|| {
                self.display
                    .outputs
                    .iter()
                    .find(|output| output.logical.is_some())
            })
            .map(|output| output.id.clone())
        else {
            self.display_error = Some("No enabled display can be saved as Main".into());
            cx.notify();
            return;
        };
        let layout = match rmac_display::current_layout(&self.display, &primary) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error =
                    Some(format!("Could not prepare display layout: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_confirmation = None;
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(snapshot) => {
                        this.finish_display_update(Ok(snapshot));
                        this.flush_display_stream_refresh(cx);
                    }
                    Err(error) => {
                        this.display_busy = false;
                        this.display_error =
                            Some(format!("Could not save display layout: {error}").into());
                        this.begin_display_confirmation(pending.baseline, pending.applied, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_primary_display(&mut self, output: String, cx: &mut Context<Self>) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_persist
        {
            return;
        }
        let layout = match rmac_display::current_layout(&self.display, &output) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error = Some(format!("Could not select Main display: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn revert_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        let Some(pending) = self.display_confirmation.take() else {
            return;
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_display::restore_snapshot(&pending.baseline, &pending.applied)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_displays(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let keep_view = view.clone();
        let confirmation_seconds = self
            .display_confirmation
            .as_ref()
            .map(|pending| pending.seconds_remaining);
        let mut cards = vec![div()
            .flex()
            .items_center()
            .justify_between()
            .px_1()
            .pb_1()
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(if self.display.compositor.is_empty() {
                        "Displays".to_string()
                    } else {
                        format!("Displays · {}", self.display.compositor)
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(confirmation_seconds, |actions, seconds| {
                        actions
                            .child(
                                div()
                                    .id("display-keep")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.0))
                                    .text_size(rmac_ui::text_px(12.0))
                                    .text_color(accent())
                                    .cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .child(format!("Keep Changes ({seconds}s)"))
                                    .on_click(move |_, _, cx| {
                                        keep_view.update(cx, |settings, cx| {
                                            settings.keep_display_change(cx)
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .id("display-revert")
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.0))
                                    .text_size(rmac_ui::text_px(12.0))
                                    .text_color(hsl(0xff3b30))
                                    .cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .child("Revert")
                                    .on_click(move |_, _, cx| {
                                        revert_view.update(cx, |settings, cx| {
                                            settings.revert_display_change(cx)
                                        });
                                    }),
                            )
                    })
                    .child(
                        div()
                            .id("display-refresh")
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(accent())
                            .cursor_pointer()
                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            .child(if self.display_busy {
                                "Applying…"
                            } else {
                                "Refresh"
                            })
                            .on_click(move |_, _, cx| {
                                refresh_view
                                    .update(cx, |settings, cx| settings.refresh_displays(cx));
                            }),
                    ),
            )];
        if self.display_loading {
            cards.push(note_card("Loading displays from the compositor…"));
            return self.pane(cards);
        }
        if !self.display.available {
            cards.push(note_card(
                "The display service is not available in this desktop session.",
            ));
            return self.pane(cards);
        }
        if self.display.outputs.is_empty() {
            cards.push(note_card("No displays were detected."));
        }

        let enabled_outputs = self
            .display
            .outputs
            .iter()
            .filter(|output| output.logical.is_some())
            .count();
        if enabled_outputs > 1 {
            cards.push(section_header("Arrange"));
            cards.push(display_layout_preview(&self.display.outputs));
            if !self.display.mirror_supported {
                cards.push(note_card(
                    "niri does not provide native display mirroring. Displays remain extended; wl-mirror can mirror content without pretending it is a compositor layout mode.",
                ));
            }
        }
        let main_output = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .cloned();

        for output in &self.display.outputs {
            let title = if output.primary {
                format!("{} · Main", output.name)
            } else {
                output.name.clone()
            };
            cards.push(
                div()
                    .px_1()
                    .pt_2()
                    .pb_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(title),
            );
            let mut rows = Vec::new();
            if let Some(detail) = &output.detail {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Type".into(),
                    detail.clone().into(),
                ));
            }
            rows.push(value_row(
                "icons/monitor.svg",
                accent(),
                "Connector".into(),
                output.connector.clone().into(),
            ));
            if let Some(mode) = output.current_mode() {
                rows.push(value_row(
                    "icons/monitor.svg",
                    secondary(),
                    "Resolution".into(),
                    mode.label().into(),
                ));
            } else {
                rows.push(value_row(
                    "icons/monitor.svg",
                    secondary(),
                    "Status".into(),
                    "Disabled".into(),
                ));
            }
            if let Some(logical) = &output.logical {
                rows.push(value_row(
                    "icons/settings.svg",
                    secondary(),
                    "Scale".into(),
                    format!("{}%", (logical.scale * 100.0).round() as u32).into(),
                ));
                rows.push(value_row(
                    "icons/refresh-cw.svg",
                    secondary(),
                    "Rotation".into(),
                    logical.transform.label().into(),
                ));
                rows.push(value_row(
                    "icons/folder-symlink.svg",
                    secondary(),
                    "Position".into(),
                    format!("{}, {}", logical.x, logical.y).into(),
                ));
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Logical Size".into(),
                    format!("{} × {}", logical.width, logical.height).into(),
                ));
            }
            if let Some((width, height)) = output.physical_size_mm {
                rows.push(value_row(
                    "icons/info.svg",
                    secondary(),
                    "Physical Size".into(),
                    format!("{width} × {height} mm").into(),
                ));
            }
            if output.primary {
                rows.push(value_row(
                    "icons/monitor.svg",
                    accent(),
                    "Main Display".into(),
                    "This display".into(),
                ));
            } else if output.logical.is_some() && self.display.can_persist {
                let output_id = output.id.clone();
                let primary_view = view.clone();
                rows.push(
                    row_base()
                        .id(SharedString::from(format!("display-primary-{output_id}")))
                        .child(text_block(
                            "Use as Main Display".into(),
                            Some("Anchors rmac shell surfaces and niri startup focus".into()),
                        ))
                        .child(glyph("icons/chevron-right.svg", 14.0, secondary()))
                        .when(
                            !self.display_busy && self.display_confirmation.is_none(),
                            |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        primary_view.update(cx, |settings, cx| {
                                            settings.set_primary_display(output_id.clone(), cx)
                                        });
                                    })
                            },
                        )
                        .into_any_element(),
                );
            }
            cards.push(card(rows));

            let Some(logical) = output.logical.as_ref() else {
                continue;
            };
            if !self.display.can_configure {
                continue;
            }

            if !output.primary {
                if let (Some(main), Some(moving)) = (main_output.as_ref(), output.logical.as_ref())
                {
                    if let Some(anchor) = main.logical.as_ref() {
                        cards.push(section_header("Arrange relative to Main"));
                        let placement_rows = [
                            DisplayPlacement::Left,
                            DisplayPlacement::Right,
                            DisplayPlacement::Above,
                            DisplayPlacement::Below,
                        ]
                        .into_iter()
                        .filter_map(|placement| {
                            let (x, y) = relative_display_position(moving, anchor, placement)?;
                            let selected = moving.x == x && moving.y == y;
                            let output_id = output.id.clone();
                            let expected_output = output.clone();
                            let placement_view = view.clone();
                            Some(
                                row_base()
                                    .id(SharedString::from(format!(
                                        "display-position-{output_id}-{}",
                                        placement.label()
                                    )))
                                    .child(text_block(placement.label().into(), None))
                                    .when(selected, |row| {
                                        row.child(glyph("icons/check.svg", 14.0, accent()))
                                    })
                                    .when(!selected, |row| {
                                        row.cursor_pointer()
                                            .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                            .on_click(move |_, _, cx| {
                                                let change = DisplayChange::Position {
                                                    output: expected_output.clone(),
                                                    x,
                                                    y,
                                                };
                                                placement_view.update(cx, |settings, cx| {
                                                    settings.apply_display_change(change, cx)
                                                });
                                            })
                                    })
                                    .into_any_element(),
                            )
                        })
                        .collect();
                        cards.push(card(placement_rows));
                    }
                }
            }

            if (0.5..=4.0).contains(&logical.scale) {
                cards.push(section_header("Scale"));
                let scale_rows = [1.0, 1.25, 1.5, 1.75, 2.0]
                    .into_iter()
                    .map(|scale| {
                        let selected = (logical.scale - scale).abs() < 0.001;
                        let output_id = output.id.clone();
                        let expected_output = output.clone();
                        let scale_view = view.clone();
                        row_base()
                            .id(SharedString::from(format!(
                                "display-scale-{output_id}-{scale}"
                            )))
                            .child(text_block(
                                format!("{}%", (scale * 100.0) as u32).into(),
                                None,
                            ))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Scale {
                                            output: expected_output.clone(),
                                            scale,
                                        };
                                        scale_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(scale_rows));
            }

            if logical.transform.is_configurable() {
                cards.push(section_header("Rotation"));
                let rotations = [
                    rmac_display::Transform::Normal,
                    rmac_display::Transform::Rotate90,
                    rmac_display::Transform::Rotate180,
                    rmac_display::Transform::Rotate270,
                ];
                let rotation_rows = rotations
                    .into_iter()
                    .map(|transform| {
                        let selected = logical.transform == transform;
                        let label = transform.label();
                        let output_id = output.id.clone();
                        let expected_output = output.clone();
                        let rotation_view = view.clone();
                        row_base()
                            .id(SharedString::from(format!(
                                "display-rotation-{output_id}-{label}"
                            )))
                            .child(text_block(label.into(), None))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Transform {
                                            output: expected_output.clone(),
                                            transform: transform.clone(),
                                        };
                                        rotation_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(rotation_rows));
            }

            if output.current_mode().is_some() {
                cards.push(section_header("Resolution"));
                let mode_rows = output
                    .modes
                    .iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        let mode = *mode;
                        let selected = output.current_mode == Some(index);
                        let output_id = output.id.clone();
                        let expected_output = output.clone();
                        let mode_view = view.clone();
                        let subtitle = mode.preferred.then(|| "Preferred".into());
                        row_base()
                            .id(SharedString::from(format!(
                                "display-mode-{output_id}-{index}"
                            )))
                            .child(text_block(mode.label().into(), subtitle))
                            .when(selected, |row| {
                                row.child(glyph("icons/check.svg", 14.0, accent()))
                            })
                            .when(!selected, |row| {
                                row.cursor_pointer()
                                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                                    .on_click(move |_, _, cx| {
                                        let change = DisplayChange::Mode {
                                            output: expected_output.clone(),
                                            mode,
                                        };
                                        mode_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    })
                            })
                            .into_any_element()
                    })
                    .collect();
                cards.push(card(mode_rows));
            }
        }

        if let Some(graphics) = &self.sysinfo.graphics {
            cards.push(section_header("Graphics"));
            cards.push(card(vec![value_row(
                "icons/settings.svg",
                secondary(),
                "Chipset".into(),
                graphics.clone().into(),
            )]));
        }
        if self.display.can_configure {
            if self.display.can_persist {
                cards.push(note_card(
                    "Mode, scale, rotation, and arrangement changes remain temporary until you choose Keep Changes. rmac then validates an owned niri include with the complete live layout before saving it.",
                ));
            } else if let Some(detail) = &self.display.persistence_detail {
                cards.push(note_card(detail.clone()));
            }
        }
        self.pane(cards)
    }
}
