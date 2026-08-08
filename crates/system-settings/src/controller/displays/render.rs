//! Displays settings presentation.

use super::*;

fn display_choice_row(
    id: SharedString,
    title: SharedString,
    subtitle: Option<SharedString>,
    selected: bool,
    disabled: bool,
) -> ListRow {
    let has_subtitle = subtitle.is_some();
    let foreground = if selected { on_accent() } else { label() };
    let secondary_foreground = if selected { on_accent() } else { secondary() };
    let mut text = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(foreground)
            .child(title),
    );
    if let Some(subtitle) = subtitle {
        text = text.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary_foreground)
                .child(subtitle),
        );
    }
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .child(text)
        .when(selected, |row| {
            row.child(glyph("icons/check.svg", 14.0, on_accent()))
        });
    ListRow::new(ElementId::from(id), content)
        .selected(selected)
        .disabled(disabled)
        .h(px(if has_subtitle { 60.0 } else { 44.0 }))
        .px_3()
}

impl Settings {
    pub(in crate::controller) fn render_displays(&self, cx: &Context<Self>) -> Div {
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
                                Button::new("display-keep", format!("Keep Changes ({seconds}s)"))
                                    .primary()
                                    .disabled(
                                        self.display_loading
                                            || self.display_busy
                                            || !self.display.can_persist,
                                    )
                                    .on_click(move |_, _, cx| {
                                        keep_view.update(cx, |settings, cx| {
                                            settings.keep_display_change(cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new("display-revert", "Revert")
                                    .destructive()
                                    .disabled(
                                        self.display_loading
                                            || self.display_busy
                                            || !self.display.can_configure,
                                    )
                                    .on_click(move |_, _, cx| {
                                        revert_view.update(cx, |settings, cx| {
                                            settings.revert_display_change(cx)
                                        });
                                    }),
                            )
                    })
                    .child(
                        Button::new("display-refresh", "Refresh")
                            .ghost()
                            .busy(self.display_busy)
                            .disabled(
                                self.display_loading
                                    || self.display_busy
                                    || self.display_confirmation.is_some(),
                            )
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
                    ListRow::new(
                        SharedString::from(format!("display-primary-{output_id}")),
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .child(text_block(
                                "Use as Main Display".into(),
                                Some("Anchors rmac shell surfaces and niri startup focus".into()),
                            ))
                            .child(glyph("icons/chevron-right.svg", 14.0, secondary())),
                    )
                    .disabled(self.display_busy || self.display_confirmation.is_some())
                    .h(px(60.0))
                    .px_3()
                    .on_activate(move |_, _, cx| {
                        primary_view.update(cx, |settings, cx| {
                            settings.set_primary_display(output_id.clone(), cx)
                        });
                    })
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
                                display_choice_row(
                                    SharedString::from(format!(
                                        "display-position-{output_id}-{}",
                                        placement.label()
                                    )),
                                    placement.label().into(),
                                    None,
                                    selected,
                                    self.display_busy || self.display_confirmation.is_some(),
                                )
                                .on_activate(move |_, _, cx| {
                                    if !selected {
                                        let change = DisplayChange::Position {
                                            output: expected_output.clone(),
                                            x,
                                            y,
                                        };
                                        placement_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    }
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
                        display_choice_row(
                            SharedString::from(format!("display-scale-{output_id}-{scale}")),
                            format!("{}%", (scale * 100.0) as u32).into(),
                            None,
                            selected,
                            self.display_busy || self.display_confirmation.is_some(),
                        )
                        .on_activate(move |_, _, cx| {
                            if !selected {
                                let change = DisplayChange::Scale {
                                    output: expected_output.clone(),
                                    scale,
                                };
                                scale_view.update(cx, |settings, cx| {
                                    settings.apply_display_change(change, cx)
                                });
                            }
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
                        display_choice_row(
                            SharedString::from(format!("display-rotation-{output_id}-{label}")),
                            label.into(),
                            None,
                            selected,
                            self.display_busy || self.display_confirmation.is_some(),
                        )
                        .on_activate(move |_, _, cx| {
                            if !selected {
                                let change = DisplayChange::Transform {
                                    output: expected_output.clone(),
                                    transform: transform.clone(),
                                };
                                rotation_view.update(cx, |settings, cx| {
                                    settings.apply_display_change(change, cx)
                                });
                            }
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
                        display_choice_row(
                            SharedString::from(format!("display-mode-{output_id}-{index}")),
                            mode.label().into(),
                            subtitle,
                            selected,
                            self.display_busy || self.display_confirmation.is_some(),
                        )
                        .on_activate(move |_, _, cx| {
                            if !selected {
                                let change = DisplayChange::Mode {
                                    output: expected_output.clone(),
                                    mode,
                                };
                                mode_view.update(cx, |settings, cx| {
                                    settings.apply_display_change(change, cx)
                                });
                            }
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
