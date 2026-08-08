//! Login Items settings presentation.

use super::*;

mod background;
mod open_at_login;

impl Settings {
    pub(in crate::controller) fn render_login_items(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = Button::new("refresh-login-items", "Refresh")
            .busy(
                self.login_item_busy.as_deref() == Some("refresh")
                    || self.login_items_stream_refreshing,
            )
            .disabled(
                self.login_items_loading
                    || self.login_item_busy.is_some()
                    || self.login_items_stream_refreshing,
            )
            .on_click(move |_, _, cx| {
                refresh_view.update(cx, |settings, cx| settings.refresh_login_items(cx));
            });
        let Some(snapshot) = &self.login_items else {
            return self.pane(vec![
                card(vec![row_base()
                    .child(tile("icons/app-window.svg", secondary(), 22.0))
                    .child(text_block(
                        "Open at login".into(),
                        Some("XDG autostart directories".into()),
                    ))
                    .child(refresh)
                    .into_any_element()]),
                note_card(if self.login_items_loading {
                    "Reading effective XDG autostart entries…"
                } else {
                    "Autostart entries are unavailable. No private fallback toggles are shown."
                }),
            ]);
        };

        let mut cards = Vec::new();
        self.append_open_at_login(view.clone(), snapshot, &mut cards);
        self.append_background_services(view, snapshot, &mut cards);

        if !snapshot.issues.is_empty() {
            cards.push(section_header("Entries needing attention"));
            cards.push(card(
                snapshot
                    .issues
                    .iter()
                    .map(|issue| {
                        row_base()
                            .child(tile("icons/info.svg", rmac_ui::mac::warning_text(), 22.0))
                            .child(text_block(
                                issue.file.clone().into(),
                                Some(issue.detail.clone().into()),
                            ))
                            .into_any_element()
                    })
                    .collect(),
            ));
        }
        let refresh_row = row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Authoritative state".into(),
                Some("Live XDG files · systemd user unit changes".into()),
            ))
            .child(refresh)
            .into_any_element();
        cards.push(card(vec![refresh_row]));
        if snapshot.truncated {
            cards.push(note_card(
                "The autostart inventory exceeded the bounded display limit.",
            ));
        }
        cards.push(note_card(
            "Changes to systemd user services take effect at the next sign-in; this pane does not start or stop running services. Adding or removing systemd unit files remains an administrator workflow.",
        ));
        self.pane(cards)
    }
}
