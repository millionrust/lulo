//! Waiting for a session service to claim its well-known bus name.
//!
//! A watcher whose service is absent (a private nested session, a stopped
//! or crashed service) must not reconnect on a timer for the life of the
//! process: every failed attempt republishes an error that a window may
//! repaint for. [`wait_for_session_name`] parks the watcher until the bus
//! reports a new owner instead.

use futures_util::StreamExt as _;
use zbus::{message::Type, MatchRule, MessageStream};

/// How [`wait_for_session_name`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameWait {
    /// The name already had an owner, so the caller's failure was not caused
    /// by an absent service. Callers should fall back to a bounded retry.
    AlreadyOwned,
    /// The name gained an owner while waiting.
    Appeared,
}

/// Waits until `name` has an owner on this process's session bus.
///
/// The `NameOwnerChanged` match is armed before the ownership check, so a
/// service that starts between the two is still seen.
pub async fn wait_for_session_name(name: &str) -> zbus::Result<NameWait> {
    let connection = crate::session().await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .path("/org/freedesktop/DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(name)?
        .build();
    let mut owners = MessageStream::for_match_rule(rule, &connection, Some(4)).await?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    if dbus
        .name_has_owner(zbus::names::BusName::try_from(name)?)
        .await?
    {
        return Ok(NameWait::AlreadyOwned);
    }
    while let Some(message) = owners.next().await {
        let message = message?;
        let (changed, _old_owner, new_owner): (String, String, String) =
            message.body().deserialize()?;
        if changed == name && !new_owner.is_empty() {
            return Ok(NameWait::Appeared);
        }
    }
    Err(zbus::Error::Failure(format!(
        "the session bus closed while waiting for {name}"
    )))
}
