//! Displays settings presentation, laid out like macOS 26
//! (design-lab/settings.html): a full-width well with the display
//! arrangement and names, then each display's pop-ups and facts.

use super::*;

mod output;

impl Settings {
    pub(in crate::controller) fn render_displays(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let revert_view = view.clone();
        let keep_view = view.clone();
        let mut cards = Vec::new();
        let footer = footer_buttons(vec![push_button("display-refresh", "Refresh")
            .busy(self.display_busy)
            .disabled(
                self.display_loading || self.display_busy || self.display_confirmation.is_some(),
            )
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_displays(cx));
            })
            .into_any_element()]);
        if self.display_loading {
            cards.push(note_card("Loading displays from the compositor…"));
            return self.pane(cards);
        }
        if !self.display.available {
            cards.push(note_card(
                "The display service is not available in this desktop session.",
            ));
            cards.push(footer);
            return self.pane(cards);
        }
        if self.display.outputs.is_empty() {
            cards.push(note_card("No displays were detected."));
        } else {
            cards.push(display_layout_preview(&self.display.outputs));
        }

        // A pending change must be kept or reverted before the timer runs
        // out, as with the Mac's "Keep these display settings?" prompt.
        if let Some(pending) = self.display_confirmation.as_ref() {
            let seconds = pending.seconds_remaining;
            cards.push(card(vec![row_base()
                .child(text_block(
                    "Keep these display settings?".into(),
                    Some(format!("Reverting in {seconds} seconds.").into()),
                ))
                .child(
                    push_button("display-revert", "Revert")
                        .disabled(
                            self.display_loading
                                || self.display_busy
                                || !self.display.can_configure,
                        )
                        .on_click(move |_, _, cx| {
                            revert_view
                                .update(cx, |settings, cx| settings.revert_display_change(cx));
                        }),
                )
                .child(
                    Button::new("display-keep", "Keep Changes")
                        .primary()
                        .disabled(
                            self.display_loading || self.display_busy || !self.display.can_persist,
                        )
                        .on_click(move |_, _, cx| {
                            keep_view.update(cx, |settings, cx| settings.keep_display_change(cx));
                        }),
                )
                .into_any_element()]));
        }

        let enabled_outputs = self
            .display
            .outputs
            .iter()
            .filter(|output| output.logical.is_some())
            .count();
        if enabled_outputs > 1 && !self.display.mirror_supported {
            cards.push(footnote(
                "niri does not provide native display mirroring. Displays remain extended; wl-mirror can mirror content without pretending it is a compositor layout mode.",
            ));
        }
        let main_output = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .cloned();

        self.append_display_outputs(view, main_output, &mut cards);

        if let Some(graphics) = &self.sysinfo.graphics {
            cards.push(card(vec![row_base()
                .child(text_block("Graphics".into(), None))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child(graphics.clone()),
                )
                .into_any_element()]));
        }
        cards.push(footer);
        if self.display.can_configure {
            if self.display.can_persist {
                cards.push(footnote(
                    "Resolution, scale, rotation and arrangement changes stay temporary until you choose Keep Changes; rmac then saves the complete live layout to its niri include.",
                ));
            } else if let Some(detail) = &self.display.persistence_detail {
                cards.push(note_card(detail.clone()));
            }
        }
        self.pane(cards)
    }
}
