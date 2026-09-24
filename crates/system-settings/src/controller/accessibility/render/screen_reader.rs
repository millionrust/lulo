//! Accessibility › Screen Reader: the VoiceOver-equivalent on/off switch,
//! backed by GSettings' `screen-reader-enabled` and Orca, plus whether niri
//! and Orca are ready.

use super::*;

impl Settings {
    pub(super) fn accessibility_screen_reader_page(&self, view: Entity<Self>) -> Div {
        let screen_reader = &self.screen_reader;
        let enabled_output = self
            .dock_compositor
            .outputs
            .values()
            .any(rmac_compositor::Output::enabled);
        let prerequisites_present =
            screen_reader.prerequisites_present(enabled_output) && !self.screen_reader_loading;

        let mut body = div().v_flex();

        if self.screen_reader_toggle_loading {
            body = body.child(footnote("Checking the screen reader setting…"));
        } else if let Some(toggle) = self.screen_reader_toggle.clone() {
            let writable = toggle.available
                && toggle.writable
                && prerequisites_present
                && !self.screen_reader_toggle_busy
                && !self.screen_reader_toggle_stream_refreshing;
            let toggle_view = view.clone();
            body = body.child(card(vec![switch_row(
                "screen-reader-enabled",
                "Screen Reader",
                Some(
                    "Announces items onscreen and lets you control Lulo OS from the keyboard, using Orca."
                        .into(),
                ),
                toggle.enabled,
                writable,
                move |value, _, cx| {
                    toggle_view
                        .update(cx, |settings, cx| settings.set_screen_reader_enabled(value, cx));
                },
            )]));
            if let Some(detail) = &toggle.detail {
                body = body.child(footnote(detail.clone()));
            }
        } else {
            body = body.child(note_card(
                "The screen reader setting is unavailable on this system.",
            ));
        }
        if let Some(error) = &self.screen_reader_toggle_error {
            body = body.child(footnote(error.clone()));
        }

        body = body.child(card(vec![fact_row("Shortcut", "⌘F5")]));
        body = body.child(footnote(
            "Super+F5 on a PC keyboard. Lulo OS reacts only to this shortcut and the switch \
             above, not to GNOME's own Super+Alt+S.",
        ));

        if !prerequisites_present {
            if let Some(limitation) = screen_reader.limitation(enabled_output) {
                body = body.child(footnote(limitation));
            }
        }
        body.child(footnote(
            "niri has no desktop zoom or screen curtain, so those controls are not shown.",
        ))
        .child(footer_buttons(vec![push_button(
            "refresh-screen-reader",
            "Refresh",
        )
        .disabled(
            self.screen_reader_loading
                || self.screen_reader_toggle_busy
                || self.screen_reader_toggle_stream_refreshing,
        )
        .on_click(move |_, _, cx| {
            view.update(cx, |settings, cx| {
                settings.refresh_screen_reader(cx);
                settings.refresh_screen_reader_toggle(cx);
            });
        })
        .into_any_element()]))
    }
}
