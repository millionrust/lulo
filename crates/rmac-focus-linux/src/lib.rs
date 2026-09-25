//! Linux time-change and resume hints for the Focus runtime.
//!
//! Signals are wake hints only. Consumers always resample `chrono::Local` and
//! monotonic time; no D-Bus payload becomes schedule authority.

use std::fmt;

use async_channel::Sender;

pub mod client;
pub mod service;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Reasons {
    pub initial: bool,
    pub clock_or_timezone_changed: bool,
    pub resumed: bool,
    pub service_restarted: bool,
}

impl Reasons {
    pub const fn initial() -> Self {
        Self {
            initial: true,
            clock_or_timezone_changed: false,
            resumed: false,
            service_restarted: false,
        }
    }

    pub const fn is_empty(self) -> bool {
        !self.initial && !self.clock_or_timezone_changed && !self.resumed && !self.service_restarted
    }

    pub fn merge(&mut self, other: Self) {
        self.initial |= other.initial;
        self.clock_or_timezone_changed |= other.clock_or_timezone_changed;
        self.resumed |= other.resumed;
        self.service_restarted |= other.service_restarted;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Refresh(Reasons),
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Connect,
    Subscribe,
    Read,
    Publish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Focus time watcher failed ({:?})", self.kind)
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(target_os = "linux")]
pub async fn watch(sender: Sender<Event>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                sender.send(Event::Unavailable).await.map_err(|_| Error {
                    kind: ErrorKind::Publish,
                })?;
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: Sender<Event>) -> Result<(), Error> {
    sender.send(Event::Unavailable).await.map_err(|_| Error {
        kind: ErrorKind::Publish,
    })
}

#[cfg(target_os = "linux")]
async fn watch_once(sender: &Sender<Event>) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system().await.map_err(|_| Error {
        kind: ErrorKind::Connect,
    })?;
    let rule = |path: &'static str| -> Result<_, Error> {
        MatchRule::builder()
            .msg_type(Type::Signal)
            .path_namespace(path)
            .map_err(|_| Error {
                kind: ErrorKind::Subscribe,
            })
            .map(|builder| builder.build())
    };
    let mut timedate =
        MessageStream::for_match_rule(rule("/org/freedesktop/timedate1")?, &connection, Some(8))
            .await
            .map_err(|_| Error {
                kind: ErrorKind::Subscribe,
            })?
            .fuse();
    let mut login =
        MessageStream::for_match_rule(rule("/org/freedesktop/login1")?, &connection, Some(8))
            .await
            .map_err(|_| Error {
                kind: ErrorKind::Subscribe,
            })?
            .fuse();
    let mut dbus =
        MessageStream::for_match_rule(rule("/org/freedesktop/DBus")?, &connection, Some(8))
            .await
            .map_err(|_| Error {
                kind: ErrorKind::Subscribe,
            })?
            .fuse();

    publish(sender, Reasons::initial()).await?;
    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let reason = futures_util::select! {
            message = timedate.next() => timedate_reason(message)?,
            message = login.next() => login_reason(message)?,
            message = dbus.next() => dbus_reason(message)?,
            _ = closed => return Ok(()),
        };
        if let Some(reason) = reason {
            publish(sender, reason).await?;
        }
    }
}

#[cfg(target_os = "linux")]
fn timedate_reason(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<Reasons>, Error> {
    checked_message(message).map(|_| {
        Some(Reasons {
            clock_or_timezone_changed: true,
            ..Reasons::default()
        })
    })
}

#[cfg(target_os = "linux")]
fn login_reason(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<Reasons>, Error> {
    let message = checked_message(message)?;
    if message.header().member().map(|member| member.as_str()) != Some("PrepareForSleep") {
        return Ok(None);
    }
    let preparing: bool = message.body().deserialize().map_err(|_| Error {
        kind: ErrorKind::Read,
    })?;
    Ok((!preparing).then_some(Reasons {
        resumed: true,
        ..Reasons::default()
    }))
}

#[cfg(target_os = "linux")]
fn dbus_reason(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<Reasons>, Error> {
    let message = checked_message(message)?;
    if message.header().member().map(|member| member.as_str()) != Some("NameOwnerChanged") {
        return Ok(None);
    }
    let (name, _old_owner, new_owner): (String, String, String) =
        message.body().deserialize().map_err(|_| Error {
            kind: ErrorKind::Read,
        })?;
    Ok((!new_owner.is_empty()
        && matches!(
            name.as_str(),
            "org.freedesktop.timedate1" | "org.freedesktop.login1"
        ))
    .then_some(Reasons {
        service_restarted: true,
        ..Reasons::default()
    }))
}

#[cfg(target_os = "linux")]
fn checked_message(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<zbus::Message, Error> {
    match message {
        Some(Ok(message)) => Ok(message),
        Some(Err(_)) | None => Err(Error {
            kind: ErrorKind::Read,
        }),
    }
}

#[cfg(target_os = "linux")]
async fn publish(sender: &Sender<Event>, reasons: Reasons) -> Result<(), Error> {
    sender
        .send(Event::Refresh(reasons))
        .await
        .map_err(|_| Error {
            kind: ErrorKind::Publish,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_reasons_merge_without_losing_resume_or_time_change() {
        let mut reasons = Reasons {
            resumed: true,
            ..Reasons::default()
        };
        reasons.merge(Reasons {
            clock_or_timezone_changed: true,
            service_restarted: true,
            ..Reasons::default()
        });
        assert!(reasons.resumed);
        assert!(reasons.clock_or_timezone_changed);
        assert!(reasons.service_restarted);
        assert!(!reasons.is_empty());
        assert!(!Reasons::initial().is_empty());
    }

    #[test]
    fn errors_and_unavailable_events_contain_no_service_payload() {
        assert_eq!(
            format!(
                "{}",
                Error {
                    kind: ErrorKind::Read
                }
            ),
            "Focus time watcher failed (Read)"
        );
        assert_eq!(Event::Unavailable, Event::Unavailable);
    }
}
