//! Desktop & Dock pane rendering, laid out like macOS 26
//! (design-lab/settings.html): a "Dock" section of grouped rows with
//! pop-ups and switches, then the displays the Dock appears on.

use super::*;

impl Settings {
    pub(super) fn render_desktop_dock(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let retry_view = view.clone();
        // Changes save as they are made and the store's live stream keeps the
        // pane current, so, as on the Mac, there is nothing to revert or
        // refresh by hand. Only an unreadable store offers a retry.
        let footer = footer_buttons(vec![push_button("dock-refresh", "Try Again")
            .disabled(self.shell_settings_loading || self.shell_settings_busy)
            .on_click(move |_, _, cx| {
                retry_view.update(cx, |settings, cx| {
                    settings.refresh_shell_settings(false, cx)
                });
            })
            .into_any_element()]);
        let mut cards = Vec::new();

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative Dock settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned Lulo OS shell-settings authority is unavailable. No Dock preference can be changed until it is readable again.",
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

        // DOCK-01: Size, a plain draggable slider with the live value as a
        // tooltip rather than a trailing number, like the Displays pane's
        // Brightness. 32-128 logical px matches the separator's own
        // drag-to-resize range (shell/bins/rmac-dock's DOCK-03).
        cards.push(card(vec![value_slider_row(
            "Size",
            "dock-size",
            &self.dock_size_slider,
            format!("{}", dock.tile_size.round() as i32).into(),
        )]));

        // DOCK-07: Magnification, Off at the slider's minimum then
        // continuous, like the Mac's "Off · Small … Large" -- replacing
        // the on/off switch plus a three-preset picker.
        cards.push(card(vec![value_slider_row(
            "Magnification",
            "dock-magnification",
            &self.dock_magnification_slider,
            if dock.magnification {
                format!("{:.2}×", dock.magnification_scale).into()
            } else {
                SharedString::from("Off")
            },
        )]));

        cards.push(card(vec![
            dock_switch_row(
                view.clone(),
                "dock-show-running-indicators",
                "Show indicators for open applications",
                None,
                dock.show_running_indicators,
                enabled,
                DockChange::ShowRunningIndicators,
            ),
            dock_switch_row(
                view.clone(),
                "dock-show-recent-apps",
                "Show suggested and recent apps in Dock",
                None,
                dock.show_recent_apps,
                enabled,
                DockChange::ShowRecentApps,
            ),
        ]));

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

        // macOS 26.2 puts this pop-up, with the Mac's own wording, under
        // "Desktop & Stage Manager". rmac has no Stage Manager, so its
        // choices are Always and Never (design-lab/reveal-desktop.html).
        cards.push(section_header("Desktop"));
        let reveal = snapshot.settings.click_wallpaper_to_reveal;
        let reveal_choices = rmac_shell_settings::ClickWallpaperToReveal::ALL
            .into_iter()
            .map(|value| {
                let reveal_view = view.clone();
                choice(value.title(), value == reveal, move |_, cx| {
                    reveal_view.update(cx, |settings, cx| {
                        settings.apply_click_wallpaper_to_reveal(value, cx)
                    });
                })
            })
            .collect::<Vec<_>>();
        let reveal_value = popup_value(&reveal_choices, reveal.title());
        cards.push(card(vec![popup_row(
            "click-wallpaper-to-reveal",
            "Click wallpaper to show desktop",
            Some(
                "Click wallpaper to move windows out of the way, revealing your desktop items and widgets."
                    .into(),
            ),
            reveal_value,
            reveal_choices,
            enabled,
        )]));

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
                "niri is not connected in this process. Output-specific choices are limited to currently known outputs; saved Dock policy remains editable and is applied when the Lulo OS niri session is available.",
            ));
        }
        self.pane(cards)
    }
}
