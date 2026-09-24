//! Unsaved work on the session bus.
//!
//! An app process that holds an edited, unsaved document owns the name
//! `org.rmac.UnsavedWork.p<pid>` for exactly as long as it does, and answers
//! `org.rmac.UnsavedWork1.Preserve` by writing its crash-recovery drafts at
//! once. The menu bar follows these names (no polling: the bus announces
//! every owner change) and holds a logind delay inhibitor while any exists,
//! so a shutdown or sleep that did not start from Log Out, Restart or Shut
//! Down still gives every edited document the chance to reach disk. A
//! process that dies loses its names with its connection, so a crashed app
//! never keeps the inhibitor alive.

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::{interface, Connection};

use crate::{authenticated_sender, bus_error, Error};

/// Every unsaved-work name lives under this namespace.
pub const NAMESPACE: &str = "org.rmac.UnsavedWork";
pub const OBJECT_PATH: &str = "/org/rmac/UnsavedWork";
pub const INTERFACE_NAME: &str = "org.rmac.UnsavedWork1";
const PRESERVE_CAPACITY: usize = 4;

/// The name process `pid` owns while it has unsaved work. A bus name
/// element may not start with a digit, hence the `p`.
pub fn bus_name(pid: u32) -> String {
    format!("{NAMESPACE}.p{pid}")
}

/// Whether `name` is an unsaved-work name, and not the namespace itself or
/// some other `org.rmac` endpoint.
pub fn is_bus_name(name: &str) -> bool {
    name.strip_prefix(NAMESPACE)
        .and_then(|rest| rest.strip_prefix(".p"))
        .is_some_and(|pid| {
            !pid.is_empty() && pid.len() <= 10 && pid.bytes().all(|b| b.is_ascii_digit())
        })
}

/// A request from the bus to write recovery drafts now. The app's main
/// thread answers on the enclosed sender once every draft is on disk (or has
/// failed visibly); dropping it unanswered reports a failure to the caller.
pub type PreserveRequest = async_channel::Sender<()>;

pub fn preserve_channel() -> (
    async_channel::Sender<PreserveRequest>,
    async_channel::Receiver<PreserveRequest>,
) {
    async_channel::bounded(PRESERVE_CAPACITY)
}

struct UnsavedInterface {
    requests: async_channel::Sender<PreserveRequest>,
}

#[interface(name = "org.rmac.UnsavedWork1")]
impl UnsavedInterface {
    async fn preserve(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        let (done, finished) = async_channel::bounded(1);
        self.requests
            .try_send(done)
            .map_err(|_| fdo::Error::Failed("the preserve queue is unavailable".into()))?;
        finished
            .recv()
            .await
            .map_err(|_| fdo::Error::Failed("the app could not preserve its drafts".into()))
    }
}

/// One process's unsaved-work endpoint. The object is always exported; the
/// name is owned only while [`UnsavedEndpoint::set_unsaved`] says so.
pub struct UnsavedEndpoint {
    connection: Connection,
    name: String,
    owned: bool,
}

impl UnsavedEndpoint {
    /// Connect and export the Preserve object without claiming the name.
    pub async fn connect(requests: async_channel::Sender<PreserveRequest>) -> Result<Self, Error> {
        let connection = Builder::session()
            .map_err(bus_error("connect to the session bus"))?
            .serve_at(OBJECT_PATH, UnsavedInterface { requests })
            .map_err(bus_error("export the unsaved-work object"))?
            .build()
            .await
            .map_err(bus_error("publish the unsaved-work object"))?;
        Ok(Self {
            connection,
            name: bus_name(std::process::id()),
            owned: false,
        })
    }

    /// Own the name while `unsaved`, release it once everything is saved.
    pub async fn set_unsaved(&mut self, unsaved: bool) -> Result<(), Error> {
        if unsaved == self.owned {
            return Ok(());
        }
        if unsaved {
            self.connection
                .request_name(self.name.as_str())
                .await
                .map_err(bus_error("claim the unsaved-work name"))?;
        } else {
            self.connection
                .release_name(self.name.as_str())
                .await
                .map_err(bus_error("release the unsaved-work name"))?;
        }
        self.owned = unsaved;
        Ok(())
    }
}

/// The unsaved-work names owned right now, to seed a watcher.
pub async fn current_owners() -> Result<Vec<String>, Error> {
    let connection = crate::session().await?;
    let bus = fdo::DBusProxy::new(&connection)
        .await
        .map_err(bus_error("reach the bus daemon"))?;
    let names = bus
        .list_names()
        .await
        .map_err(|error| Error::Bus(format!("could not list bus names: {error}")))?;
    Ok(names
        .iter()
        .map(|name| name.as_str().to_owned())
        .filter(|name| is_bus_name(name))
        .collect())
}

/// Ask the process owning `name` to write its recovery drafts now. The
/// caller bounds the wait; a busy or hung app must not hold a shutdown.
pub async fn preserve(name: &str) -> Result<(), Error> {
    if !is_bus_name(name) {
        return Err(Error::Protocol);
    }
    crate::session()
        .await?
        .call_method(
            Some(name),
            OBJECT_PATH,
            Some(INTERFACE_NAME),
            "Preserve",
            &(),
        )
        .await
        .map_err(|error| Error::Bus(format!("{name} Preserve: {error}")))?;
    Ok(())
}

/// Unsaved-work names appearing and disappearing.
pub struct UnsavedOwners {
    stream: zbus::MessageStream,
}

pub async fn watch_owners() -> Result<UnsavedOwners, Error> {
    let rule_error =
        |error: zbus::Error| Error::Bus(format!("could not build the unsaved-work match: {error}"));
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(rule_error)?
        .interface("org.freedesktop.DBus")
        .map_err(rule_error)?
        .member("NameOwnerChanged")
        .map_err(rule_error)?
        .arg0ns(NAMESPACE)
        .map_err(rule_error)?
        .build();
    let connection = crate::session().await?;
    let stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(64))
        .await
        .map_err(bus_error("watch unsaved-work owners"))?;
    Ok(UnsavedOwners { stream })
}

impl UnsavedOwners {
    /// The next name that gained (`true`) or lost (`false`) its owner;
    /// `None` once the bus connection closes.
    pub async fn next(&mut self) -> Option<(String, bool)> {
        use futures_util::StreamExt;
        loop {
            let message = match self.stream.next().await? {
                Ok(message) => message,
                Err(error) => {
                    crate::forget_closed_session(&error);
                    return None;
                }
            };
            let body = message.body();
            let Ok((name, _old, new)) = body.deserialize::<(&str, &str, &str)>() else {
                continue;
            };
            if is_bus_name(name) {
                return Some((name.to_owned(), !new.is_empty()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_names_carry_the_pid_and_nothing_else_matches() {
        assert_eq!(bus_name(4242), "org.rmac.UnsavedWork.p4242");
        assert!(is_bus_name(&bus_name(1)));
        assert!(is_bus_name(&bus_name(u32::MAX)));
        for other in [
            "org.rmac.UnsavedWork",
            "org.rmac.UnsavedWork.p",
            "org.rmac.UnsavedWork.4242",
            "org.rmac.UnsavedWork.p42x",
            "org.rmac.UnsavedWork.p42.p1",
            "org.rmac.UnsavedWorkers.p42",
            "org.rmac.TextEditor.Menu",
        ] {
            assert!(!is_bus_name(other), "{other}");
        }
    }

    #[test]
    fn unsaved_names_are_not_menu_names() {
        assert_eq!(crate::app_for_bus_name(&bus_name(7)), None);
    }
}
