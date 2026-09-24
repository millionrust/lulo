//! System Settings Appearance pane presentation, laid out like macOS 26
//! (design-lab/settings.html): the Appearance thumbnails, then a Theme
//! group with the accent colours and wallpaper tinting, then contrast and
//! motion pop-ups.

use super::*;

/// macOS 26: Appearance thumbnails are 74 × 65 buttons on an 82 pt pitch
/// (a 68 × 44 picture, its label 11 pt below); accent swatches are 24 pt
/// circles in 32 pt hit boxes on a 35.5 pt pitch.
const THUMB_WIDTH: f32 = 68.0;
const THUMB_HEIGHT: f32 = 44.0;
const THUMB_BUTTON: f32 = 74.0;
const THUMB_GAP: f32 = 8.0;
const SWATCH: f32 = 24.0;
const SWATCH_BUTTON: f32 = 32.0;
const SWATCH_GAP: f32 = 3.5;

impl Settings {
    pub(in crate::controller) fn render_appearance(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        // The theme store streams its changes, so the pane stays current on
        // its own; only an unavailable service offers a retry.
        let refresh = footer_buttons(vec![push_button("theme-refresh", "Try Again")
            .disabled(self.theme_loading || self.theme_busy || self.theme_stream_refreshing)
            .busy(self.theme_busy || self.theme_stream_refreshing)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_theme(cx));
            })
            .into_any_element()]);
        let mut cards = Vec::new();
        if self.theme_loading {
            cards.push(note_card("Loading appearance preferences…"));
            return self.pane(cards);
        }
        let Some(theme) = &self.theme else {
            cards.push(note_card(
                "The Lulo OS theme preference service is unavailable.",
            ));
            cards.push(refresh);
            return self.pane(cards);
        };
        let enabled = !self.theme_busy && !self.theme_stream_refreshing;
        let preferences = &theme.preferences;

        let thumbnail = |id: &'static str,
                         name: &'static str,
                         preference: rmac_theme::SchemePreference,
                         picture: Div| {
            let selected = preferences.color_scheme == preference;
            let option_view = view.clone();
            div()
                .id(id)
                .w(px(THUMB_BUTTON))
                .v_flex()
                .items_center()
                .gap(px(5.0))
                .when(enabled, |option| option.cursor_pointer())
                .child(
                    picture
                        .w(px(THUMB_WIDTH))
                        .h(px(THUMB_HEIGHT))
                        .rounded(px(6.0))
                        .overflow_hidden()
                        .when(selected, |picture| {
                            picture.border_2().border_color(accent())
                        })
                        .when(!selected, |picture| picture.border_1().border_color(sep())),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .font_weight(if selected {
                            rmac_ui::mac::BOLD
                        } else {
                            rmac_ui::mac::REGULAR
                        })
                        .text_color(if selected { label() } else { secondary() })
                        .child(name),
                )
                .when(enabled, |option| {
                    option.on_click(move |_, _, cx| {
                        option_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(ThemeChange::Scheme(preference), cx)
                        });
                    })
                })
        };
        let light = || div().bg(hsl(0xe8e8ed));
        let dark = || div().bg(hsl(0x2c2c2e));
        cards.push(card(vec![row_base()
            .items_start()
            .child(text_block("Appearance".into(), None))
            .child(
                div()
                    .flex()
                    .gap(px(THUMB_GAP))
                    .child(thumbnail(
                        "theme-auto",
                        "Auto",
                        rmac_theme::SchemePreference::Automatic,
                        div()
                            .flex()
                            .child(light().flex_1().h_full())
                            .child(dark().flex_1().h_full()),
                    ))
                    .child(thumbnail(
                        "theme-light",
                        "Light",
                        rmac_theme::SchemePreference::Light,
                        light(),
                    ))
                    .child(thumbnail(
                        "theme-dark",
                        "Dark",
                        rmac_theme::SchemePreference::Dark,
                        dark(),
                    )),
            )
            .into_any_element()]));

        // Theme: the accent colours, Multicolour (automatic) first, with the
        // chosen colour's name under the row as on the Mac.
        let automatic_selected =
            preferences.accent_color == rmac_theme::AccentPreference::Automatic;
        let swatch = |id: SharedString, fill: gpui::Background, selected: bool, preference| {
            let swatch_view = view.clone();
            div()
                .id(id)
                .size(px(SWATCH_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(selected, |button| button.border_2().border_color(accent()))
                .when(enabled, |button| button.cursor_pointer())
                .child(div().size(px(SWATCH)).rounded_full().bg(fill))
                .when(enabled, |button| {
                    button.on_click(move |_, _, cx| {
                        swatch_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(ThemeChange::Accent(preference), cx)
                        });
                    })
                })
        };
        let mut swatches = div().flex().gap(px(SWATCH_GAP)).child(swatch(
            "theme-accent-auto".into(),
            gpui::linear_gradient(
                135.0,
                gpui::linear_color_stop(hsl(0xff375f), 0.0),
                gpui::linear_color_stop(hsl(0x0a84ff), 1.0),
            ),
            automatic_selected,
            rmac_theme::AccentPreference::Automatic,
        ));
        let mut selected_name: SharedString = "Multicolour".into();
        for (index, (name, hex)) in ACCENTS.iter().copied().enumerate() {
            let preference = accent_preference(hex);
            let selected = preferences.accent_color == preference;
            if selected {
                selected_name = name.into();
            }
            swatches = swatches.child(swatch(
                SharedString::from(format!("theme-accent-{index}")),
                hsl(hex).into(),
                selected,
                preference,
            ));
        }
        let tint_view = view.clone();
        cards.push(section_header("Theme"));
        cards.push(card(vec![row_base()
            .items_start()
            .child(text_block("Colour".into(), None))
            .child(
                div()
                    .v_flex()
                    .items_end()
                    .gap(px(3.0))
                    .child(swatches)
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .line_height(px(14.0))
                            .text_color(secondary())
                            .child(selected_name),
                    ),
            )
            .into_any_element()]));
        // macOS 26 files the tint switch under Windows, a plain 37 pt row.
        cards.push(section_header("Windows"));
        cards.push(card(vec![row_base()
            .child(text_block(
                "Tint window background with wallpaper colour".into(),
                None,
            ))
            .child(
                Toggle::new("theme-wallpaper-tinting")
                    .checked(preferences.allow_wallpaper_tinting)
                    .disabled(!enabled)
                    .on_click(move |value, _, cx| {
                        tint_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(ThemeChange::WallpaperTinting(*value), cx)
                        });
                    }),
            )
            .into_any_element()]));
        cards.push(section_header("Accessibility"));
        cards.push(card(vec![
            theme_segment_row(
                view.clone(),
                "theme-contrast",
                "Contrast",
                &THEME_CONTRAST_OPTIONS,
                match preferences.contrast {
                    rmac_theme::ContrastPreference::Automatic => 0,
                    rmac_theme::ContrastPreference::Normal => 1,
                    rmac_theme::ContrastPreference::Higher => 2,
                },
                enabled,
            ),
            theme_segment_row(
                view,
                "theme-motion",
                "Motion",
                &THEME_MOTION_OPTIONS,
                match preferences.motion {
                    rmac_theme::MotionPreferenceSetting::Automatic => 0,
                    rmac_theme::MotionPreferenceSetting::Full => 1,
                    rmac_theme::MotionPreferenceSetting::Reduced => 2,
                },
                enabled,
            ),
        ]));

        if let Some(detail) = theme.detail.clone() {
            cards.push(note_card(detail));
        }
        if !self.host_appearance.available {
            cards.push(note_card(
                "The Linux Settings portal is unavailable here. Automatic values use safe Lulo OS defaults; explicit choices remain writable.",
            ));
        }
        if self.theme_error.is_some()
            || self.theme_store_stream_error.is_some()
            || self.theme_portal_stream_error.is_some()
        {
            cards.push(refresh);
        }
        self.pane(cards)
    }
}
