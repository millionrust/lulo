//! XDG PermissionStore snapshot, reset, and watch authority.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(super) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: bounded_text(&detail.into(), MAX_ERROR_BYTES),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub(super) enum LookupError {
    NotFound,
    Failed(String),
}

pub(super) trait Store {
    fn version(&self) -> Result<u32, String>;
    fn lookup(&self, resource: PortalResource)
        -> Result<HashMap<String, Vec<String>>, LookupError>;
    fn get_permission(
        &self,
        resource: PortalResource,
        app_id: &str,
    ) -> Result<Vec<String>, LookupError>;
    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String>;
}

impl Store for Proxy<'_> {
    fn version(&self) -> Result<u32, String> {
        self.get_property("version")
            .map_err(|_| "PermissionStore did not provide its interface version".to_string())
    }

    fn lookup(
        &self,
        resource: PortalResource,
    ) -> Result<HashMap<String, Vec<String>>, LookupError> {
        self.call::<_, _, (HashMap<String, Vec<String>>, OwnedValue)>(
            "Lookup",
            &(DEVICE_TABLE, resource.id()),
        )
        .map(|(permissions, _)| permissions)
        .map_err(|error| match error {
            zbus::Error::MethodError(name, _, _) if name.as_str() == NOT_FOUND => {
                LookupError::NotFound
            }
            _ => LookupError::Failed(
                "PermissionStore could not read a device decision table".to_string(),
            ),
        })
    }

    fn get_permission(
        &self,
        resource: PortalResource,
        app_id: &str,
    ) -> Result<Vec<String>, LookupError> {
        self.call("GetPermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|error| match error {
                zbus::Error::MethodError(name, _, _) if name.as_str() == NOT_FOUND => {
                    LookupError::NotFound
                }
                _ => LookupError::Failed(
                    "PermissionStore could not read the selected decision".to_string(),
                ),
            })
    }

    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
        self.call("DeletePermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|_| "PermissionStore rejected the decision reset".to_string())
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(_) => {
            return Ok(Snapshot {
                detail: Some("The session D-Bus is unavailable.".into()),
                ..Snapshot::default()
            });
        }
    };
    let proxy = match Proxy::new(&connection, DESTINATION, PATH, INTERFACE) {
        Ok(proxy) => proxy,
        Err(_) => {
            return Ok(Snapshot {
                detail: Some("The portal PermissionStore is unavailable.".into()),
                ..Snapshot::default()
            });
        }
    };
    snapshot_with(&proxy)
}

pub fn reset_decision(expected: &PortalDecision) -> Result<Snapshot, Error> {
    validate_decision(expected)?;
    let connection = Connection::session().map_err(|_| {
        Error::new(
            "connect to the portal PermissionStore",
            "session D-Bus is unavailable",
        )
    })?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE).map_err(|_| {
        Error::new(
            "open the portal PermissionStore",
            "PermissionStore is unavailable",
        )
    })?;
    reset_with(&proxy, expected)
}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<rmac_privacy::WatchEvent>) -> Result<(), Error> {
    loop {
        match watch_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                if sender
                    .send(rmac_privacy::WatchEvent::Unavailable)
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }
        async_io::Timer::after(RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(sender: async_channel::Sender<rmac_privacy::WatchEvent>) -> Result<(), Error> {
    sender
        .send(rmac_privacy::WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch portal permissions", "the watcher closed"))
}

#[cfg(target_os = "linux")]
pub(super) async fn watch_once(
    sender: &async_channel::Sender<rmac_privacy::WatchEvent>,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::session()
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    let changed_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(DESTINATION)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .path(PATH)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .interface(INTERFACE)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .member("Changed")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .add_arg(DESTINATION)
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .build();
    let mut changes = MessageStream::for_match_rule(changed_rule, &connection, Some(16))
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(4))
        .await
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?
        .fuse();

    sender
        .send(rmac_privacy::WatchEvent::Changed)
        .await
        .map_err(|_| Error::new("watch portal permissions", "the watcher closed"))?;

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = changes.next() => {
                message
                    .ok_or_else(|| Error::new("watch portal permissions", "the change stream ended"))?
                    .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
                rmac_privacy::WatchEvent::Changed
            },
            message = owners.next() => owner_event_from_message(message)?,
            _ = closed => return Ok(()),
        };
        let _ = sender.try_send(event);
    }
}

#[cfg(target_os = "linux")]
pub(super) fn owner_event_from_message(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<rmac_privacy::WatchEvent, Error> {
    let message = message
        .ok_or_else(|| Error::new("watch portal permissions", "the owner stream ended"))?
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("watch portal permissions", error.to_string()))?;
    permission_store_owner_event(&name, &new_owner)
        .ok_or_else(|| Error::new("watch portal permissions", "an unrelated owner changed"))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn permission_store_owner_event(
    name: &str,
    new_owner: &str,
) -> Option<rmac_privacy::WatchEvent> {
    (name == DESTINATION).then_some(if new_owner.is_empty() {
        rmac_privacy::WatchEvent::Unavailable
    } else {
        rmac_privacy::WatchEvent::Changed
    })
}

pub(super) fn snapshot_with(store: &impl Store) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    let mut decisions = Vec::new();
    for resource in RESOURCES {
        match store.lookup(resource) {
            Ok(entries) => {
                if decisions.len().saturating_add(entries.len()) > MAX_DECISIONS {
                    return Err(Error::new(
                        "read portal device permissions",
                        "PermissionStore returned too many decisions",
                    ));
                }
                for (app_id, permissions) in entries {
                    let decision = PortalDecision {
                        resource,
                        app_id,
                        permissions,
                    };
                    validate_decision(&decision)?;
                    decisions.push(decision);
                }
            }
            Err(LookupError::NotFound) => {}
            Err(LookupError::Failed(error)) => {
                return Err(Error::new("read portal device permissions", error));
            }
        }
    }
    decisions
        .sort_by(|left, right| (left.resource, &left.app_id).cmp(&(right.resource, &right.app_id)));
    Ok(Snapshot {
        available: true,
        version,
        can_reset: version >= 2,
        decisions,
        detail: (version < 2).then(|| {
            "PermissionStore version 2 is required to reset individual decisions".to_string()
        }),
    })
}

pub(super) fn reset_with(store: &impl Store, expected: &PortalDecision) -> Result<Snapshot, Error> {
    validate_decision(expected)?;
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    if version < 2 {
        return Err(Error::new(
            "reset portal permission",
            "PermissionStore version 2 is required",
        ));
    }
    let current_permissions = match store.get_permission(expected.resource, &expected.app_id) {
        Ok(permissions) => permissions,
        Err(LookupError::NotFound) => {
            return Err(Error::new(
                "reset portal permission",
                "the selected decision no longer exists; refresh and try again",
            ));
        }
        Err(LookupError::Failed(_)) => {
            return Err(Error::new(
                "reset portal permission",
                "the selected decision could not be revalidated",
            ));
        }
    };
    let current = PortalDecision {
        resource: expected.resource,
        app_id: expected.app_id.clone(),
        permissions: current_permissions,
    };
    validate_decision(&current)?;
    if current != *expected {
        return Err(Error::new(
            "reset portal permission",
            "the selected decision changed before reset; refresh and try again",
        ));
    }
    store
        .delete_permission(expected.resource, &expected.app_id)
        .map_err(|error| Error::new("reset portal permission", error))?;
    let snapshot = snapshot_with(store)?;
    if snapshot.decisions.iter().any(|decision| {
        decision.resource == expected.resource && decision.app_id == expected.app_id
    }) {
        return Err(Error::new(
            "reset portal permission",
            "the selected decision remained after reset",
        ));
    }
    Ok(snapshot)
}

pub(super) fn validate_app_id(app_id: &str) -> Result<(), Error> {
    if app_id.is_empty() || app_id.len() > 255 || app_id.chars().any(char::is_control) {
        return Err(Error::new(
            "validate portal application ID",
            "the application ID is empty, too long, or contains control characters",
        ));
    }
    Ok(())
}

pub(super) fn validate_decision(decision: &PortalDecision) -> Result<(), Error> {
    validate_app_id(&decision.app_id)?;
    if decision.permissions.len() > MAX_PERMISSIONS_PER_DECISION
        || decision
            .permissions
            .iter()
            .map(String::len)
            .fold(0_usize, usize::saturating_add)
            .saturating_add(decision.permissions.len().saturating_sub(1) * 2)
            > MAX_PERMISSION_SUMMARY_BYTES
        || decision.permissions.iter().any(|permission| {
            permission.len() > MAX_PERMISSION_TOKEN_BYTES
                || permission.chars().any(char::is_control)
        })
    {
        return Err(Error::new(
            "validate portal decision",
            "the permission tokens exceed the safe display bounds",
        ));
    }
    Ok(())
}
