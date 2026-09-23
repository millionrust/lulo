//! App Background Activity: one 52 pt row per systemd user service.

use super::*;

impl Settings {
    pub(super) fn append_background_services(
        &self,
        view: Entity<Self>,
        snapshot: &rmac_login_items::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        cards.push(section_with_note(
            "App Background Activity",
            "These services start with your session and can keep running in the background. Changes take effect the next time you log in.",
            false,
        ));
        if snapshot.background_services.is_empty() {
            cards.push(group().child(group_placeholder(
                if snapshot.background_services_error.is_some() {
                    "Background services are unavailable."
                } else {
                    "No background services."
                },
            )));
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
                    let subtitle = if busy {
                        "Saving…".to_string()
                    } else {
                        format!("{} · {}", service.detail, service.state.label())
                    };
                    large_row(
                        tile26("icons/settings.svg", hsl(0x8e8e93)),
                        service.name.clone(),
                        Some(subtitle_text(subtitle)),
                    )
                    .when(service.source.is_some(), |row| {
                        row.child(reveal_button(
                            SharedString::from(format!("reveal-background-service-{}", service.id)),
                            move |_, cx| {
                                reveal_view.update(cx, |settings, cx| {
                                    settings.reveal_login_item(reveal_id.clone(), true, cx);
                                });
                            },
                        ))
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
                                settings.set_background_service_enabled(id.clone(), *enabled, cx);
                            });
                        }),
                    )
                    .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        if let Some(error) = &snapshot.background_services_error {
            cards.push(footnote(format!(
                "Background service status is unavailable: {error}"
            )));
        }
        if snapshot.background_services_truncated {
            cards.push(footnote(
                "Some background services are not shown because the list is too long.",
            ));
        }
    }
}
