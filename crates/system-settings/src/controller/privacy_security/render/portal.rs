//! Portal permission inventory and reset presentation.

use super::*;

impl Settings {
    pub(super) fn append_portal_permissions(&self, view: Entity<Self>, cards: &mut Vec<Div>) {
        let refresh_view = view.clone();
        cards.push(card(vec![row_base()
            .child(tile("icons/shield.svg", accent(), style::ROW_ICON))
            .child(text_block(
                "Portal permission decisions".into(),
                Some("Camera and microphone decisions stored by XDG portals".into()),
            ))
            .child(
                Button::new("privacy-refresh", "Refresh")
                    .busy(self.privacy_loading || self.privacy_stream_refreshing)
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_privacy(cx));
                    }),
            )
            .into_any_element()]));

        if self.privacy_loading {
            cards.push(note_card(
                "Loading decisions from the portal PermissionStore…",
            ));
        } else if let Some(snapshot) = &self.privacy {
            if snapshot.available {
                cards.push(card(vec![value_row(
                    "icons/info.svg",
                    secondary(),
                    "PermissionStore interface".into(),
                    format!("Version {}", snapshot.version).into(),
                )]));
                for resource in [
                    rmac_privacy::PortalResource::Camera,
                    rmac_privacy::PortalResource::Microphone,
                ] {
                    cards.push(section_header(resource.label()));
                    let decisions = snapshot
                        .decisions
                        .iter()
                        .filter(|decision| decision.resource == resource)
                        .cloned()
                        .collect::<Vec<_>>();
                    if decisions.is_empty() {
                        cards.push(note_card(format!(
                            "No stored {} decisions. This does not prove that native or already-running applications lack access.",
                            resource.label().to_lowercase()
                        )));
                        continue;
                    }
                    let rows = decisions
                        .into_iter()
                        .map(|decision| {
                            let identity = self.application_identity(&decision.app_id);
                            let display_name = identity
                                .map(|application| application.name.as_str())
                                .unwrap_or(&decision.app_id)
                                .to_owned();
                            let detail = format!(
                                "{} · Stored tokens: {}",
                                decision.app_id,
                                decision.summary()
                            );
                            let reset_view = view.clone();
                            let reset_decision = decision.clone();
                            let busy = self.privacy_busy.as_ref().is_some_and(
                                |(busy_resource, busy_app)| {
                                    *busy_resource == decision.resource
                                        && busy_app == &decision.app_id
                                },
                            );
                            row_base()
                                .child(tile("icons/app-window.svg", secondary(), style::ROW_ICON))
                                .child(text_block(display_name.into(), Some(detail.into())))
                                .child(
                                    Button::new(
                                        SharedString::from(format!(
                                            "privacy-reset-{}-{}",
                                            decision.resource.id(),
                                            decision.app_id
                                        )),
                                        "Reset",
                                    )
                                    .busy(busy)
                                    .disabled(
                                        !snapshot.can_reset
                                            || self.privacy_busy.is_some()
                                            || self.privacy_stream_refreshing,
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            reset_view.update(cx, |settings, cx| {
                                                settings.request_privacy_reset(
                                                    reset_decision.clone(),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                )
                                .into_any_element()
                        })
                        .collect();
                    cards.push(card(rows));
                }
                if let Some(detail) = &snapshot.detail {
                    cards.push(note_card(detail.clone()));
                }
            } else {
                cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                    "The portal PermissionStore is unavailable in this session.".into()
                })));
            }
        }

        if let Some(decision) = &self.privacy_reset_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Reset the stored {} decision for {}? The next portal request may ask again. This does not terminate active access or change permissions for native applications.",
                decision.resource.label().to_lowercase(),
                decision.app_id
            )));
            cards.push(card(vec![row_base()
                .child(div().flex_1())
                .child(
                    Button::new("privacy-reset-cancel", "Cancel").on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| settings.cancel_privacy_reset(cx));
                    }),
                )
                .child(
                    rmac_ui::dialog_button(
                        "privacy-reset-confirm",
                        "Reset Decision",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_privacy_reset(cx));
                    }),
                )
                .into_any_element()]));
        }
    }
}
