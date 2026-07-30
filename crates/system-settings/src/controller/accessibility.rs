//! Accessibility settings presentation.

use super::*;

impl Settings {
    pub(super) fn render_accessibility(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let gtk_refresh_view = view.clone();
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/accessibility.svg", accent(), 22.0))
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
            .child(tile("icons/app-window.svg", secondary(), 22.0))
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

        let keyboard_view = view.clone();
        let mouse_view = view.clone();
        let screen_reader_refresh_view = view.clone();
        let trackpad_view = view;
        cards.push(section_header("Motor"));
        cards.push(card(vec![
            row_base()
                .child(tile("icons/keyboard.svg", secondary(), 22.0))
                .child(text_block(
                    "Keyboard".into(),
                    Some("Repeat timing and layout controls backed by niri".into()),
                ))
                .child(
                    Button::new("accessibility-keyboard", "Open").on_click(move |_, _, cx| {
                        keyboard_view
                            .update(cx, |settings, cx| settings.select_category("Keyboard", cx));
                    }),
                )
                .into_any_element(),
            row_base()
                .child(tile("icons/mouse.svg", secondary(), 22.0))
                .child(text_block(
                    "Pointer".into(),
                    Some("Speed, acceleration, handedness, and scroll controls".into()),
                ))
                .child(
                    Button::new("accessibility-mouse", "Mouse").on_click(move |_, _, cx| {
                        mouse_view.update(cx, |settings, cx| settings.select_category("Mouse", cx));
                    }),
                )
                .child(Button::new("accessibility-trackpad", "Trackpad").on_click(
                    move |_, _, cx| {
                        trackpad_view
                            .update(cx, |settings, cx| settings.select_category("Trackpad", cx));
                    },
                ))
                .into_any_element(),
        ]));

        if !self.input_loading {
            let keyboard = &self.input.settings.keyboard;
            let selected_preset = KEYBOARD_RESPONSE_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::KeyboardRepeatPreset { delay_ms, rate }
                        if *delay_ms == keyboard.repeat_delay_ms && *rate == keyboard.repeat_rate
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-key-response",
                    "Key repeat preset",
                    &KEYBOARD_RESPONSE_PRESETS,
                    selected_preset,
                    self.input.can_configure && !self.input_busy,
                ),
                value_row(
                    "icons/keyboard.svg",
                    secondary(),
                    "Effective key repeat".into(),
                    format!(
                        "{} ms delay · {} characters/s",
                        keyboard.repeat_delay_ms, keyboard.repeat_rate
                    )
                    .into(),
                ),
            ]));
            let mouse = &self.input.settings.mouse;
            let mouse_writable = self.input.can_configure && mouse.enabled && !self.input_busy;
            let selected_pointer_preset = MOUSE_PRECISION_PRESETS.iter().position(|(_, change)| {
                matches!(
                    change,
                    InputChange::MousePrecisionPreset { speed, profile }
                        if *speed == mouse.accel_speed && *profile == mouse.accel_profile
                )
            });
            cards.push(card(vec![
                input_segment_row(
                    cx.entity(),
                    "accessibility-pointer-precision",
                    "Mouse precision",
                    &MOUSE_PRECISION_PRESETS,
                    selected_pointer_preset,
                    mouse_writable,
                ),
                input_switch_row(
                    cx.entity(),
                    "accessibility-middle-emulation",
                    "icons/mouse.svg",
                    "Middle-button emulation",
                    Some("Press the left and right mouse buttons together"),
                    mouse.middle_emulation,
                    mouse_writable,
                    InputChange::MouseMiddleEmulation,
                ),
                value_row(
                    "icons/mouse.svg",
                    secondary(),
                    "Effective mouse response".into(),
                    format!(
                        "{} acceleration · speed {}",
                        mouse.accel_profile.label(),
                        mouse.accel_speed
                    )
                    .into(),
                ),
            ]));
            if let Some(detail) = self
                .input
                .detail
                .clone()
                .filter(|_| !self.input.can_configure)
            {
                cards.push(note_card(detail));
            }
        } else {
            cards.push(note_card(
                "Loading keyboard accessibility settings from niri…",
            ));
        }
        cards.push(note_card(
            "Niri currently provides repeat timing but no compositor authority for Sticky Keys, Slow Keys, or Bounce Keys. Those controls remain unavailable instead of being simulated inside individual apps.",
        ));
        cards.push(note_card(
            "Mouse precision and middle-button emulation are applied by niri through libinput. Niri does not currently provide Mouse Keys, dwell click, or a session-wide double-click timing authority, so those controls remain unavailable.",
        ));

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
        self.pane(cards)
    }
    pub(super) fn finish_gtk_text_update(
        &mut self,
        result: std::result::Result<rmac_gtk_settings::Snapshot, rmac_gtk_settings::Error>,
    ) {
        self.gtk_text_loading = false;
        self.gtk_text_busy = false;
        match result {
            Ok(snapshot) => {
                self.gtk_text = Some(snapshot);
                self.gtk_text_error = None;
            }
            Err(error) => {
                self.gtk_text_error =
                    Some(format!("Could not update GTK text scaling: {error}").into());
            }
        }
    }

    pub(super) fn queue_gtk_text_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            self.gtk_text_refresh_pending = true;
            return;
        }
        self.gtk_text_refresh_pending = false;
        self.gtk_text_stream_refreshing = true;
        let generation = self.gtk_text_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.gtk_text_stream_refreshing = false;
                if gtk_text_stream_snapshot_is_current(
                    generation,
                    this.gtk_text_generation,
                    this.gtk_text_loading,
                    this.gtk_text_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.gtk_text = Some(snapshot);
                            this.gtk_text_error = None;
                        }
                        Err(_) => {
                            this.gtk_text_error =
                                Some("Could not refresh changed GTK text scaling".into());
                        }
                    }
                } else {
                    this.gtk_text_refresh_pending = true;
                }
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_gtk_text_refresh(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_refresh_pending
            && !self.gtk_text_loading
            && !self.gtk_text_busy
            && !self.gtk_text_stream_refreshing
        {
            self.queue_gtk_text_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_gtk_text(&mut self, cx: &mut Context<Self>) {
        if self.gtk_text_loading || self.gtk_text_busy || self.gtk_text_stream_refreshing {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_gtk_text_scale(&mut self, factor: f64, cx: &mut Context<Self>) {
        if self.gtk_text_loading
            || self.gtk_text_busy
            || self.gtk_text_stream_refreshing
            || !self
                .gtk_text
                .as_ref()
                .is_some_and(|snapshot| snapshot.available && snapshot.writable)
        {
            return;
        }
        self.gtk_text_busy = true;
        self.gtk_text_generation = self.gtk_text_generation.wrapping_add(1);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_gtk_settings::set_text_scale(factor) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
