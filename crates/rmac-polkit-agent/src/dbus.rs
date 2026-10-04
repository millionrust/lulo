//! The D-Bus side: register with `org.freedesktop.PolicyKit1.Authority` for
//! the graphical session and export `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! Only polkitd may drive the agent. Its unique name is taken from the reply
//! to our registration (so a request that arrives right after registering
//! is accepted) and refreshed when polkitd restarts, when the agent
//! registers again. Every call from any other sender is refused.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_lite::StreamExt as _;
use zbus::message::Header;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use crate::identity::WireIdentity;
use crate::request::{Coordinator, Outcome, Request};

pub const AGENT_PATH: &str = "/org/rmac/PolicyKit1/AuthenticationAgent";
pub const AUTHORITY_NAME: &str = "org.freedesktop.PolicyKit1";
pub const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
pub const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
    NotAuthorized(String),
}

pub struct Agent {
    coordinator: Arc<Coordinator>,
    authority: Arc<Mutex<Option<String>>>,
}

impl Agent {
    fn check_caller(&self, header: &Header<'_>) -> Result<(), AgentError> {
        let sender = header.sender().map(|name| name.as_str());
        let authority = self.authority.lock().unwrap();
        if sender.is_some() && sender == authority.as_deref() {
            Ok(())
        } else {
            Err(AgentError::NotAuthorized(
                "Only the polkit authority may use this agent".into(),
            ))
        }
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] header: Header<'_>,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<WireIdentity>,
    ) -> Result<(), AgentError> {
        self.check_caller(&header)?;
        let request = Request {
            action_id,
            message,
            icon_name,
            details,
            cookie,
            identities,
        };
        match self.coordinator.begin(request).await {
            Outcome::Authorized => Ok(()),
            Outcome::Cancelled => Err(AgentError::Cancelled(
                "The authentication dialog was dismissed".into(),
            )),
            Outcome::Failed => Err(AgentError::Failed("Authentication failed".into())),
        }
    }

    async fn cancel_authentication(
        &self,
        #[zbus(header)] header: Header<'_>,
        cookie: String,
    ) -> Result<(), AgentError> {
        self.check_caller(&header)?;
        self.coordinator.cancel(&cookie);
        Ok(())
    }
}

/// The `unix-session` subject polkit registers agents for.
pub type Subject = (String, HashMap<String, Value<'static>>);

pub fn session_subject(session_id: &str) -> Subject {
    (
        "unix-session".to_owned(),
        HashMap::from([("session-id".to_owned(), Value::from(session_id.to_owned()))]),
    )
}

fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "en_US.UTF-8".to_owned())
}

/// Register for `subject`, returning polkitd's unique name.
pub async fn register(connection: &Connection, subject: &Subject) -> zbus::Result<String> {
    let options: HashMap<String, Value<'static>> = HashMap::new();
    let reply = match connection
        .call_method(
            Some(AUTHORITY_NAME),
            AUTHORITY_PATH,
            Some(AUTHORITY_INTERFACE),
            "RegisterAuthenticationAgentWithOptions",
            &(subject, locale(), AGENT_PATH, options),
        )
        .await
    {
        Ok(reply) => reply,
        // polkit before 0.104 has only the plain call.
        Err(_) => {
            connection
                .call_method(
                    Some(AUTHORITY_NAME),
                    AUTHORITY_PATH,
                    Some(AUTHORITY_INTERFACE),
                    "RegisterAuthenticationAgent",
                    &(subject, locale(), AGENT_PATH),
                )
                .await?
        }
    };
    reply
        .header()
        .sender()
        .map(|name| name.to_string())
        .ok_or_else(|| zbus::Error::Failure("the registration reply has no sender".into()))
}

async fn property(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Option<OwnedValue> {
    connection
        .call_method(
            Some(destination),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(interface, name),
        )
        .await
        .ok()?
        .body()
        .deserialize::<OwnedValue>()
        .ok()
}

fn plain_session_id(id: &str) -> Option<String> {
    (!id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| id.to_owned())
}

/// The graphical session this agent serves: `XDG_SESSION_ID` when the
/// session exported it, else logind's `session/auto` (the caller's session,
/// or the user's display session for a user service), else the user's
/// `Display` session.
pub async fn session_id(system: &Connection) -> Option<String> {
    if let Some(id) = std::env::var("XDG_SESSION_ID")
        .ok()
        .and_then(|id| plain_session_id(&id))
    {
        return Some(id);
    }
    const LOGIN1: &str = "org.freedesktop.login1";
    if let Some(value) = property(
        system,
        LOGIN1,
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
        "Id",
    )
    .await
    {
        if let Value::Str(id) = &*value {
            if let Some(id) = plain_session_id(id.as_str()) {
                return Some(id);
            }
        }
    }
    let value = property(
        system,
        LOGIN1,
        "/org/freedesktop/login1/user/self",
        "org.freedesktop.login1.User",
        "Display",
    )
    .await?;
    let Value::Structure(display) = &*value else {
        return None;
    };
    match display.fields().first() {
        Some(Value::Str(id)) => plain_session_id(id.as_str()),
        _ => None,
    }
}

/// Export the agent, register it, and keep it registered across polkitd
/// restarts. Runs until the connection closes.
pub async fn serve(
    connection: &Connection,
    subject: Subject,
    coordinator: Arc<Coordinator>,
) -> zbus::Result<()> {
    let authority = Arc::new(Mutex::new(None));
    connection
        .object_server()
        .at(
            AGENT_PATH,
            Agent {
                coordinator,
                authority: authority.clone(),
            },
        )
        .await?;
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, AUTHORITY_NAME)])
        .await?;
    let owner = register(connection, &subject).await?;
    *authority.lock().unwrap() = Some(owner);
    eprintln!("registered as the polkit authentication agent");

    while let Some(signal) = owners.next().await {
        let Ok(args) = signal.args() else {
            continue;
        };
        let new_owner = args.new_owner().as_ref().map(|name| name.to_string());
        *authority.lock().unwrap() = None;
        if new_owner.is_some() {
            match register(connection, &subject).await {
                Ok(owner) => *authority.lock().unwrap() = Some(owner),
                Err(error) => eprintln!("could not register again with polkit: {error}"),
            }
        }
    }
    Ok(())
}
