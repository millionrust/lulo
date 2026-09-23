//! Accessibility settings presentation, laid out like macOS 26: a header
//! card, then Vision and Motor lists whose rows open section pages
//! (design-lab/settings.html).

use super::*;

mod motor;
mod screen_reader;

const SCREEN_READER: &str = "Screen Reader";
const DISPLAY: &str = "Display";
const MOTION: &str = "Motion";
const POINTER_CONTROL: &str = "Pointer Control";

fn page_row(
    view: &Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    page: &'static str,
) -> AnyElement {
    let open_view = view.clone();
    icon_nav_row(
        SharedString::from(format!("accessibility-page-{page}")),
        tile(icon, color, style::ROW_ICON).into_any_element(),
        page,
        None,
        move |_, cx| {
            open_view.update(cx, |settings, cx| {
                settings.push(
                    SubPage::AccessibilityPage {
                        page: page.to_owned(),
                    },
                    cx,
                );
            });
        },
    )
}

impl Settings {
    pub(in crate::controller) fn render_accessibility(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let keyboard_view = view.clone();
        let cards = vec![
            header_card(
                tile26("icons/accessibility.svg", accent()),
                "Accessibility",
                "Personalise rmac in ways that work best for you with accessibility features for vision and motor.",
                None,
            ),
            section_header("Vision"),
            card(vec![
                page_row(&view, "icons/accessibility.svg", hsl(0x1d1d1f), SCREEN_READER),
                page_row(&view, "icons/monitor.svg", accent(), DISPLAY),
                page_row(&view, "icons/sparkles.svg", hsl(0x34c759), MOTION),
            ]),
            section_header("Motor"),
            card(vec![
                icon_nav_row(
                    "accessibility-keyboard",
                    tile("icons/keyboard.svg", hsl(0x8e8e93), style::ROW_ICON).into_any_element(),
                    "Keyboard",
                    None,
                    move |_, cx| {
                        keyboard_view
                            .update(cx, |settings, cx| settings.select_category("Keyboard", cx));
                    },
                ),
                page_row(&view, "icons/mouse.svg", hsl(0x8e8e93), POINTER_CONTROL),
            ]),
        ];
        self.pane(cards)
    }

    /// Accessibility › Display, Motion, Screen Reader or Pointer Control.
    pub(in crate::controller) fn accessibility_page_body(
        &self,
        page: &str,
        cx: &Context<Self>,
    ) -> Div {
        let view = cx.entity();
        match page {
            DISPLAY => self.accessibility_display_page(view),
            MOTION => self.accessibility_motion_page(view),
            SCREEN_READER => self.accessibility_screen_reader_page(view),
            POINTER_CONTROL => self.accessibility_pointer_page(cx),
            _ => note_card("This Accessibility page is not available."),
        }
    }

    fn theme_refresh_footer(&self, view: Entity<Self>) -> Div {
        footer_buttons(vec![push_button("accessibility-refresh", "Refresh")
            .disabled(
                self.theme_loading
                    || self.theme_busy
                    || self.theme_stream_refreshing
                    || self.gtk_text_loading
                    || self.gtk_text_busy
                    || self.gtk_text_stream_refreshing,
            )
            .on_click(move |_, _, cx| {
                view.update(cx, |settings, cx| {
                    settings.refresh_theme(cx);
                    settings.refresh_gtk_text(cx);
                });
            })
            .into_any_element()])
    }

    /// Display: contrast, then a Text section with the rmac and GTK text
    /// sizes, as the Mac's Display page groups them.
    fn accessibility_display_page(&self, view: Entity<Self>) -> Div {
        let mut body = div().v_flex();
        let theme_enabled = !self.theme_busy && !self.theme_stream_refreshing;
        let mut text_rows = Vec::new();
        if self.theme_loading {
            body = body.child(footnote("Loading accessibility preferences…"));
        } else if let Some(theme) = &self.theme {
            let preferences = &theme.preferences;
            body = body.child(card(vec![theme_segment_row(
                view.clone(),
                "accessibility-contrast",
                "Contrast",
                &THEME_CONTRAST_OPTIONS,
                match preferences.contrast {
                    rmac_theme::ContrastPreference::Automatic => 0,
                    rmac_theme::ContrastPreference::Normal => 1,
                    rmac_theme::ContrastPreference::Higher => 2,
                },
                theme_enabled,
            )]));
            text_rows.push(theme_segment_row(
                view.clone(),
                "accessibility-text-scale",
                "Text size",
                &THEME_TEXT_SCALE_OPTIONS,
                match preferences.text_scale {
                    rmac_theme::TextScalePreference::Standard => 0,
                    rmac_theme::TextScalePreference::Large => 1,
                    rmac_theme::TextScalePreference::ExtraLarge => 2,
                },
                theme_enabled,
            ));
        } else {
            body = body.child(note_card(
                "The rmac visual accessibility preference service is unavailable.",
            ));
        }
        let mut gtk_note = None;
        if let Some(snapshot) = self.gtk_text.as_ref().filter(|_| !self.gtk_text_loading) {
            if snapshot.available {
                let selected = GTK_TEXT_SCALE_OPTIONS
                    .iter()
                    .position(|(_, factor)| (snapshot.factor - factor).abs() < 0.001);
                text_rows.push(gtk_text_scale_row(
                    view.clone(),
                    selected,
                    snapshot.writable && !self.gtk_text_busy && !self.gtk_text_stream_refreshing,
                ));
                gtk_note = snapshot.detail.clone();
            } else {
                gtk_note = Some(snapshot.detail.clone().unwrap_or_else(|| {
                    "The GNOME interface text-scaling authority is unavailable.".into()
                }));
            }
        }
        if !text_rows.is_empty() {
            body = body
                .child(section_header("Text"))
                .child(card(text_rows))
                .child(footnote(
                    "Text size applies to rmac's own interface text; GTK application text changes GNOME applications. Neither changes display scaling.",
                ));
        }
        if let Some(note) = gtk_note {
            body = body.child(footnote(note));
        }
        body.child(self.theme_refresh_footer(view))
    }

    /// Motion: the rmac motion preference.
    fn accessibility_motion_page(&self, view: Entity<Self>) -> Div {
        let mut body = div().v_flex();
        if self.theme_loading {
            body = body.child(footnote("Loading accessibility preferences…"));
        } else if let Some(theme) = &self.theme {
            body = body.child(card(vec![theme_segment_row(
                view.clone(),
                "accessibility-motion",
                "Motion",
                &THEME_MOTION_OPTIONS,
                match theme.preferences.motion {
                    rmac_theme::MotionPreferenceSetting::Automatic => 0,
                    rmac_theme::MotionPreferenceSetting::Full => 1,
                    rmac_theme::MotionPreferenceSetting::Reduced => 2,
                },
                !self.theme_busy && !self.theme_stream_refreshing,
            )]));
        } else {
            body = body.child(note_card(
                "The rmac visual accessibility preference service is unavailable.",
            ));
        }
        body.child(self.theme_refresh_footer(view))
    }
}
