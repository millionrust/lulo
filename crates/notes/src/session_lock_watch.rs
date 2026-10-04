//! Open locked notes close when the PC goes to sleep or the screen locks,
//! as on macOS. logind announces both on the system bus (`PrepareForSleep`
//! on the manager; `Lock` and the `LockedHint` property on a session); this
//! waits on those signals and never polls. Without a system bus (a nested
//! test session) it simply ends.

use std::collections::HashMap;

use futures_util::StreamExt as _;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{message::Type, MatchRule, MessageStream};

use super::*;

impl NotesView {
    pub(super) fn watch_session_lock(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Ok(connection) = rmac_dbus::system().await else {
                return;
            };
            let Ok(rule) = MatchRule::builder()
                .msg_type(Type::Signal)
                .path_namespace("/org/freedesktop/login1")
                .map(|builder| builder.build())
            else {
                return;
            };
            let Ok(mut stream) = MessageStream::for_match_rule(rule, &connection, Some(16)).await
            else {
                return;
            };
            while let Some(message) = stream.next().await {
                let Ok(message) = message else {
                    continue;
                };
                if !locks_session(&message) {
                    continue;
                }
                if this
                    .update(cx, |this, cx| this.close_all_locked_notes(cx))
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }
}

fn locks_session(message: &zbus::Message) -> bool {
    let header = message.header();
    match header.member().map(|member| member.as_str()) {
        Some("PrepareForSleep") => message.body().deserialize::<bool>().unwrap_or(false),
        Some("Lock") => true,
        Some("PropertiesChanged") => message
            .body()
            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            .is_ok_and(|(_, changed, _)| {
                changed
                    .get("LockedHint")
                    .is_some_and(|value| matches!(&**value, Value::Bool(true)))
            }),
        _ => false,
    }
}
