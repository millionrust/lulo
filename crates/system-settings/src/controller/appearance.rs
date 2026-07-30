//! Appearance settings lifecycle and presentation.

use super::*;

impl Settings {
    pub(super) fn finish_theme_update(&mut self, result: std::result::Result<ThemeLoad, String>) {
        self.theme_loading = false;
        self.theme_busy = false;
        match result {
            Ok(load) => {
                self.host_appearance = load.host;
                self.theme = Some(load.theme);
                self.theme_error = None;
            }
            Err(error) => {
                self.theme_error = Some(format!("Could not update Appearance: {error}").into());
            }
        }
    }
    pub(super) fn refresh_theme(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_theme_change(&mut self, change: ThemeChange, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            return;
        }
        let Some(theme) = &self.theme else {
            return;
        };
        let expected = theme.preferences.clone();
        self.theme_generation = self.theme_generation.wrapping_add(1);
        self.theme_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(apply_theme_change_authoritatively(change, expected))
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn queue_theme_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_loading || self.theme_busy || self.theme_stream_refreshing {
            self.theme_refresh_pending = true;
            return;
        }
        self.theme_refresh_pending = false;
        self.theme_stream_refreshing = true;
        let generation = self.theme_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.theme_stream_refreshing = false;
                if theme_stream_snapshot_is_current(
                    generation,
                    this.theme_generation,
                    this.theme_loading,
                    this.theme_busy,
                ) {
                    match result {
                        Ok(load) => this.finish_theme_update(Ok(load)),
                        Err(_) => {
                            this.theme_error =
                                Some("Could not refresh changed appearance preferences".into());
                        }
                    }
                } else {
                    this.theme_refresh_pending = true;
                }
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_theme_refresh(&mut self, cx: &mut Context<Self>) {
        if self.theme_refresh_pending
            && !self.theme_loading
            && !self.theme_busy
            && !self.theme_stream_refreshing
        {
            self.queue_theme_stream_refresh(cx);
        }
    }
    pub(super) fn render_appearance(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
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
                    .child("rmac Appearance"),
            )
            .child(
                div()
                    .id("theme-refresh")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(accent())
                    .cursor_pointer()
                    .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    .child(if self.theme_busy || self.theme_stream_refreshing {
                        "Applying…"
                    } else {
                        "Refresh"
                    })
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_theme(cx));
                    }),
            )];
        if self.theme_loading {
            cards.push(note_card("Loading appearance preferences…"));
            return self.pane(cards);
        }
        let Some(theme) = &self.theme else {
            cards.push(note_card(
                "The rmac theme preference service is unavailable.",
            ));
            return self.pane(cards);
        };
        let enabled = !self.theme_busy && !self.theme_stream_refreshing;
        let preferences = &theme.preferences;
        let scheme_card = {
            let option = |id: &'static str,
                          name: &'static str,
                          preference: rmac_theme::SchemePreference,
                          swatch: Hsla| {
                let selected = preferences.color_scheme == preference;
                let option_view = view.clone();
                div()
                    .id(ElementId::from(id))
                    .v_flex()
                    .items_center()
                    .gap_1p5()
                    .when(enabled, |element| {
                        element.cursor_pointer().on_click(move |_, _, cx| {
                            option_view.update(cx, |settings, cx| {
                                settings.apply_theme_change(ThemeChange::Scheme(preference), cx)
                            });
                        })
                    })
                    .when(!enabled, |element| element.opacity(0.55))
                    .child(
                        div()
                            .w(px(64.0))
                            .h(px(40.0))
                            .rounded(px(6.0))
                            .bg(swatch)
                            .border_2()
                            .border_color(if selected { accent() } else { sep() }),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(if selected { accent() } else { label() })
                            .child(name),
                    )
            };
            div()
                .flex()
                .gap_5()
                .justify_center()
                .p_4()
                .rounded(px(10.0))
                .mb_3()
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(option(
                    "theme-light",
                    "Light",
                    rmac_theme::SchemePreference::Light,
                    hsl(0xf5f5f7),
                ))
                .child(option(
                    "theme-dark",
                    "Dark",
                    rmac_theme::SchemePreference::Dark,
                    hsl(0x2c2c2e),
                ))
                .child(option(
                    "theme-auto",
                    "Automatic",
                    rmac_theme::SchemePreference::Automatic,
                    hsl(0x8e8e93),
                ))
        };
        cards.push(scheme_card);

        let mut swatches = Vec::new();
        let automatic_selected =
            preferences.accent_color == rmac_theme::AccentPreference::Automatic;
        let auto_view = view.clone();
        swatches.push(
            div()
                .id("theme-accent-auto")
                .h(px(24.0))
                .px_2()
                .rounded(px(6.0))
                .flex()
                .items_center()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(if automatic_selected {
                    on_accent()
                } else {
                    label()
                })
                .bg(if automatic_selected {
                    accent()
                } else {
                    rmac_ui::mac::control_fill()
                })
                .when(enabled, |element| {
                    element.cursor_pointer().on_click(move |_, _, cx| {
                        auto_view.update(cx, |settings, cx| {
                            settings.apply_theme_change(
                                ThemeChange::Accent(rmac_theme::AccentPreference::Automatic),
                                cx,
                            )
                        });
                    })
                })
                .when(!enabled, |element| element.opacity(0.55))
                .child("Automatic")
                .into_any_element(),
        );
        for (index, (name, hex)) in ACCENTS.iter().copied().enumerate() {
            let preference = accent_preference(hex);
            let selected = preferences.accent_color == preference;
            let swatch_foreground = swatch_foreground(hex);
            let swatch_view = view.clone();
            swatches.push(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "theme-accent-{index}"
                    ))))
                    .w(px(48.0))
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .when(enabled, |element| {
                        element.cursor_pointer().on_click(move |_, _, cx| {
                            swatch_view.update(cx, |settings, cx| {
                                settings.apply_theme_change(ThemeChange::Accent(preference), cx)
                            });
                        })
                    })
                    .when(!enabled, |element| element.opacity(0.55))
                    .child(
                        div()
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded_full()
                            .bg(hsl(hex))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |element| {
                                element
                                    .border_2()
                                    .border_color(swatch_foreground)
                                    .shadow_sm()
                                    .child(glyph("icons/check.svg", 12.0, swatch_foreground))
                            }),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(secondary())
                            .child(name),
                    )
                    .into_any_element(),
            );
        }
        cards.push(
            div()
                .v_flex()
                .mb_3()
                .rounded(px(10.0))
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(label_row("Accent color", None))
                .child(div().h(px(1.0)).bg(sep()).mx_3())
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .flex_wrap()
                        .p_3()
                        .children(swatches),
                ),
        );
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

        let host_scheme = if self.host_appearance.capabilities.color_scheme {
            self.host_appearance.color_scheme.label()
        } else {
            "Not exposed"
        };
        cards.push(section_header("Authority"));
        cards.push(card(vec![
            value_row(
                "icons/info.svg",
                secondary(),
                "Host preference".into(),
                host_scheme.into(),
            ),
            value_row(
                "icons/palette.svg",
                accent(),
                "Effective appearance".into(),
                match theme.effective.color_scheme {
                    rmac_appearance::ResolvedColorScheme::Light => "Light".into(),
                    rmac_appearance::ResolvedColorScheme::Dark => "Dark".into(),
                },
            ),
        ]));
        if let Some(detail) = theme.detail.clone() {
            cards.push(note_card(detail));
        }
        if !self.host_appearance.available {
            cards.push(note_card(
                "The Linux Settings portal is unavailable here. Automatic values use safe rmac defaults; explicit choices remain writable.",
            ));
        }
        self.pane(cards)
    }
}
