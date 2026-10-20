//! Menu Bar pane, laid out like macOS 26 (design-lab/settings.html): the
//! "Menu Bar Controls" list of status items, each with a tickbox, coloured
//! icon and name, and the clock's seconds. Every row writes the versioned
//! shell-settings store that `rmac-shell-status` and the menu bar read.

use super::*;

/// Menu Bar Controls rows are 43 pt apart on the Mac (42 + separator).
const CONTROL_ROW_HEIGHT: f32 = 42.0;

impl Settings {
    pub(super) fn render_menu_bar(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let mut cards = Vec::new();
        if self.shell_settings_loading && self.shell_settings.is_none() {
            cards.push(note_card("Loading the menu bar settings…"));
            return self.pane(cards);
        }
        let Some(snapshot) = self.shell_settings.as_ref() else {
            cards.push(note_card(
                "The versioned Lulo OS shell-settings authority is unavailable. Menu bar items stay as they are until it is readable again.",
            ));
            return self.pane(cards);
        };
        let settings = &snapshot.settings;
        let indicators = &settings.indicators;
        let enabled = !self.shell_settings_busy;

        let control = |id: &'static str,
                       icon: &'static str,
                       color: Hsla,
                       title: &'static str,
                       checked: bool,
                       change: fn(bool) -> MenuBarChange| {
            let control_view = view.clone();
            row_base()
                .min_h(px(CONTROL_ROW_HEIGHT))
                .gap(px(12.0))
                .child(
                    Checkbox::new(id)
                        .checked(checked)
                        .disabled(!enabled)
                        .on_change(move |value, _, cx| {
                            control_view.update(cx, |settings, cx| {
                                settings.apply_menu_bar_change(change(*value), cx)
                            });
                        }),
                )
                .child(tile(icon, color, style::HEADER_ICON))
                .child(text_block(title.into(), None))
                .into_any_element()
        };
        cards.push(section_header("Menu Bar Controls"));
        let mut controls = Vec::new();
        // The Mac lists the clock first, with its options behind a button.
        let clock_view = view.clone();
        controls.push(
            row_base()
                .min_h(px(CONTROL_ROW_HEIGHT))
                .child(text_block("Clock".into(), None))
                .child(
                    push_button("menu-bar-clock-options", "Clock Options…")
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            clock_view.update(cx, |settings, cx| {
                                settings.set_menu_bar_sheet(Some(MenuBarSheet::Clock), cx)
                            });
                        }),
                )
                .into_any_element(),
        );
        if self.hardware.has_wifi {
            controls.push(control(
                "menu-bar-network",
                "icons/wifi.svg",
                accent(),
                "Wi-Fi",
                indicators.network,
                MenuBarChange::Network,
            ));
        }
        // Each control only where the bar can show its item: on Windows
        // the Lulo bar reads Wi-Fi, Bluetooth, sound and the battery from
        // Windows; VPN, Focus and Notifications are Lulo OS services.
        if self.hardware.has_bluetooth {
            controls.push(control(
                "menu-bar-bluetooth",
                "icons/bluetooth.svg",
                accent(),
                "Bluetooth",
                indicators.bluetooth,
                MenuBarChange::Bluetooth,
            ));
        }
        if crate::navigation::platform_supports("VPN") {
            controls.push(control(
                "menu-bar-vpn",
                "icons/key.svg",
                accent(),
                "VPN",
                indicators.vpn,
                MenuBarChange::Vpn,
            ));
        }
        if self.hardware.has_battery {
            let battery_view = view.clone();
            controls.push(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().child(control(
                        "menu-bar-battery",
                        "icons/battery-charging.svg",
                        hsl(0x34c759),
                        "Battery",
                        indicators.power,
                        MenuBarChange::Power,
                    )))
                    .child(
                        div().pr(px(style::ROW_PADDING)).child(
                            push_button("menu-bar-battery-options", "Battery Options…")
                                .disabled(!enabled || !indicators.power)
                                .on_click(move |_, _, cx| {
                                    battery_view.update(cx, |settings, cx| {
                                        settings.set_menu_bar_sheet(Some(MenuBarSheet::Battery), cx)
                                    });
                                }),
                        ),
                    )
                    .into_any_element(),
            );
        }
        if crate::navigation::platform_supports("Focus") {
            controls.push(control(
                "menu-bar-focus",
                "icons/moon.svg",
                hsl(0x5e5ce6),
                "Focus",
                indicators.focus,
                MenuBarChange::Focus,
            ));
        }
        controls.push(control(
            "menu-bar-sound",
            "icons/volume-2.svg",
            hsl(0xff2d55),
            "Sound",
            indicators.sound,
            MenuBarChange::Sound,
        ));
        if crate::navigation::platform_supports("Notifications") {
            controls.push(control(
                "menu-bar-notifications",
                "icons/bell.svg",
                hsl(0xff3b30),
                "Notifications",
                indicators.notifications,
                MenuBarChange::Notifications,
            ));
        }
        cards.push(card(controls));

        self.pane(cards)
    }

    pub(super) fn set_menu_bar_sheet(
        &mut self,
        sheet: Option<MenuBarSheet>,
        cx: &mut Context<Self>,
    ) {
        if self.menu_bar_sheet != sheet {
            self.menu_bar_sheet = sheet;
            cx.notify();
        }
    }

    /// Menu Bar ▸ Clock Options… and Battery Options…, as on macOS 26.
    pub(super) fn render_menu_bar_sheet(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let sheet = self.menu_bar_sheet?;
        let snapshot = self.shell_settings.as_ref()?;
        let view = cx.entity();
        let enabled = !self.shell_settings_busy;
        let settings = &snapshot.settings;
        let switch = |id: &'static str,
                      title: &'static str,
                      checked: bool,
                      disabled: bool,
                      change: fn(bool) -> MenuBarChange| {
            let switch_view = view.clone();
            row_base()
                .child(text_block(title.into(), None))
                .child(
                    Toggle::new(id)
                        .checked(checked)
                        .disabled(!enabled || disabled)
                        .on_click(move |value, _, cx| {
                            switch_view.update(cx, |settings, cx| {
                                settings.apply_menu_bar_change(change(*value), cx)
                            });
                        }),
                )
                .into_any_element()
        };
        let heading = |title: &'static str| {
            div()
                .mb(px(12.0))
                .text_size(rmac_ui::text_px(13.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(label())
                .child(title)
        };
        let (id, height, body) = match sheet {
            MenuBarSheet::Clock => {
                let clock = &settings.clock;
                let twenty_four = clock.format == rmac_shell_settings::ClockFormat::TwentyFourHour;
                let date_choices: Vec<PopupChoice> = [(true, "Always"), (false, "Never")]
                    .into_iter()
                    .map(|(value, title)| {
                        let date_view = view.clone();
                        let selected = clock.show_date == value;
                        choice(title, selected, move |_, cx| {
                            if !selected {
                                date_view.update(cx, |settings, cx| {
                                    settings
                                        .apply_menu_bar_change(MenuBarChange::ShowDate(value), cx)
                                });
                            }
                        })
                    })
                    .collect();
                let date_value = popup_value(&date_choices, "Always");
                let body = div()
                    .v_flex()
                    .child(heading("Clock Options"))
                    .child(section_header("Date"))
                    .child(card(vec![
                        popup_row(
                            "menu-bar-clock-date",
                            "Show date",
                            None,
                            date_value,
                            date_choices,
                            enabled,
                        ),
                        switch(
                            "menu-bar-clock-weekday",
                            "Show the day of the week",
                            clock.show_weekday,
                            false,
                            MenuBarChange::ShowWeekday,
                        ),
                    ]))
                    .child(section_header("Time"))
                    .child(card(vec![
                        switch(
                            "menu-bar-clock-24-hour",
                            "Use a 24-hour clock",
                            twenty_four,
                            false,
                            MenuBarChange::TwentyFourHour,
                        ),
                        switch(
                            "menu-bar-clock-am-pm",
                            "Show am/pm",
                            clock.show_am_pm && !twenty_four,
                            twenty_four,
                            MenuBarChange::ShowAmPm,
                        ),
                        switch(
                            "menu-bar-seconds",
                            "Display the time with seconds",
                            clock.show_seconds,
                            false,
                            MenuBarChange::ShowSeconds,
                        ),
                    ]));
                ("menu-bar-clock-sheet", 430.0, body)
            }
            MenuBarSheet::Battery => {
                let body = div()
                    .v_flex()
                    .child(heading("Battery Options"))
                    .child(card(vec![switch(
                        "menu-bar-battery-percentage",
                        "Show Percentage",
                        settings.indicators.battery_percentage,
                        false,
                        MenuBarChange::BatteryPercentage,
                    )]));
                ("menu-bar-battery-sheet", 190.0, body)
            }
        };
        let done_view = view.clone();
        Some(
            settings_sheet(
                id,
                460.0,
                height,
                None,
                body.into_any_element(),
                vec![sheet_default_button(
                    "menu-bar-sheet-done",
                    "Done",
                    move |_, cx| {
                        done_view.update(cx, |settings, cx| settings.set_menu_bar_sheet(None, cx));
                    },
                )],
            )
            .into_any_element(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuBarSheet {
    Clock,
    Battery,
}
