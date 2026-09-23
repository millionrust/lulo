//! Accessibility › Screen Reader: whether niri and Orca are ready, with the
//! notes that explain anything missing.

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
        let status = if self.screen_reader_loading {
            "Checking…"
        } else if prerequisites_present {
            "Ready"
        } else {
            "Not ready"
        };
        let mut body = div().v_flex().child(card(vec![
            fact_row("Orca", status),
            fact_row("Shortcut", "Super–Alt–S"),
            fact_row(
                "Orca installed",
                if screen_reader.orca_installed {
                    "Yes"
                } else {
                    "No"
                },
            ),
            fact_row(
                "X11 display for Orca",
                if screen_reader.x11_display {
                    "Available"
                } else if screen_reader.xwayland_satellite_installed {
                    "xwayland-satellite installed"
                } else {
                    "Unavailable"
                },
            ),
        ]));
        if !self.screen_reader_loading {
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
        .disabled(self.screen_reader_loading)
        .on_click(move |_, _, cx| {
            view.update(cx, |settings, cx| settings.refresh_screen_reader(cx));
        })
        .into_any_element()]))
    }
}
