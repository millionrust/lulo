//! Keyboard, Mouse, and Trackpad settings presentation, on macOS 26's
//! layout (design-lab/settings.html): stepped sliders with tick marks for
//! the niri presets, switches for the libinput options, Trackpad's tab bar,
//! and Refresh under the last group.

use super::*;

/// The picked preset of a stepped slider, applied through niri.
fn input_pick(view: &Entity<Settings>, options: &'static [InputOption]) -> IndexHandler {
    let view = view.clone();
    Rc::new(move |index: usize, _: &mut Window, cx: &mut App| {
        if let Some((_, change)) = options.get(index) {
            let change = *change;
            view.update(cx, |settings, cx| settings.apply_input_change(change, cx));
        }
    })
}

/// A switch that applies `change(value)` through niri.
fn input_switch(
    view: &Entity<Settings>,
    id: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    enabled: bool,
    change: fn(bool) -> InputChange,
) -> AnyElement {
    let view = view.clone();
    switch_row(
        id,
        title,
        subtitle.map(Into::into),
        checked,
        enabled,
        move |value, _, cx| {
            view.update(cx, |settings, cx| {
                settings.apply_input_change(change(value), cx)
            });
        },
    )
}

/// One of Keyboard's twin sliders: the 13 pt label over a 210 pt slider.
fn twin_slider(title: &'static str, slider: Div) -> Div {
    div()
        .v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .text_color(label())
                .child(title),
        )
        .child(slider)
}

/// A [`twin_slider`]'s track: `rmac_ui`'s own `Slider` (SET-101 -- the
/// hand-rolled stepped track it replaces here drew with no thumb when the
/// system's real value didn't land exactly on a preset, and Delay's ran
/// past the card's right edge), with the preset range's end labels below
/// it the way the stepped track's tick labels were.
fn twin_value_slider(
    state: &Entity<SliderState>,
    start_label: &'static str,
    end_label: &'static str,
    enabled: bool,
) -> Div {
    div()
        .w(px(style::TWIN_SLIDER_WIDTH))
        .flex_none()
        .v_flex()
        .gap(px(4.0))
        .child(
            Slider::new(state)
                .disabled(!enabled)
                .w(px(style::TWIN_SLIDER_WIDTH)),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(14.0))
                .text_color(label())
                .child(start_label)
                .child(end_label),
        )
}

fn mouse_acceleration(on: bool) -> InputChange {
    InputChange::MouseAccelProfile(if on {
        rmac_input::AccelProfile::Adaptive
    } else {
        rmac_input::AccelProfile::Flat
    })
}

fn touchpad_acceleration(on: bool) -> InputChange {
    InputChange::TouchpadAccelProfile(if on {
        rmac_input::AccelProfile::Adaptive
    } else {
        rmac_input::AccelProfile::Flat
    })
}

impl Settings {
    pub(in crate::controller) fn render_keyboard(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.keyboard;
        let writable = self.input.can_configure && !self.input_busy;
        cards.push(
            group().child(
                div()
                    .flex()
                    .gap(px(style::TWIN_SLIDER_GAP))
                    .p(px(style::ROW_PADDING))
                    .child(twin_slider(
                        "Key repeat rate",
                        twin_value_slider(
                            &self.keyboard_repeat_rate_slider,
                            "Slow",
                            "Fast",
                            writable,
                        ),
                    ))
                    .child(twin_slider(
                        "Delay until repeat",
                        twin_value_slider(
                            &self.keyboard_repeat_delay_slider,
                            "Long",
                            "Short",
                            writable,
                        ),
                    )),
            ),
        );
        let shortcuts_view = view.clone();
        cards.push(card(vec![
            input_switch(
                &view,
                "keyboard-numlock",
                "Use Num Lock on startup",
                None,
                settings.numlock,
                writable,
                InputChange::KeyboardNumlock,
            ),
            button_row(vec![push_button(
                "keyboard-shortcuts",
                "Keyboard Shortcuts…",
            )
            .on_click(move |_, _, cx| {
                shortcuts_view.update(cx, |settings, cx| settings.open_keyboard_shortcuts(cx));
            })
            .into_any_element()]),
        ]));

        cards.extend(self.mac_keyboard_cards(cx));

        // Text Input: the system layouts localed reports, edited on Language
        // & Region where their editor lives.
        cards.push(section_header("Text Input"));
        let sources_view = view.clone();
        cards.push(card(vec![value_button_row(
            "Input Sources",
            None,
            Some(
                self.locale
                    .as_ref()
                    .map(locale_keyboard_summary)
                    .unwrap_or_else(|| "Not reported".to_owned())
                    .into(),
            ),
            Some(
                push_button("keyboard-input-sources", "Edit…")
                    .on_click(move |_, window, cx| {
                        sources_view.update(cx, |settings, cx| {
                            settings.select_category("Language & Region", window, cx);
                        });
                    })
                    .into_any_element(),
            ),
        )]));
        cards.push(self.input_refresh_button(cx));
        self.pane(cards)
    }

    pub(in crate::controller) fn render_mouse(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.mouse;
        let writable = self.input.can_configure && settings.enabled && !self.input_busy;
        if !settings.enabled {
            cards.push(note_card(
                "Mouse settings are turned off in the niri configuration, so they do not affect any mouse.",
            ));
        }
        cards.push(card(vec![
            stepped_slider_row(
                "mouse-tracking",
                "Tracking speed",
                MOUSE_SPEEDS.len(),
                Some(speed_index(settings.accel_speed)),
                "Slow",
                "Fast",
                writable,
                input_pick(&view, &MOUSE_SPEEDS),
            ),
            input_switch(
                &view,
                "mouse-natural-scroll",
                "Natural scrolling",
                Some("Content tracks finger movement"),
                settings.natural_scroll,
                writable,
                InputChange::MouseNaturalScroll,
            ),
            input_switch(
                &view,
                "mouse-acceleration",
                "Pointer acceleration",
                None,
                settings.accel_profile == rmac_input::AccelProfile::Adaptive,
                writable,
                mouse_acceleration,
            ),
            input_switch(
                &view,
                "mouse-left-handed",
                "Primary button on right",
                None,
                settings.left_handed,
                writable,
                InputChange::MouseLeftHanded,
            ),
            input_switch(
                &view,
                "mouse-middle-emulation",
                "Middle-click emulation",
                Some("Click the left and right buttons together"),
                settings.middle_emulation,
                writable,
                InputChange::MouseMiddleEmulation,
            ),
        ]));
        cards.push(self.input_refresh_button(cx));
        self.pane(cards)
    }

    pub(in crate::controller) fn render_trackpad(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let tab_view = view.clone();
        let mut cards = vec![tab_bar(
            "trackpad-tab",
            &["Point & Click", "Scroll & Zoom"],
            self.trackpad_tab,
            Rc::new(move |index: usize, _: &mut Window, cx: &mut App| {
                tab_view.update(cx, |settings, cx| {
                    settings.trackpad_tab = index;
                    cx.notify();
                });
            }),
        )];
        if let Some(note) = self.input_unavailable_card() {
            cards.push(note);
        }
        let settings = &self.input.settings.touchpad;
        let writable = self.input.can_configure && settings.pointer.enabled && !self.input_busy;
        if !settings.pointer.enabled {
            cards.push(note_card(
                "Trackpad settings are turned off in the niri configuration, so they do not affect any trackpad.",
            ));
        }
        let rows = if self.trackpad_tab == 1 {
            vec![input_switch(
                &view,
                "touchpad-natural-scroll",
                "Natural scrolling",
                Some("Content tracks finger movement"),
                settings.pointer.natural_scroll,
                writable,
                InputChange::TouchpadNaturalScroll,
            )]
        } else {
            vec![
                stepped_slider_row(
                    "touchpad-tracking",
                    "Tracking speed",
                    TOUCHPAD_SPEEDS.len(),
                    Some(speed_index(settings.pointer.accel_speed)),
                    "Slow",
                    "Fast",
                    writable,
                    input_pick(&view, &TOUCHPAD_SPEEDS),
                ),
                input_switch(
                    &view,
                    "touchpad-acceleration",
                    "Pointer acceleration",
                    None,
                    settings.pointer.accel_profile == rmac_input::AccelProfile::Adaptive,
                    writable,
                    touchpad_acceleration,
                ),
                input_switch(
                    &view,
                    "touchpad-tap",
                    "Tap to click",
                    Some("Tap with one finger"),
                    settings.tap_to_click,
                    writable,
                    InputChange::TouchpadTap,
                ),
                input_switch(
                    &view,
                    "touchpad-dwt",
                    "Ignore trackpad while typing",
                    None,
                    settings.disable_while_typing,
                    writable,
                    InputChange::TouchpadDwt,
                ),
                input_switch(
                    &view,
                    "touchpad-drag-lock",
                    "Drag lock",
                    Some("Keep dragging briefly after lifting your finger"),
                    settings.drag_lock,
                    writable,
                    InputChange::TouchpadDragLock,
                ),
                {
                    let secondary_click_view = view.clone();
                    let secondary_click_choices: Vec<PopupChoice> = [
                        rmac_input::SecondaryClick::TwoFingerClickOrTap,
                        rmac_input::SecondaryClick::CornerClick,
                    ]
                    .into_iter()
                    .map(|value| {
                        let selected = settings.secondary_click == value;
                        let choice_view = secondary_click_view.clone();
                        choice(value.label(), selected, move |_, cx| {
                            if !selected {
                                choice_view.update(cx, |settings, cx| {
                                    settings.apply_input_change(
                                        InputChange::TouchpadSecondaryClick(value),
                                        cx,
                                    )
                                });
                            }
                        })
                    })
                    .collect();
                    let secondary_click_current =
                        popup_value(&secondary_click_choices, settings.secondary_click.label());
                    popup_row(
                        "touchpad-secondary-click",
                        "Secondary click",
                        None,
                        secondary_click_current,
                        secondary_click_choices,
                        writable,
                    )
                },
                input_switch(
                    &view,
                    "touchpad-left-handed",
                    "Primary click on right",
                    None,
                    settings.pointer.left_handed,
                    writable,
                    InputChange::TouchpadLeftHanded,
                ),
                input_switch(
                    &view,
                    "touchpad-middle-emulation",
                    "Middle-click emulation",
                    Some("Click the left and right sides together"),
                    settings.pointer.middle_emulation,
                    writable,
                    InputChange::TouchpadMiddleEmulation,
                ),
            ]
        };
        cards.push(card(rows));
        cards.push(self.input_refresh_button(cx));
        self.pane(cards)
    }
}
