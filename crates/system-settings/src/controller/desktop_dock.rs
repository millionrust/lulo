//! Desktop & Dock pane rendering.

use super::*;

impl Settings {
    pub(super) fn render_desktop_dock(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
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
                    .child("rmac Dock"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("dock-revert", "Revert")
                            .disabled(
                                self.shell_settings_loading
                                    || self.shell_settings_busy
                                    || self.shell_settings_revert.is_none(),
                            )
                            .on_click(move |_, _, cx| {
                                revert_view
                                    .update(cx, |settings, cx| settings.revert_dock_change(cx));
                            }),
                    )
                    .child(
                        Button::new(
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
                        }),
                    ),
            )];

        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the authoritative Dock settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned rmac shell-settings authority is unavailable. No Dock preference can be changed until it is readable again.",
            ));
            return self.pane(cards);
        };
        let dock = &snapshot.settings.dock;
        let enabled = !self.shell_settings_busy;
        let outputs_live =
            self.dock_compositor.connection == rmac_compositor::ConnectionState::Connected;

        cards.push(section_header("Position and visibility"));
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
                Some("Reveal uses deliberate edge pressure so it does not steal focus".into()),
                dock.autohide,
                enabled,
                DockChange::Autohide,
            ),
            dock_switch_row(
                view.clone(),
                "dock-reserve-space",
                "Reserve screen space",
                Some("Keep tiled windows outside the visible Dock area".into()),
                dock.reserve_space,
                enabled,
                DockChange::ReserveSpace,
            ),
        ]));

        cards.push(section_header("Magnification"));
        cards.push(card(vec![
            dock_switch_row(
                view.clone(),
                "dock-magnification",
                "Magnify icons",
                Some("Reduced Motion overrides this effect at runtime".into()),
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

        cards.push(section_header("Application clicks"));
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
        let mut output_rows = vec![dock_output_row(
            &view,
            "all",
            "All displays".into(),
            Some("Follow every enabled niri output".into()),
            dock.outputs == rmac_shell_settings::OutputScope::All,
            enabled,
            rmac_shell_settings::OutputScope::All,
        )];
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
            output_rows.push(dock_output_row(
                &view,
                &format!("named-{output_id}"),
                title.into(),
                Some(format!("niri output {output_id}").into()),
                selected,
                enabled && outputs_live,
                rmac_shell_settings::OutputScope::Named(output_id),
            ));
        }
        cards.push(card(output_rows));

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

        let connection = match self.dock_compositor.connection {
            rmac_compositor::ConnectionState::Connected => "Connected",
            rmac_compositor::ConnectionState::Connecting => "Connecting",
            rmac_compositor::ConnectionState::Reconnecting => "Reconnecting",
            rmac_compositor::ConnectionState::Disconnected => "Unavailable",
        };
        let enabled_outputs = self
            .dock_compositor
            .outputs
            .values()
            .filter(|output| output.enabled())
            .count();
        cards.push(section_header("Authority"));
        cards.push(card(vec![
            value_row(
                "icons/settings.svg",
                accent(),
                "Saved preferences".into(),
                "C4 shell settings".into(),
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "niri event stream".into(),
                connection.into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Enabled outputs".into(),
                enabled_outputs.to_string().into(),
            ),
        ]));
        if snapshot.recovered_from_last_good || snapshot.migrated_from.is_some() {
            cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                snapshot.migrated_from.map_or_else(
                    || "Recovered the last-known-good Dock preferences.".into(),
                    |version| format!("Migrated Dock preferences from version {version}."),
                )
            })));
        }
        if self.dock_compositor.connection != rmac_compositor::ConnectionState::Connected {
            cards.push(note_card(
                "niri is not connected in this process. Output-specific choices are limited to currently known outputs; saved Dock policy remains editable and is applied when the rmac niri session is available.",
            ));
        }
        cards.push(note_card(
            "These controls configure only the original rmac Dock. They do not modify GNOME or third-party docks, and niri continues to own workspace and window-layout rules.",
        ));
        self.pane(cards)
    }
}
