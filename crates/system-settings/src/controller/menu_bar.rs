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
                "The versioned rmac shell-settings authority is unavailable. Menu bar items stay as they are until it is readable again.",
            ));
            return self.pane(cards);
        };
        let settings = &snapshot.settings;
        let indicators = &settings.indicators;
        let enabled = !self.shell_settings_busy;

        let switch_view = view.clone();
        cards.push(card(vec![row_base()
            .child(text_block("Show seconds in the clock".into(), None))
            .child(
                Toggle::new("menu-bar-seconds")
                    .checked(settings.clock.show_seconds)
                    .disabled(!enabled)
                    .on_click(move |value, _, cx| {
                        switch_view.update(cx, |settings, cx| {
                            settings.apply_menu_bar_change(MenuBarChange::ShowSeconds(*value), cx)
                        });
                    }),
            )
            .into_any_element()]));

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
        cards.push(card(vec![
            control(
                "menu-bar-network",
                "icons/wifi.svg",
                accent(),
                "Wi-Fi",
                indicators.network,
                MenuBarChange::Network,
            ),
            control(
                "menu-bar-vpn",
                "icons/key.svg",
                accent(),
                "VPN",
                indicators.vpn,
                MenuBarChange::Vpn,
            ),
            control(
                "menu-bar-bluetooth",
                "icons/bluetooth.svg",
                accent(),
                "Bluetooth",
                indicators.bluetooth,
                MenuBarChange::Bluetooth,
            ),
            control(
                "menu-bar-sound",
                "icons/volume-2.svg",
                hsl(0xff2d55),
                "Sound",
                indicators.sound,
                MenuBarChange::Sound,
            ),
            control(
                "menu-bar-battery",
                "icons/battery-charging.svg",
                hsl(0x34c759),
                "Battery",
                indicators.power,
                MenuBarChange::Power,
            ),
            control(
                "menu-bar-focus",
                "icons/moon.svg",
                hsl(0x5e5ce6),
                "Focus",
                indicators.focus,
                MenuBarChange::Focus,
            ),
            control(
                "menu-bar-notifications",
                "icons/bell.svg",
                hsl(0xff3b30),
                "Notifications",
                indicators.notifications,
                MenuBarChange::Notifications,
            ),
        ]));

        let percentage_view = view.clone();
        cards.push(card(vec![row_base()
            .child(text_block("Show battery percentage".into(), None))
            .child(
                Toggle::new("menu-bar-battery-percentage")
                    .checked(indicators.battery_percentage)
                    .disabled(!enabled || !indicators.power)
                    .on_click(move |value, _, cx| {
                        percentage_view.update(cx, |settings, cx| {
                            settings
                                .apply_menu_bar_change(MenuBarChange::BatteryPercentage(*value), cx)
                        });
                    }),
            )
            .into_any_element()]));
        self.pane(cards)
    }
}
