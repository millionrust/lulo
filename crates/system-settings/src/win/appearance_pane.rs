//! Appearance on Windows: Lulo's light, dark or automatic appearance and its
//! accent colour, laid out like Lulo OS's pane (macOS 26 measurements).
//!
//! The choice is saved in the theme store every Lulo app reads
//! (`rmac-theme`); `rmac-ui` watches that file, so open Lulo apps, this
//! window included, change as soon as it is written. Automatic follows
//! Windows' own "Choose your app mode" (`rmac-appearance-portal`).

use gpui::{
    div, prelude::FluentBuilder as _, px, Context, Div, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
};
use rmac_ui::StyledExt as _;

use super::form::{card, label, note_card, row_base, secondary, section_header, text_block};
use super::{hex, WinSettings};
use crate::appearance::{accent_preference, ThemeChange, ACCENTS};

/// macOS 26: the thumbnails are 74 pt buttons holding a 68 × 44 picture;
/// accent swatches are 24 pt circles in 32 pt hit boxes.
const THUMB_WIDTH: f32 = 68.0;
const THUMB_HEIGHT: f32 = 44.0;
const THUMB_BUTTON: f32 = 74.0;
const THUMB_GAP: f32 = 8.0;
const THUMB_RADIUS: f32 = 6.0;
const SWATCH: f32 = 24.0;
const SWATCH_BUTTON: f32 = 32.0;
const SWATCH_GAP: f32 = 3.5;

impl WinSettings {
    pub(super) fn load_theme(&mut self, cx: &mut Context<Self>) {
        let task = cx
            .background_executor()
            .spawn(async { crate::appearance::load_theme_state().await });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                this.theme = Some(loaded.map_err(SharedString::from));
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_theme(&mut self, change: ThemeChange, cx: &mut Context<Self>) {
        let Some(Ok(loaded)) = &self.theme else {
            return;
        };
        if self.theme_busy {
            return;
        }
        let expected = loaded.theme.preferences.clone();
        self.theme_busy = true;
        self.theme_error = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            crate::appearance::apply_theme_change_authoritatively(change, expected).await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.theme_busy = false;
                match result {
                    Ok(loaded) => this.theme = Some(Ok(loaded)),
                    Err(error) => {
                        this.theme_error = Some(error.into());
                        // Something else changed the file; show what is
                        // there now.
                        this.load_theme(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_appearance(&self, cx: &mut Context<Self>) -> Div {
        let mut cards = Vec::new();
        let preferences = match &self.theme {
            Some(Ok(loaded)) => loaded.theme.preferences.clone(),
            _ => rmac_theme::Preferences::default(),
        };
        match &self.theme {
            None => cards.push(note_card("Reading the appearance Lulo apps use…")),
            Some(Err(error)) => cards.push(note_card(format!(
                "Lulo's appearance preferences could not be read: {error}."
            ))),
            Some(Ok(_)) => {}
        }
        let enabled = matches!(self.theme, Some(Ok(_))) && !self.theme_busy;
        let view = cx.entity();

        let thumbnail = |id: &'static str,
                         name: &'static str,
                         preference: rmac_theme::SchemePreference,
                         picture: Div| {
            let selected = preferences.color_scheme == preference;
            let view = view.clone();
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
                        .rounded(px(THUMB_RADIUS))
                        .overflow_hidden()
                        .when(selected, |picture| {
                            picture.border_2().border_color(rmac_ui::mac::accent())
                        })
                        .when(!selected, |picture| {
                            picture
                                .border_1()
                                .border_color(super::style::group_separator())
                        }),
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
                        view.update(cx, |settings, cx| {
                            settings.apply_theme(ThemeChange::Scheme(preference), cx)
                        });
                    })
                })
        };
        let light = || div().bg(hex(0xe8e8ed));
        let dark = || div().bg(hex(0x2c2c2e));
        cards.push(card(vec![row_base()
            .items_start()
            .child(text_block("Appearance", None))
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

        // Theme ▸ Colour: Multicolour (automatic) first, then the eight
        // accents, with the chosen colour's name under them.
        let swatch = |id: SharedString,
                      fill: gpui::Background,
                      selected: bool,
                      preference: rmac_theme::AccentPreference| {
            let view = view.clone();
            div()
                .id(id)
                .size(px(SWATCH_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(selected, |button| {
                    button.border_2().border_color(rmac_ui::mac::accent())
                })
                .when(enabled, |button| button.cursor_pointer())
                .child(div().size(px(SWATCH)).rounded_full().bg(fill))
                .when(enabled, |button| {
                    button.on_click(move |_, _, cx| {
                        view.update(cx, |settings, cx| {
                            settings.apply_theme(ThemeChange::Accent(preference), cx)
                        });
                    })
                })
        };
        let automatic = preferences.accent_color == rmac_theme::AccentPreference::Automatic;
        let mut swatches = div().flex().gap(px(SWATCH_GAP)).child(swatch(
            "theme-accent-auto".into(),
            gpui::linear_gradient(
                135.0,
                gpui::linear_color_stop(hex(0xff375f), 0.0),
                gpui::linear_color_stop(hex(0x0a84ff), 1.0),
            ),
            automatic,
            rmac_theme::AccentPreference::Automatic,
        ));
        let mut selected_name = SharedString::from("Multicolour");
        for (index, (name, value)) in ACCENTS.iter().copied().enumerate() {
            let preference = accent_preference(value);
            let selected = preferences.accent_color == preference;
            if selected {
                selected_name = name.into();
            }
            swatches = swatches.child(swatch(
                SharedString::from(format!("theme-accent-{index}")),
                hex(value).into(),
                selected,
                preference,
            ));
        }
        cards.push(section_header("Theme"));
        cards.push(card(vec![row_base()
            .items_start()
            .child(text_block("Colour", None))
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
        cards.push(super::form::footnote(
            "Lulo apps on this PC use this appearance. Windows' own apps and \
             windows keep the appearance chosen in Windows Settings.",
        ));
        if let Some(error) = &self.theme_error {
            cards.push(note_card(error.clone()));
        }
        div().v_flex().children(cards)
    }
}
