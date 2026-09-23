//! Accessibility settings presentation.

use super::*;

mod motor;
mod screen_reader;

impl Settings {
    pub(in crate::controller) fn render_accessibility(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let gtk_refresh_view = view.clone();
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/accessibility.svg", accent(), style::ROW_ICON))
            .child(text_block(
                "Visual preferences".into(),
                Some("Live across rmac apps and shell surfaces".into()),
            ))
            .child(
                Button::new("accessibility-refresh", "Refresh")
                    .busy(self.theme_busy || self.theme_stream_refreshing)
                    .disabled(self.theme_loading || self.theme_busy || self.theme_stream_refreshing)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_theme(cx));
                    }),
            )
            .into_any_element()])];
        if self.theme_loading {
            cards.push(note_card("Loading accessibility preferences…"));
            return self.pane(cards);
        }
        if let Some(theme) = &self.theme {
            let preferences = &theme.preferences;
            cards.push(section_header("Vision"));
            cards.push(card(vec![
                theme_segment_row(
                    view.clone(),
                    "accessibility-contrast",
                    "Display contrast",
                    &THEME_CONTRAST_OPTIONS,
                    match preferences.contrast {
                        rmac_theme::ContrastPreference::Automatic => 0,
                        rmac_theme::ContrastPreference::Normal => 1,
                        rmac_theme::ContrastPreference::Higher => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                theme_segment_row(
                    view.clone(),
                    "accessibility-motion",
                    "Interface motion",
                    &THEME_MOTION_OPTIONS,
                    match preferences.motion {
                        rmac_theme::MotionPreferenceSetting::Automatic => 0,
                        rmac_theme::MotionPreferenceSetting::Full => 1,
                        rmac_theme::MotionPreferenceSetting::Reduced => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                theme_segment_row(
                    view.clone(),
                    "accessibility-text-scale",
                    "Text size",
                    &THEME_TEXT_SCALE_OPTIONS,
                    match preferences.text_scale {
                        rmac_theme::TextScalePreference::Standard => 0,
                        rmac_theme::TextScalePreference::Large => 1,
                        rmac_theme::TextScalePreference::ExtraLarge => 2,
                    },
                    !self.theme_busy && !self.theme_stream_refreshing,
                ),
                value_row(
                    "icons/info.svg",
                    secondary(),
                    "Effective visual mode".into(),
                    format!(
                        "{} contrast · {} motion · {}% text",
                        match theme.effective.contrast {
                            rmac_appearance::Contrast::Normal => "Normal",
                            rmac_appearance::Contrast::Higher => "Higher",
                        },
                        match theme.effective.motion {
                            rmac_appearance::MotionPreference::Full => "Full",
                            rmac_appearance::MotionPreference::Reduced => "Reduced",
                        },
                        (theme.effective.text_scale.factor() * 100.0).round() as u16,
                    )
                    .into(),
                ),
            ]));
        } else {
            cards.push(note_card(
                "The rmac visual accessibility preference service is unavailable.",
            ));
        }

        cards.push(section_header("GTK Application Text"));
        cards.push(card(vec![row_base()
            .child(tile("icons/app-window.svg", secondary(), style::ROW_ICON))
            .child(text_block(
                "GTK text scaling".into(),
                Some("GNOME interface authority; separate from rmac and display scale".into()),
            ))
            .child(
                Button::new("gtk-text-refresh", "Refresh")
                    .busy(self.gtk_text_busy || self.gtk_text_stream_refreshing)
                    .disabled(
                        self.gtk_text_loading
                            || self.gtk_text_busy
                            || self.gtk_text_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        gtk_refresh_view.update(cx, |settings, cx| settings.refresh_gtk_text(cx));
                    }),
            )
            .into_any_element()]));
        if self.gtk_text_loading {
            cards.push(note_card("Loading GTK text scaling from GSettings…"));
        } else if let Some(snapshot) = &self.gtk_text {
            if snapshot.available {
                let selected = GTK_TEXT_SCALE_OPTIONS
                    .iter()
                    .position(|(_, factor)| (snapshot.factor - factor).abs() < 0.001);
                cards.push(card(vec![
                    gtk_text_scale_row(
                        view.clone(),
                        selected,
                        snapshot.writable
                            && !self.gtk_text_busy
                            && !self.gtk_text_stream_refreshing,
                    ),
                    value_row(
                        "icons/app-window.svg",
                        secondary(),
                        "Effective GTK text".into(),
                        format!("{}%", (snapshot.factor * 100.0).round() as u16).into(),
                    ),
                ]));
                if let Some(detail) = &snapshot.detail {
                    cards.push(note_card(detail.clone()));
                }
            } else {
                cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                    "The GNOME interface text-scaling authority is unavailable.".into()
                })));
            }
        }

        self.append_accessibility_motor(view.clone(), cx, &mut cards);
        self.append_accessibility_screen_reader(view, &mut cards);
        self.pane(cards)
    }
}
