//! Displays settings presentation.

use super::*;

mod choice;
mod output;

use choice::display_choice_row;

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

        self.append_display_outputs(view, main_output, &mut cards);

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
