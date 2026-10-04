//! Whether the login session shows the lock screen, for the banner daemon.
//!
//! The lock provider sets logind's `LockedHint` once the compositor has
//! confirmed the lock and clears it after an authenticated unlock. While it
//! is set, banners are held (macOS shows nothing new on the lock screen
//! either): the lock surface hides them visually, but a banner's assertive
//! live region and sound would still reach a screen reader and the speakers.
//! Held banners are presented on unlock; Center history records them as they
//! arrive.

use std::collections::VecDeque;

use async_channel::Sender;
use futures_util::StreamExt as _;

/// How many runtime events wait for unlock. The oldest is dropped past this;
/// its notification is still in Center history.
pub const HELD_EVENT_LIMIT: usize = 256;

/// Holds events while the session is locked and releases them, in order, on
/// unlock.
#[derive(Debug)]
pub struct LockGate<T> {
    locked: bool,
    held: VecDeque<T>,
}

impl<T> Default for LockGate<T> {
    fn default() -> Self {
        Self {
            locked: false,
            held: VecDeque::new(),
        }
    }
}

impl<T> LockGate<T> {
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// The event to present now, or `None` when it waits for unlock.
    pub fn admit(&mut self, event: T) -> Option<T> {
        if !self.locked {
            return Some(event);
        }
        if self.held.len() == HELD_EVENT_LIMIT {
            self.held.pop_front();
        }
        self.held.push_back(event);
        None
    }

    /// Record the lock state; unlocking returns the held events in order.
    pub fn set_locked(&mut self, locked: bool) -> Vec<T> {
        self.locked = locked;
        if locked {
            Vec::new()
        } else {
            self.held.drain(..).collect()
        }
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn get_session(&self, session_id: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// Send the session's lock state now and on every change. Returns when the
/// state can no longer be followed; the caller then treats the session as
/// unlocked, which never holds banners forever.
pub async fn watch(sender: Sender<bool>) -> zbus::Result<()> {
    let connection = rmac_dbus::system().await?;
    let manager = ManagerProxy::new(&connection).await?;
    let session_id = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()) => id,
        _ => {
            // A user service outside the session scope: logind's "auto"
            // names the user's display session.
            SessionProxy::builder(&connection)
                .path("/org/freedesktop/login1/session/auto")?
                .build()
                .await?
                .id()
                .await?
        }
    };
    let path = manager.get_session(&session_id).await?;
    let session = SessionProxy::builder(&connection)
        .path(path)?
        .build()
        .await?;
    let mut changes = session.receive_locked_hint_changed().await;
    let initial = session.locked_hint().await.unwrap_or(false);
    if sender.send(initial).await.is_err() {
        return Ok(());
    }
    while let Some(change) = changes.next().await {
        let Ok(locked) = change.get().await else {
            continue;
        };
        if sender.send(locked).await.is_err() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_events_wait_and_unlock_releases_them_in_order() {
        let mut gate = LockGate::default();
        assert_eq!(gate.admit(1), Some(1));
        assert!(gate.set_locked(true).is_empty());
        assert!(gate.locked());
        assert_eq!(gate.admit(2), None);
        assert_eq!(gate.admit(3), None);
        assert_eq!(gate.set_locked(false), vec![2, 3]);
        assert_eq!(gate.admit(4), Some(4));
        assert!(gate.set_locked(false).is_empty());
    }

    #[test]
    fn held_events_are_bounded_dropping_the_oldest() {
        let mut gate = LockGate::default();
        gate.set_locked(true);
        for event in 0..HELD_EVENT_LIMIT + 10 {
            assert_eq!(gate.admit(event), None);
        }
        let released = gate.set_locked(false);
        assert_eq!(released.len(), HELD_EVENT_LIMIT);
        assert_eq!(released[0], 10);
        assert_eq!(released.last(), Some(&(HELD_EVENT_LIMIT + 9)));
    }
}
