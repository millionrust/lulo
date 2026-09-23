//! System Settings Bluetooth device state and action row projection.

use super::*;

pub(in crate::controller) fn bluetooth_device_row(
    view: &Entity<Settings>,
    device: &rmac_bluetooth::Device,
    busy: bool,
    forgetting: bool,
) -> AnyElement {
    // The Mac shows only the connection state under a known device, and
    // the device kind under a nearby one.
    let subtitle: Option<SharedString> = if device.connected {
        Some("Connected".into())
    } else if device.paired {
        Some("Not Connected".into())
    } else {
        (!device.kind.is_empty()).then(|| device.kind.clone().into())
    };
    let connect_id = device.id.clone();
    let pair_id = device.id.clone();
    let pair_name = SharedString::from(device.name.clone());
    let connect = !device.connected;
    let connect_view = view.clone();
    let pair_view = view.clone();
    let forget_id = device.id.clone();
    let forget_name = SharedString::from(device.name.clone());
    let forget_view = view.clone();
    let action = if device.connected {
        "Disconnect"
    } else if device.paired {
        "Connect"
    } else {
        "Pair"
    };
    row_base()
        .min_h(px(style::NAV_ROW_HEIGHT + 15.0))
        .child(tile(
            "icons/bluetooth.svg",
            if device.connected {
                accent()
            } else {
                secondary()
            },
            style::HEADER_ICON,
        ))
        .child(text_block(device.name.clone().into(), subtitle))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    push_button(
                        SharedString::from(format!("bluetooth-device-action-{connect_id}")),
                        action,
                    )
                    .disabled(busy)
                    .when(device.paired, |button| {
                        button.on_click(move |_, _, cx| {
                            connect_view.update(cx, |settings, cx| {
                                settings.set_bluetooth_device_connected(
                                    connect_id.clone(),
                                    connect,
                                    cx,
                                );
                            });
                        })
                    })
                    .when(!device.paired, |button| {
                        button.on_click(move |_, window, cx| {
                            pair_view.update(cx, |settings, cx| {
                                settings.begin_bluetooth_pairing(
                                    pair_id.clone(),
                                    pair_name.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                    }),
                )
                .when(device.paired, |actions| {
                    actions.child(
                        push_button(
                            SharedString::from(format!("bluetooth-device-forget-{forget_id}")),
                            "Forget…",
                        )
                        .busy(forgetting)
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            forget_view.update(cx, |settings, cx| {
                                settings.request_bluetooth_forget(
                                    forget_id.clone(),
                                    forget_name.clone(),
                                    cx,
                                );
                            });
                        }),
                    )
                }),
        )
        .into_any_element()
}
