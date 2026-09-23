//! Camera and Microphone rows and their per-application pages, backed by the
//! XDG portal PermissionStore.

use super::*;

const RESOURCES: [rmac_privacy::PortalResource; 2] = [
    rmac_privacy::PortalResource::Camera,
    rmac_privacy::PortalResource::Microphone,
];

fn resource_icon(resource: rmac_privacy::PortalResource) -> &'static str {
    match resource {
        rmac_privacy::PortalResource::Camera => "icons/user.svg",
        rmac_privacy::PortalResource::Microphone => "icons/volume-2.svg",
    }
}

impl Settings {
    pub(super) fn append_portal_permissions(&self, view: Entity<Self>, cards: &mut Vec<Div>) {
        if self.privacy_loading && self.privacy.is_none() {
            cards.push(footnote(
                "Loading decisions from the portal PermissionStore…",
            ));
            return;
        }
        let Some(snapshot) = &self.privacy else {
            return;
        };
        if !snapshot.available {
            cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                "The portal PermissionStore is unavailable in this session.".into()
            })));
            return;
        }
        // The Mac lists each privacy category as a 42 pt row showing how
        // many applications hold a decision.
        let rows = RESOURCES
            .into_iter()
            .map(|resource| {
                let count = snapshot
                    .decisions
                    .iter()
                    .filter(|decision| decision.resource == resource)
                    .count();
                let open_view = view.clone();
                icon_nav_row(
                    SharedString::from(format!("privacy-{}", resource.id())),
                    tile(resource_icon(resource), hsl(0x8e8e93), style::ROW_ICON)
                        .into_any_element(),
                    resource.label(),
                    Some(count.to_string().into()),
                    move |_, cx| {
                        open_view.update(cx, |settings, cx| {
                            settings.push(
                                SubPage::PrivacyResource {
                                    resource: resource.id().to_owned(),
                                },
                                cx,
                            );
                        });
                    },
                )
            })
            .collect();
        cards.push(card(rows));
        if let Some(detail) = &snapshot.detail {
            cards.push(footnote(detail.clone()));
        }
    }

    /// Privacy & Security › Camera / Microphone: "Allow the applications
    /// below to access your camera." then one row per stored decision.
    pub(in crate::controller) fn privacy_resource_body(
        &self,
        resource_id: &str,
        cx: &Context<Self>,
    ) -> Div {
        let view = cx.entity();
        let Some(resource) = RESOURCES
            .into_iter()
            .find(|resource| resource.id() == resource_id)
        else {
            return note_card("This privacy category is not available.");
        };
        let Some(snapshot) = self.privacy.as_ref().filter(|snapshot| snapshot.available) else {
            return note_card("The portal PermissionStore is unavailable in this session.");
        };
        let noun = resource.label().to_lowercase();
        let mut rows = vec![row_base()
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .line_height(px(16.0))
                    .text_color(secondary())
                    .child(format!(
                        "Allow the applications below to access your {noun}."
                    )),
            )
            .into_any_element()];
        let decisions = snapshot
            .decisions
            .iter()
            .filter(|decision| decision.resource == resource)
            .cloned()
            .collect::<Vec<_>>();
        if decisions.is_empty() {
            rows.push(
                group_placeholder(format!("No applications have asked to use your {noun}."))
                    .into_any_element(),
            );
        }
        for decision in decisions {
            let identity = self.application_identity(&decision.app_id);
            let display_name = identity
                .map(|application| application.name.clone())
                .unwrap_or_else(|| decision.app_id.clone());
            let icon = app_icon(
                identity.and_then(|application| application.icon.as_ref()),
                "icons/app-window.svg",
                secondary(),
                style::ROW_ICON,
            );
            let busy = self
                .privacy_busy
                .as_ref()
                .is_some_and(|(busy_resource, busy_app)| {
                    *busy_resource == decision.resource && busy_app == &decision.app_id
                });
            let reset_view = view.clone();
            let reset_decision = decision.clone();
            rows.push(
                icon_row(icon, display_name)
                    .child(
                        push_button(
                            SharedString::from(format!(
                                "privacy-reset-{}-{}",
                                decision.resource.id(),
                                decision.app_id
                            )),
                            "Reset…",
                        )
                        .busy(busy)
                        .disabled(
                            !snapshot.can_reset
                                || self.privacy_busy.is_some()
                                || self.privacy_stream_refreshing,
                        )
                        .on_click(move |_, _, cx| {
                            reset_view.update(cx, |settings, cx| {
                                settings.request_privacy_reset(reset_decision.clone(), cx);
                            });
                        }),
                    )
                    .into_any_element(),
            );
        }
        let mut body = div().v_flex().child(card(rows));

        if let Some(decision) = self
            .privacy_reset_confirmation
            .as_ref()
            .filter(|decision| decision.resource == resource)
        {
            let cancel_view = view.clone();
            let confirm_view = view;
            body = body
                .child(footnote(format!(
                    "Reset the stored {} decision for {}? The next request may ask again. This does not end access already in use or change permissions for native applications.",
                    decision.resource.label().to_lowercase(),
                    decision.app_id
                )))
                .child(footer_buttons(vec![
                    push_button("privacy-reset-cancel", "Cancel")
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_privacy_reset(cx));
                        })
                        .into_any_element(),
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
                    })
                    .into_any_element(),
                ]));
        }
        body
    }
}
