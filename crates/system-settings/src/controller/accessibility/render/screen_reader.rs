//! Accessibility text and screen-reader presentation.

use super::*;

impl Settings {
    pub(super) fn append_accessibility_screen_reader(
        &self,
        view: Entity<Self>,
        cards: &mut Vec<Div>,
    ) {
        let screen_reader_refresh_view = view;
        cards.push(section_header("Text & Screen Reader"));
        cards.push(note_card(
            "Text size applies live to shared controls and app-owned interface text across the current rmac apps. It does not change GTK, browser, editor or terminal content fonts, display scaling, or compositor scaling.",
        ));
        let screen_reader = &self.screen_reader;
        let enabled_output = self
            .dock_compositor
            .outputs
            .values()
            .any(rmac_compositor::Output::enabled);
        let prerequisites_present =
            screen_reader.prerequisites_present(enabled_output) && !self.screen_reader_loading;
        cards.push(card(vec![
            row_base()
                .child(tile(
                    "icons/accessibility.svg",
                    if prerequisites_present {
                        hsl(0x34c759)
                    } else {
                        secondary()
                    },
                    22.0,
                ))
                .child(text_block(
                    "Niri/Orca prerequisites".into(),
                    Some(if self.screen_reader_loading {
                        "Checking…".into()
                    } else if prerequisites_present {
                        "Detected".into()
                    } else {
                        "Incomplete".into()
                    }),
                ))
                .child(
                    Button::new("refresh-screen-reader", "Refresh")
                        .busy(self.screen_reader_loading)
                        .disabled(self.screen_reader_loading)
                        .on_click(move |_, _, cx| {
                            screen_reader_refresh_view
                                .update(cx, |settings, cx| settings.refresh_screen_reader(cx));
                        }),
                )
                .into_any_element(),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Desktop session".into(),
                if screen_reader.niri_session {
                    "Full niri session".into()
                } else {
                    "Not a full niri session".into()
                },
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Enabled display".into(),
                if enabled_output {
                    "Detected from niri".into()
                } else {
                    "Not detected".into()
                },
            ),
            value_row(
                "icons/monitor.svg",
                secondary(),
                "Xwayland display for Orca".into(),
                if screen_reader.x11_display {
                    "DISPLAY exported".into()
                } else {
                    "Unavailable".into()
                },
            ),
            value_row(
                "icons/settings.svg",
                secondary(),
                "xwayland-satellite".into(),
                if screen_reader.xwayland_satellite_installed {
                    "Executable in PATH".into()
                } else if screen_reader.x11_display {
                    "Custom X11 path exported".into()
                } else {
                    "Not found in PATH".into()
                },
            ),
            value_row(
                "icons/accessibility.svg",
                secondary(),
                "Orca".into(),
                if screen_reader.orca_installed {
                    "Installed".into()
                } else {
                    "Not found in PATH".into()
                },
            ),
            value_row(
                "icons/keyboard.svg",
                accent(),
                "Niri default shortcut".into(),
                "Super–Alt–S".into(),
            ),
        ]));
        if self.screen_reader_loading {
            cards.push(note_card("Checking niri and Orca prerequisites…"));
        } else if let Some(limitation) = screen_reader.limitation(enabled_output) {
            cards.push(note_card(limitation));
        } else {
            cards.push(note_card(
                "The detectable prerequisites are present. Environment checks cannot prove working EGL, speech output, the configured shortcut, or application semantics; test all four on the Linux PC.",
            ));
        }
        cards.push(note_card(
            "Niri does not currently provide built-in desktop zoom or a screen curtain. Those controls remain unavailable instead of being simulated by rmac.",
        ));
        cards.push(note_card(
            "This readiness check covers niri and Orca only. rmac application roles, names, states, actions, focus, and announcements still require Linux AT-SPI/Orca runtime evidence before accessibility can be claimed.",
        ));
    }
}
