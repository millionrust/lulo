//! Per-output display configuration, laid out like macOS 26
//! (design-lab/settings.html): one group of pop-ups (resolution, scale,
//! rotation, arrangement) per display, then its read-only facts.

use super::*;

fn fact_row(title: &'static str, value: SharedString) -> AnyElement {
    row_base()
        .child(text_block(title.into(), None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

impl Settings {
    pub(super) fn append_display_outputs(
        &self,
        view: Entity<Self>,
        main_output: Option<rmac_display::Output>,
        cards: &mut Vec<Div>,
    ) {
        let several = self.display.outputs.len() > 1;
        let locked = self.display_busy || self.display_confirmation.is_some();
        for output in &self.display.outputs {
            if several {
                cards.push(section_header(if output.primary {
                    format!("{} (Main)", output.name)
                } else {
                    output.name.clone()
                }));
            }
            let configurable = self.display.can_configure && !locked;
            let mut controls = Vec::new();

            if let Some(logical) = output.logical.as_ref() {
                if output.current_mode().is_some() {
                    let choices: Vec<PopupChoice> = output
                        .modes
                        .iter()
                        .enumerate()
                        .map(|(index, mode)| {
                            let mode = *mode;
                            let selected = output.current_mode == Some(index);
                            let expected_output = output.clone();
                            let mode_view = view.clone();
                            let label = if mode.preferred {
                                format!("{} (Preferred)", mode.label())
                            } else {
                                mode.label()
                            };
                            choice(label, selected, move |_, cx| {
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
                        })
                        .collect();
                    let current = output
                        .current_mode()
                        .map(|mode| mode.label())
                        .unwrap_or_default();
                    controls.push(popup_row(
                        SharedString::from(format!("display-mode-{}", output.id)),
                        "Resolution",
                        None,
                        current.into(),
                        choices,
                        configurable,
                    ));
                }

                if (0.5..=4.0).contains(&logical.scale) {
                    let choices: Vec<PopupChoice> = [1.0, 1.25, 1.5, 1.75, 2.0]
                        .into_iter()
                        .map(|scale| {
                            let selected = (logical.scale - scale).abs() < 0.001;
                            let expected_output = output.clone();
                            let scale_view = view.clone();
                            choice(
                                format!("{}%", (scale * 100.0) as u32),
                                selected,
                                move |_, cx| {
                                    if !selected {
                                        let change = DisplayChange::Scale {
                                            output: expected_output.clone(),
                                            scale,
                                        };
                                        scale_view.update(cx, |settings, cx| {
                                            settings.apply_display_change(change, cx)
                                        });
                                    }
                                },
                            )
                        })
                        .collect();
                    let current = format!("{}%", (logical.scale * 100.0).round() as u32);
                    controls.push(popup_row(
                        SharedString::from(format!("display-scale-{}", output.id)),
                        "Scale",
                        None,
                        current.into(),
                        choices,
                        configurable,
                    ));
                }

                if logical.transform.is_configurable() {
                    let choices: Vec<PopupChoice> = [
                        rmac_display::Transform::Normal,
                        rmac_display::Transform::Rotate90,
                        rmac_display::Transform::Rotate180,
                        rmac_display::Transform::Rotate270,
                    ]
                    .into_iter()
                    .map(|transform| {
                        let selected = logical.transform == transform;
                        let label = transform.label();
                        let expected_output = output.clone();
                        let rotation_view = view.clone();
                        choice(label, selected, move |_, cx| {
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
                    })
                    .collect();
                    controls.push(popup_row(
                        SharedString::from(format!("display-rotation-{}", output.id)),
                        "Rotation",
                        None,
                        logical.transform.label().into(),
                        choices,
                        configurable,
                    ));
                }

                if output.primary {
                    if let Some(percentage) = self.brightness {
                        controls.push(value_slider_row(
                            "Brightness",
                            SharedString::from(format!("display-brightness-{}", output.id)),
                            &self.brightness_slider,
                            format!("{percentage}%").into(),
                        ));
                    }
                }

                if !output.primary {
                    if let Some(anchor) =
                        main_output.as_ref().and_then(|main| main.logical.as_ref())
                    {
                        let choices: Vec<PopupChoice> = [
                            DisplayPlacement::Left,
                            DisplayPlacement::Right,
                            DisplayPlacement::Above,
                            DisplayPlacement::Below,
                        ]
                        .into_iter()
                        .filter_map(|placement| {
                            let (x, y) = relative_display_position(logical, anchor, placement)?;
                            let selected = logical.x == x && logical.y == y;
                            let expected_output = output.clone();
                            let placement_view = view.clone();
                            Some(choice(placement.label(), selected, move |_, cx| {
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
                            }))
                        })
                        .collect();
                        let current = popup_value(&choices, "Custom");
                        controls.push(popup_row(
                            SharedString::from(format!("display-position-{}", output.id)),
                            "Position",
                            None,
                            current,
                            choices,
                            configurable,
                        ));
                    }
                }
            }
            if !controls.is_empty() {
                cards.push(card(controls));
            }

            let mut facts = Vec::new();
            if let Some(detail) = &output.detail {
                facts.push(fact_row("Type", detail.clone().into()));
            }
            facts.push(fact_row("Connector", output.connector.clone().into()));
            match &output.logical {
                Some(logical) => {
                    facts.push(fact_row(
                        "Logical Size",
                        format!("{} × {}", logical.width, logical.height).into(),
                    ));
                    if several {
                        facts.push(fact_row(
                            "Position",
                            format!("{}, {}", logical.x, logical.y).into(),
                        ));
                    }
                }
                None => facts.push(fact_row("Status", "Disabled".into())),
            }
            if let Some((width, height)) = output.physical_size_mm {
                facts.push(fact_row(
                    "Physical Size",
                    format!("{width} × {height} mm").into(),
                ));
            }
            cards.push(card(facts));

            if !output.primary && output.logical.is_some() && self.display.can_persist {
                let output_id = output.id.clone();
                let primary_view = view.clone();
                cards.push(footer_buttons(vec![push_button(
                    SharedString::from(format!("display-primary-{output_id}")),
                    "Use as Main Display",
                )
                .disabled(locked)
                .on_click(move |_, _, cx| {
                    primary_view.update(cx, |settings, cx| {
                        settings.set_primary_display(output_id.clone(), cx)
                    });
                })
                .into_any_element()]));
            }
        }
    }
}
