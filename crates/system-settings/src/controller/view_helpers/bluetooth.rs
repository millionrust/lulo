//! System Settings Bluetooth device state and action row projection.

use super::*;

pub(in crate::controller) fn bluetooth_device_row(
    view: &Entity<Settings>,
    device: &rmac_bluetooth::Device,
    busy: bool,
    forgetting: bool,
) -> AnyElement {
    let mut details = Vec::new();
    if !device.kind.is_empty() {
        details.push(device.kind.clone());
    }
    if !device.address.is_empty() {
        details.push(device.address.clone());
    }
    if device.paired {
        details.push(if device.trusted {
            "Trusted".into()
        } else {
            "Not trusted".into()
        });
    }
    let subtitle = (!details.is_empty()).then(|| details.join(" · ").into());
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
        .child(tile(
            "icons/bluetooth.svg",
            if device.connected {
                accent()
            } else {
                secondary()
            },
            22.0,
        ))
        .child(text_block(device.name.clone().into(), subtitle))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(
                        SharedString::from(format!("bluetooth-device-action-{connect_id}")),
                        action,
                    )
                    .xsmall()
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
                        Button::new(
                            SharedString::from(format!("bluetooth-device-forget-{forget_id}")),
                            "Forget…",
                        )
                        .xsmall()
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
