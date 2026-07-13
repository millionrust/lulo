//! Linux portal privacy decisions backed by the XDG PermissionStore.

use rmac_privacy::{PortalDecision, PortalResource, Snapshot};
use std::collections::HashMap;
use std::fmt;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const DESTINATION: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const DEVICE_TABLE: &str = "devices";
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";
const RESOURCES: [PortalResource; 2] = [PortalResource::Camera, PortalResource::Microphone];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

enum LookupError {
    NotFound,
    Failed(String),
}

trait Store {
    fn version(&self) -> Result<u32, String>;
    fn lookup(&self, resource: PortalResource)
        -> Result<HashMap<String, Vec<String>>, LookupError>;
    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String>;
}

impl Store for Proxy<'_> {
    fn version(&self) -> Result<u32, String> {
        self.get_property("version")
            .map_err(|error| error.to_string())
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
            error => LookupError::Failed(error.to_string()),
        })
    }

    fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
        self.call("DeletePermission", &(DEVICE_TABLE, resource.id(), app_id))
            .map_err(|error| error.to_string())
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(Snapshot {
                detail: Some(format!("The session D-Bus is unavailable: {error}")),
                ..Snapshot::default()
            });
        }
    };
    let proxy = match Proxy::new(&connection, DESTINATION, PATH, INTERFACE) {
        Ok(proxy) => proxy,
        Err(error) => {
            return Ok(Snapshot {
                detail: Some(format!(
                    "The portal PermissionStore is unavailable: {error}"
                )),
                ..Snapshot::default()
            });
        }
    };
    snapshot_with(&proxy)
}

pub fn reset_decision(resource: PortalResource, app_id: &str) -> Result<Snapshot, Error> {
    validate_app_id(app_id)?;
    let connection = Connection::session()
        .map_err(|error| Error::new("connect to the portal PermissionStore", error.to_string()))?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .map_err(|error| Error::new("open the portal PermissionStore", error.to_string()))?;
    reset_with(&proxy, resource, app_id)
}

fn snapshot_with(store: &impl Store) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    let mut decisions = Vec::new();
    for resource in RESOURCES {
        match store.lookup(resource) {
            Ok(entries) => {
                decisions.extend(
                    entries
                        .into_iter()
                        .map(|(app_id, permissions)| PortalDecision {
                            resource,
                            app_id,
                            permissions,
                        }),
                )
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

fn reset_with(
    store: &impl Store,
    resource: PortalResource,
    app_id: &str,
) -> Result<Snapshot, Error> {
    let version = store
        .version()
        .map_err(|error| Error::new("read PermissionStore version", error))?;
    if version < 2 {
        return Err(Error::new(
            "reset portal permission",
            "PermissionStore version 2 is required",
        ));
    }
    store
        .delete_permission(resource, app_id)
        .map_err(|error| Error::new("reset portal permission", error))?;
    snapshot_with(store)
}

fn validate_app_id(app_id: &str) -> Result<(), Error> {
    if app_id.is_empty() || app_id.len() > 255 || app_id.chars().any(char::is_control) {
        return Err(Error::new(
            "validate portal application ID",
            "the application ID is empty, too long, or contains control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeStore {
        version: u32,
        entries: RefCell<HashMap<PortalResource, HashMap<String, Vec<String>>>>,
        deleted: RefCell<Vec<(PortalResource, String)>>,
    }

    impl Store for FakeStore {
        fn version(&self) -> Result<u32, String> {
            Ok(self.version)
        }

        fn lookup(
            &self,
            resource: PortalResource,
        ) -> Result<HashMap<String, Vec<String>>, LookupError> {
            self.entries
                .borrow()
                .get(&resource)
                .cloned()
                .ok_or(LookupError::NotFound)
        }

        fn delete_permission(&self, resource: PortalResource, app_id: &str) -> Result<(), String> {
            self.deleted
                .borrow_mut()
                .push((resource, app_id.to_string()));
            if let Some(entries) = self.entries.borrow_mut().get_mut(&resource) {
                entries.remove(app_id);
            }
            Ok(())
        }
    }

    fn store(version: u32) -> FakeStore {
        FakeStore {
            version,
            entries: RefCell::new(HashMap::from([
                (
                    PortalResource::Camera,
                    HashMap::from([("org.example.Camera".into(), vec!["yes".into()])]),
                ),
                (
                    PortalResource::Microphone,
                    HashMap::from([("org.example.Chat".into(), vec!["no".into()])]),
                ),
            ])),
            deleted: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn snapshot_is_sorted_and_preserves_raw_permission_tokens() {
        let snapshot = snapshot_with(&store(2)).unwrap();
        assert!(snapshot.available);
        assert!(snapshot.can_reset);
        assert_eq!(snapshot.decisions.len(), 2);
        assert_eq!(snapshot.decisions[0].resource, PortalResource::Camera);
        assert_eq!(snapshot.decisions[0].permissions, ["yes"]);
    }

    #[test]
    fn version_one_is_visible_but_not_resettable() {
        let snapshot = snapshot_with(&store(1)).unwrap();
        assert!(snapshot.available);
        assert!(!snapshot.can_reset);
        assert!(snapshot.detail.unwrap().contains("version 2"));
    }

    #[test]
    fn reset_deletes_only_the_selected_app_resource_pair_and_resamples() {
        let store = store(2);
        let snapshot = reset_with(&store, PortalResource::Camera, "org.example.Camera").unwrap();
        assert_eq!(
            store.deleted.borrow().as_slice(),
            &[(PortalResource::Camera, "org.example.Camera".into())]
        );
        assert_eq!(snapshot.decisions.len(), 1);
        assert_eq!(snapshot.decisions[0].resource, PortalResource::Microphone);
    }

    #[test]
    fn reset_requires_version_two_and_a_bounded_app_id() {
        assert!(reset_with(&store(1), PortalResource::Camera, "org.example.Camera").is_err());
        assert!(validate_app_id("").is_err());
        assert!(validate_app_id("org.example\nBad").is_err());
    }
}
