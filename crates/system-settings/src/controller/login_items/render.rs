//! Login Items settings presentation.

use super::*;

mod background;
mod open_at_login;

impl Settings {
    pub(in crate::controller) fn render_login_items(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let refresh = push_button("refresh-login-items", "Refresh")
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
            })
            .into_any_element();
        let Some(snapshot) = &self.login_items else {
            return self.pane(vec![
                section_with_note(
                    "Open at Login",
                    "These items will open automatically when you log in.",
                    true,
                ),
                group().child(group_placeholder(if self.login_items_loading {
                    "Loading…"
                } else {
                    "Login items are unavailable."
                })),
                footer_buttons(vec![refresh]),
            ]);
        };

        let mut cards = Vec::new();
        self.append_open_at_login(view.clone(), snapshot, &mut cards);
        self.append_background_services(view, snapshot, &mut cards);

        if !snapshot.issues.is_empty() {
            cards.push(section_header("Entries Needing Attention"));
            cards.push(card(
                snapshot
                    .issues
                    .iter()
                    .map(|issue| {
                        row_base()
                            .items_start()
                            .child(text_block(
                                issue.file.clone().into(),
                                Some(issue.detail.clone().into()),
                            ))
                            .into_any_element()
                    })
                    .collect(),
            ));
        }
        if snapshot.truncated {
            cards.push(footnote(
                "Some login items are not shown because the list is too long.",
            ));
        }
        cards.push(footer_buttons(vec![refresh]));
        self.pane(cards)
    }
}
