//! Background-service inventory projection.

use super::*;

impl Settings {
    pub(super) fn append_background_services(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_login_items::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        cards.push(section_header("Allow in background"));
        if snapshot.background_services.is_empty() {
            cards.push(note_card(if snapshot.background_services_error.is_some() {
                "The systemd user manager is unavailable. XDG application login items remain usable."
            } else {
                "No enabled or user-installed systemd background services were found."
            }));
        } else {
            let rows = snapshot
                .background_services
                .iter()
                .map(|service| {
                    let id = service.id.clone();
                    let reveal_id = service.id.clone();
                    let toggle_view = view.clone();
                    let reveal_view = view.clone();
                    let busy_key = format!("systemd:{}", service.id);
                    let busy = self.login_item_busy.as_deref() == Some(busy_key.as_str());
                    let reveal_key = format!("reveal:{}", service.id);
                    let revealing = self.login_item_busy.as_deref() == Some(reveal_key.as_str());
                    let subtitle = format!("{} · {}", service.detail, service.state.label());
                    row_base()
                        .child(tile("icons/settings.svg", secondary(), style::ROW_ICON))
                        .child(text_block(
                            service.name.clone().into(),
                            Some(subtitle.into()),
                        ))
                        .when(service.source.is_some(), |row| {
                            row.child(
                                Button::new(
                                    ElementId::from(SharedString::from(format!(
                                        "reveal-background-service-{}",
                                        service.id
                                    ))),
                                    "Show in Files",
                                )
                                .busy(revealing)
                                .disabled(self.login_item_busy.is_some())
                                .on_click(move |_, _, cx| {
                                    reveal_view.update(cx, |settings, cx| {
                                        settings.reveal_login_item(reveal_id.clone(), true, cx);
                                    });
                                }),
                            )
                        })
                        .child(
                            Toggle::new(ElementId::from(SharedString::from(format!(
                                "background-service-{}",
                                service.id
                            ))))
                            .checked(service.enabled)
                            .disabled(self.login_item_busy.is_some() || !service.can_toggle)
                            .on_click(move |enabled, _, cx| {
                                toggle_view.update(cx, |settings, cx| {
                                    settings.set_background_service_enabled(
                                        id.clone(),
                                        *enabled,
                                        cx,
                                    );
                                });
                            }),
                        )
                        .when(busy, |row| {
                            row.child(
                                div()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child("Saving…"),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(error) = &snapshot.background_services_error {
            cards.push(note_card(format!(
                "Background service status is unavailable: {error}"
            )));
        }
        if snapshot.background_services_truncated {
            cards.push(note_card(
                "The systemd user service inventory exceeded the bounded display limit.",
            ));
        }
    }
}
