//! Bluetooth pane presentation, laid out like macOS 26
//! (design-lab/settings.html): a header card with the Bluetooth switch and
//! discoverability, then My Devices and Nearby Devices.

use super::*;

mod dialogs;

impl Settings {
    pub(in crate::controller) fn render_bluetooth(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let description: SharedString = if self.bluetooth_loading {
            "Reading system state…".into()
        } else if self.bluetooth_busy {
            "Applying change…".into()
        } else {
            "Connect to accessories you can use for activities such as streaming music, typing and gaming.".into()
        };
        let power_view = view.clone();
        let power = Toggle::new("bluetooth-power")
            .checked(self.bluetooth_on)
            .disabled(self.bluetooth_loading || self.bluetooth_busy || !self.bluetooth_available)
            .on_click(move |powered, _, cx| {
                power_view.update(cx, |settings, cx| {
                    settings.set_bluetooth_powered(*powered, cx)
                });
            });
        let mut header_rows = vec![row_base()
            .items_start()
            .gap(px(12.0))
            .child(tile("icons/bluetooth.svg", accent(), style::HEADER_ICON))
            .child(text_block("Bluetooth".into(), Some(description)))
            .child(power)
            .into_any_element()];
        if self.bluetooth_on && self.bluetooth_available && !self.bluetooth_loading {
            let discoverable_view = view.clone();
            let discoverable = Toggle::new("bluetooth-discoverable")
                .checked(self.bt_discoverable)
                .disabled(self.bluetooth_busy || self.bluetooth_discovering)
                .on_click(move |enabled, _, cx| {
                    discoverable_view.update(cx, |settings, cx| {
                        settings.set_bluetooth_discoverable(*enabled, cx)
                    });
                });
            let subtitle: SharedString = match (&self.bluetooth_adapter_name, self.bt_discoverable)
            {
                (Some(name), true) => {
                    format!("This computer is discoverable as \u{201c}{name}\u{201d}.").into()
                }
                _ => "Allow nearby devices to find this computer.".into(),
            };
            header_rows.push(
                row_base()
                    .items_start()
                    .child(text_block("Discoverable".into(), Some(subtitle)))
                    .child(discoverable)
                    .into_any_element(),
            );
        }
        let mut cards = vec![card(header_rows)];

        if self.bluetooth_loading {
            cards.push(note_card("Loading Bluetooth state from the system…"));
            return self.pane(cards);
        }
        if !self.bluetooth_available {
            cards.push(note_card(
                "No Bluetooth adapter is available through the system Bluetooth service.",
            ));
            return self.pane(cards);
        }

        if self.bluetooth_on {
            let row = |device: &rmac_bluetooth::Device| {
                bluetooth_device_row(
                    &view,
                    device,
                    self.bluetooth_busy,
                    self.bluetooth_forgetting.as_deref() == Some(device.id.as_str()),
                )
            };
            let mine: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| device.paired || device.connected)
                .map(row)
                .collect();
            if !mine.is_empty() {
                cards.push(section_header("My Devices"));
                cards.push(card(mine));
            }

            let nearby: Vec<AnyElement> = self
                .bt_devices
                .iter()
                .filter(|device| !device.paired && !device.connected)
                .map(|device| bluetooth_device_row(&view, device, self.bluetooth_busy, false))
                .collect();
            cards.push(section_header("Nearby Devices"));
            if nearby.is_empty() {
                cards.push(card(vec![row_base()
                    .justify_center()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(secondary())
                            .child(if self.bluetooth_discovering {
                                "Searching…"
                            } else {
                                "No nearby devices found"
                            }),
                    )
                    .into_any_element()]));
            } else {
                cards.push(card(nearby));
            }

            let refresh_view = view.clone();
            cards.push(footer_buttons(vec![push_button(
                "bluetooth-refresh",
                if self.bluetooth_busy || self.bluetooth_discovering {
                    "Scanning…"
                } else {
                    "Refresh"
                },
            )
            .busy(self.bluetooth_busy || self.bluetooth_discovering)
            .disabled(self.bluetooth_busy || self.bluetooth_discovering)
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_bluetooth(cx));
            })
            .into_any_element()]));
            cards.push(footnote(
                "Confirm that pairing codes match on both devices. A device becomes trusted only after BlueZ reports that pairing succeeded.",
            ));
        }
        self.pane(cards)
    }
}
