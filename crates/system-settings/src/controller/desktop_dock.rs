//! Desktop & Dock pane rendering, laid out like macOS 26
//! (design-lab/settings.html): a "Dock" section of grouped rows with
//! pop-ups and switches, then the displays the Dock appears on.

use super::*;

impl Settings {
    pub(super) fn render_desktop_dock(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let footer = footer_buttons(vec![
            push_button("dock-revert", "Revert")
                .disabled(
                    self.shell_settings_loading
                        || self.shell_settings_busy
                        || self.shell_settings_revert.is_none(),
                )
                .on_click(move |_, _, cx| {
                    revert_view.update(cx, |settings, cx| settings.revert_dock_change(cx));
                })
                .into_any_element(),
            push_button(
                "dock-refresh",
                if self.shell_settings_busy {
                    "Applying…"
                } else if self.shell_settings_loading {
                    "Loading…"
                } else {
                    "Refresh"
                },
            )
            .disabled(self.shell_settings_loading || self.shell_settings_busy)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| {
                    settings.refresh_shell_settings(false, cx)
                });
            })
            .into_any_element(),
        ]);
        let mut cards = Vec::new();

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative Dock settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. No Dock preference can be changed until it is readable again.",
            ));
            cards.push(footer);
            return self.pane(cards);
        };
        let dock = &snapshot.settings.dock;
        let enabled = !self.shell_settings_busy;
        let outputs_live =
            self.dock_compositor.connection == rmac_compositor::ConnectionState::Connected;

        cards.push(first_section_header("Dock"));
        cards.push(card(vec![
            dock_segment_row(
                view.clone(),
                "dock-placement",
                "Position on screen",
                &DOCK_PLACEMENT_OPTIONS,
                match dock.placement {
                    rmac_shell_settings::DockPlacement::Left => Some(0),
                    rmac_shell_settings::DockPlacement::Bottom => Some(1),
                    rmac_shell_settings::DockPlacement::Right => Some(2),
                },
                enabled,
            ),
            dock_switch_row(
                view.clone(),
                "dock-autohide",
                "Automatically hide and show the Dock",
                None,
                dock.autohide,
                enabled,
                DockChange::Autohide,
            ),
            dock_switch_row(
                view.clone(),
                "dock-reserve-space",
                "Reserve screen space",
                Some("Keep tiled windows outside the visible Dock area.".into()),
                dock.reserve_space,
                enabled,
                DockChange::ReserveSpace,
            ),
        ]));

        cards.push(card(vec![
            dock_switch_row(
                view.clone(),
                "dock-magnification",
                "Magnification",
                Some("Reduce Motion overrides this effect.".into()),
                dock.magnification,
                enabled,
                DockChange::Magnification,
            ),
            dock_segment_row(
                view.clone(),
                "dock-magnification-scale",
                "Maximum size",
                &DOCK_MAGNIFICATION_OPTIONS,
                [1.25_f32, 1.5, 2.0]
                    .iter()
                    .position(|value| (dock.magnification_scale - value).abs() < f32::EPSILON),
                enabled && dock.magnification,
            ),
        ]));
        if ![1.25_f32, 1.5, 2.0]
            .iter()
            .any(|value| (dock.magnification_scale - value).abs() < f32::EPSILON)
        {
            cards.push(note_card(format!(
                "The saved magnification is {:.2}×. Choose a preset to replace it, or leave it unchanged.",
                dock.magnification_scale
            )));
        }

        cards.push(card(vec![dock_segment_row(
            view.clone(),
            "dock-repeated-click",
            "Click a focused app again",
            &DOCK_REPEATED_CLICK_OPTIONS,
            match dock.repeated_click {
                rmac_shell_settings::RepeatedClickBehavior::CycleWindows => Some(0),
                rmac_shell_settings::RepeatedClickBehavior::DoNothing => Some(1),
                rmac_shell_settings::RepeatedClickBehavior::HideApplication => None,
            },
            enabled,
        )]));
        if dock.repeated_click == rmac_shell_settings::RepeatedClickBehavior::HideApplication {
            cards.push(note_card(
                "The saved behavior requests application hiding, but niri has no application-hide action. The Dock reports that action as unavailable; choose Cycle Windows or Do Nothing for supported behavior.",
            ));
        }

        cards.push(section_header("Displays"));
        // The displays the Dock appears on, as one pop-up: every enabled niri
        // output is a choice while niri is connected.
        let mut output_choices: Vec<PopupChoice> = Vec::new();
        let all_view = view.clone();
        output_choices.push(choice(
            "All Displays",
            dock.outputs == rmac_shell_settings::OutputScope::All,
            move |_, cx| {
                all_view.update(cx, |settings, cx| {
                    settings.apply_dock_change(
                        DockChange::Outputs(rmac_shell_settings::OutputScope::All),
                        cx,
                    )
                });
            },
        ));
        if outputs_live {
            for output in self
                .dock_compositor
                .outputs
                .values()
                .filter(|output| output.enabled())
            {
                let output_id = output.id.0.clone();
                let display_name = format!("{} {}", output.make, output.model)
                    .trim()
                    .to_owned();
                let title = if display_name.is_empty() {
                    output_id.clone()
                } else {
                    display_name
                };
                let selected =
                    dock.outputs == rmac_shell_settings::OutputScope::Named(output_id.clone());
                let output_view = view.clone();
                output_choices.push(choice(title, selected, move |_, cx| {
                    let scope = rmac_shell_settings::OutputScope::Named(output_id.clone());
                    output_view.update(cx, |settings, cx| {
                        settings.apply_dock_change(DockChange::Outputs(scope), cx)
                    });
                }));
            }
        }
        let saved_output = match &dock.outputs {
            rmac_shell_settings::OutputScope::All => "All Displays".to_owned(),
            rmac_shell_settings::OutputScope::Primary => "Primary Display".to_owned(),
            rmac_shell_settings::OutputScope::Named(name) => name.clone(),
        };
        let current = popup_value(&output_choices, &saved_output);
        cards.push(card(vec![popup_row(
            "dock-outputs",
            "Show the Dock on",
            None,
            current,
            output_choices,
            enabled,
        )]));

        match &dock.outputs {
            rmac_shell_settings::OutputScope::Primary => cards.push(note_card(
                "Primary output is saved, but the current Dock runtime has no authoritative primary-output source and would create no surface. Choose All displays or a connected niri output.",
            )),
            rmac_shell_settings::OutputScope::Named(name)
                if !self
                    .dock_compositor
                    .outputs
                    .values()
                    .any(|output| output.enabled() && output.id.0 == *name) =>
            {
                cards.push(note_card(format!(
                    "The saved output {name} is not currently enabled in niri. The preference is preserved, but the Dock creates no surface there until it returns."
                )));
            }
            _ => {}
        }

        // macOS keeps these behind a "Hot Corners…" sheet; rmac lists the
        // four pop-ups in the pane. Only actions rmac can perform are
        // offered (rmac_shell_settings::HotCornerAction).
        cards.push(section_header("Hot Corners"));
        let corners = snapshot.settings.hot_corners;
        let mut corner_rows = Vec::new();
        for corner in HotCorner::ALL {
            let current = corner.get(&corners);
            let choices = rmac_shell_settings::HotCornerAction::ALL
                .into_iter()
                .map(|action| {
                    let corner_view = view.clone();
                    choice(action.title(), action == current, move |_, cx| {
                        corner_view.update(cx, |settings, cx| {
                            settings.apply_hot_corner_change(HotCornerChange { corner, action }, cx)
                        });
                    })
                })
                .collect::<Vec<_>>();
            let value = popup_value(&choices, current.title());
            corner_rows.push(popup_row(
                corner.id(),
                corner.title(),
                None,
                value,
                choices,
                enabled,
            ));
        }
        cards.push(card(corner_rows));

        if snapshot.recovered_from_last_good || snapshot.migrated_from.is_some() {
            cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                snapshot.migrated_from.map_or_else(
                    || "Recovered the last-known-good Dock preferences.".into(),
                    |version| format!("Migrated Dock preferences from version {version}."),
                )
            })));
        }
        if !outputs_live {
            cards.push(note_card(
                "niri is not connected in this process. Output-specific choices are limited to currently known outputs; saved Dock policy remains editable and is applied when the rmac niri session is available.",
            ));
        }
        cards.push(footer);
        self.pane(cards)
    }
}
