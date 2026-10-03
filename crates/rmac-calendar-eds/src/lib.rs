//! EDS Calendar8 transport. All blocking calls belong on a worker thread.
//! No account data or iCalendar payload is logged by this crate.

use futures_lite::StreamExt as _;
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SOURCES_NAME: &str = "org.gnome.evolution.dataserver.Sources5";
const SOURCES_PATH: &str = "/org/gnome/evolution/dataserver/SourceManager";
const SOURCE_IFACE: &str = "org.gnome.evolution.dataserver.Source";
const FACTORY_NAME: &str = "org.gnome.evolution.dataserver.Calendar8";
const FACTORY_PATH: &str = "/org/gnome/evolution/dataserver/CalendarFactory";
const CAL_IFACE: &str = "org.gnome.evolution.dataserver.Calendar";
const VIEW_IFACE: &str = "org.gnome.evolution.dataserver.CalendarView";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub uid: String,
    pub display_name: String,
    pub parent_uid: Option<String>,
    pub backend: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewEvent {
    Added(Vec<String>),
    Modified(Vec<String>),
    Removed(Vec<String>),
    Complete(Result<(), String>),
}

#[derive(Debug)]
pub enum Error {
    Unavailable,
    Transport(zbus::Error),
    InvalidSource,
}

impl From<zbus::Error> for Error {
    fn from(value: zbus::Error) -> Self {
        Self::Transport(value)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(value: zbus::fdo::Error) -> Self {
        Self::Transport(value.into())
    }
}

/// A connection is injected so private-bus integration tests cannot reach the owner's session.
#[derive(Clone)]
pub struct Eds {
    connection: Connection,
}

impl Eds {
    pub fn session() -> Result<Self, Error> {
        Ok(Self {
            connection: rmac_dbus::session_blocking()?,
        })
    }

    pub fn from_connection(connection: Connection) -> Self {
        Self { connection }
    }

    /// Check the versioned service names before exposing calendars to the caller.
    pub fn check_available(&self) -> Result<(), Error> {
        let dbus = zbus::blocking::fdo::DBusProxy::new(&self.connection)?;
        if !dbus.name_has_owner(SOURCES_NAME.try_into().map_err(|_| Error::Unavailable)?)?
            && !dbus
                .list_activatable_names()?
                .iter()
                .any(|name| name.as_str() == SOURCES_NAME)
        {
            return Err(Error::Unavailable);
        }
        if !dbus.name_has_owner(FACTORY_NAME.try_into().map_err(|_| Error::Unavailable)?)?
            && !dbus
                .list_activatable_names()?
                .iter()
                .any(|name| name.as_str() == FACTORY_NAME)
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    pub fn sources(&self) -> Result<Vec<Source>, Error> {
        let manager = Proxy::new(
            &self.connection,
            SOURCES_NAME,
            SOURCES_PATH,
            "org.freedesktop.DBus.ObjectManager",
        )?;
        let objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
            manager.call("GetManagedObjects", &())?;
        let mut sources = Vec::new();
        for interfaces in objects.into_values() {
            let Some(properties) = interfaces.get(SOURCE_IFACE) else {
                continue;
            };
            let Some(data) = properties
                .get("Data")
                .and_then(|v| String::try_from(v.clone()).ok())
            else {
                continue;
            };
            let Some(uid) = properties
                .get("UID")
                .and_then(|v| String::try_from(v.clone()).ok())
            else {
                continue;
            };
            if let Some(source) = parse_source(&uid, &data) {
                sources.push(source);
            }
        }
        sources.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.uid.cmp(&b.uid)));
        Ok(sources)
    }

    /// Subscribe before the first source load to avoid missing additions, removals,
    /// renames, and enable/disable changes. The iterator blocks without polling.
    pub fn source_changes(&self) -> Result<SourceChangeStream, Error> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SOURCES_NAME)?
            .path_namespace(SOURCES_PATH)?
            .build();
        let stream = futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
            rule,
            self.connection.inner(),
            Some(16),
        ))?;
        Ok(SourceChangeStream { stream })
    }

    pub fn open(&self, uid: &str) -> Result<Calendar, Error> {
        if uid.is_empty() {
            return Err(Error::InvalidSource);
        }
        let factory = Proxy::new(
            &self.connection,
            FACTORY_NAME,
            FACTORY_PATH,
            "org.gnome.evolution.dataserver.CalendarFactory",
        )?;
        let (bus_name, object_path): (String, String) = factory.call("OpenCalendar", &(uid,))?;
        let calendar = Calendar {
            connection: self.connection.clone(),
            bus_name,
            object_path,
        };
        calendar.proxy()?.call::<_, _, Vec<String>>("Open", &())?;
        Ok(calendar)
    }
}

pub struct SourceChangeStream {
    stream: zbus::MessageStream,
}

impl Iterator for SourceChangeStream {
    type Item = ();
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let message = futures_lite::future::block_on(self.stream.next())?.ok()?;
            let header = message.header();
            let interface = header.interface()?.as_str().to_owned();
            let member = header.member()?.as_str().to_owned();
            if (interface == "org.freedesktop.DBus.ObjectManager"
                && matches!(member.as_str(), "InterfacesAdded" | "InterfacesRemoved"))
                || (interface == "org.freedesktop.DBus.Properties" && member == "PropertiesChanged")
            {
                return Some(());
            }
        }
    }
}

#[derive(Clone)]
pub struct Calendar {
    connection: Connection,
    bus_name: String,
    object_path: String,
}

impl Calendar {
    fn proxy(&self) -> Result<Proxy<'_>, Error> {
        Ok(Proxy::new(
            &self.connection,
            self.bus_name.as_str(),
            self.object_path.as_str(),
            CAL_IFACE,
        )?)
    }

    pub fn online(&self) -> Result<bool, Error> {
        Ok(self.proxy()?.get_property("Online")?)
    }
    pub fn writable(&self) -> Result<bool, Error> {
        Ok(self.proxy()?.get_property("Writable")?)
    }
    pub fn refresh(&self) -> Result<(), Error> {
        Ok(self.proxy()?.call("Refresh", &())?)
    }
    pub fn create(&self, objects: &[String]) -> Result<Vec<String>, Error> {
        Ok(self.proxy()?.call("CreateObjects", &(objects, 0_u32))?)
    }
    pub fn modify(&self, objects: &[String], scope: &str) -> Result<(), Error> {
        Ok(self
            .proxy()?
            .call("ModifyObjects", &(objects, scope, 0_u32))?)
    }
    pub fn remove(&self, ids: &[(String, String)], scope: &str) -> Result<(), Error> {
        Ok(self.proxy()?.call("RemoveObjects", &(ids, scope, 0_u32))?)
    }
    pub fn receive(&self, object: &str) -> Result<(), Error> {
        Ok(self.proxy()?.call("ReceiveObjects", &(object, 0_u32))?)
    }
    pub fn send(&self, object: &str) -> Result<(Vec<String>, String), Error> {
        Ok(self.proxy()?.call("SendObjects", &(object, 0_u32))?)
    }
    pub fn object_list(&self, query: &str) -> Result<Vec<String>, Error> {
        Ok(self.proxy()?.call("GetObjectList", &(query,))?)
    }
    pub fn view(&self, query: &str) -> Result<View, Error> {
        let path: OwnedObjectPath = self.proxy()?.call("GetView", &(query,))?;
        Ok(View {
            connection: self.connection.clone(),
            bus_name: self.bus_name.clone(),
            object_path: path.to_string(),
        })
    }
    pub fn close(&self) -> Result<(), Error> {
        Ok(self.proxy()?.call("Close", &())?)
    }
}

pub struct View {
    connection: Connection,
    bus_name: String,
    object_path: String,
}

impl View {
    fn proxy(&self) -> Result<Proxy<'_>, Error> {
        Ok(Proxy::new(
            &self.connection,
            self.bus_name.as_str(),
            self.object_path.as_str(),
            VIEW_IFACE,
        )?)
    }
    pub fn start(&self) -> Result<(), Error> {
        Ok(self.proxy()?.call("Start", &())?)
    }
    pub fn stop(&self) -> Result<(), Error> {
        Ok(self.proxy()?.call("Stop", &())?)
    }
    pub fn dispose(&self) -> Result<(), Error> {
        Ok(self.proxy()?.call("Dispose", &())?)
    }
    /// Install the signal subscription before Start, so the initial batch is not lost.
    pub fn into_events(self) -> Result<EventStream, Error> {
        let stream = self.proxy()?.receive_all_signals()?;
        self.start()?;
        Ok(EventStream { view: self, stream })
    }
}

pub struct EventStream {
    view: View,
    stream: zbus::blocking::proxy::SignalIterator<'static>,
}

impl Iterator for EventStream {
    type Item = ViewEvent;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let message = self.stream.next()?;
            let member = message.header().member()?.as_str().to_owned();
            let event =
                match member.as_str() {
                    "ObjectsAdded" => message
                        .body()
                        .deserialize::<(Vec<String>,)>()
                        .ok()
                        .map(|v| ViewEvent::Added(v.0)),
                    "ObjectsModified" => message
                        .body()
                        .deserialize::<(Vec<String>,)>()
                        .ok()
                        .map(|v| ViewEvent::Modified(v.0)),
                    "ObjectsRemoved" => message
                        .body()
                        .deserialize::<(Vec<String>,)>()
                        .ok()
                        .map(|v| ViewEvent::Removed(v.0)),
                    "Complete" => message.body().deserialize::<(String, String)>().ok().map(
                        |(name, message)| {
                            ViewEvent::Complete(if name.is_empty() {
                                Ok(())
                            } else {
                                Err(message)
                            })
                        },
                    ),
                    _ => None,
                };
            if event.is_some() {
                return event;
            }
        }
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        let _ = self.view.stop();
        let _ = self.view.dispose();
    }
}

fn parse_source(uid: &str, data: &str) -> Option<Source> {
    let mut section = "";
    let mut name = None;
    let mut parent = None;
    let mut backend = None;
    let mut enabled = true;
    let mut calendar = false;
    for line in data.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            section = value;
            if section == "Calendar" {
                calendar = true;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (section, key) {
            ("Data Source", "DisplayName") => name = Some(value.to_owned()),
            ("Data Source", "Parent") => parent = Some(value.to_owned()),
            ("Data Source", "Enabled") => enabled = value != "false",
            ("Calendar", "BackendName") => backend = Some(value.to_owned()),
            _ => {}
        }
    }
    calendar.then(|| Source {
        uid: uid.to_owned(),
        display_name: name.unwrap_or_else(|| uid.to_owned()),
        parent_uid: parent.filter(|v| !v.is_empty()),
        backend: backend.unwrap_or_default(),
        enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_non_calendar_sources() {
        assert!(parse_source("mail", "[Data Source]\nDisplayName=Mail").is_none());
        let source = parse_source("work", "[Data Source]\nDisplayName=Work\nParent=goa-1\nEnabled=false\n[Calendar]\nBackendName=caldav").unwrap();
        assert_eq!(source.uid, "work");
        assert_eq!(source.parent_uid.as_deref(), Some("goa-1"));
        assert_eq!(source.backend, "caldav");
        assert!(!source.enabled);
    }
}
