//! Non-mutating Wayland preflight for the future session-lock adapter.
//!
//! This module deliberately does not issue `ext_session_lock_manager_v1.lock`.
//! Acquiring a session lock before output surfaces, input, and authentication
//! are wired could leave a development session unusable. The probe proves only
//! that the compositor advertises protocol version 1 and at least one output.

use std::fmt;

use wayland_client::globals::{registry_queue_init, GlobalError, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

#[cfg(test)]
use wayland_client::{protocol::wl_output::WlOutput, Proxy};
#[cfg(test)]
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;

use crate::registry_probe;

/// Capabilities observed in one initial registry snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    session_lock_version: u32,
    output_count: usize,
}

impl Capabilities {
    pub fn session_lock_version(self) -> u32 {
        self.session_lock_version
    }

    pub fn output_count(self) -> usize {
        self.output_count
    }
}

/// Connect to the compositor and inspect its initial global registry.
///
/// This is a read-only preflight. The connection is dropped before returning,
/// no global is bound, and the session is never locked.
pub fn probe() -> Result<Capabilities, Error> {
    let connection = Connection::connect_to_env().map_err(Error::Connect)?;
    let (globals, _event_queue) =
        registry_queue_init::<ProbeState>(&connection).map_err(Error::ReadRegistry)?;

    globals.contents().with_list(classify_globals)
}

fn classify_globals(globals: &[wayland_client::globals::Global]) -> Result<Capabilities, Error> {
    let capabilities = registry_probe::classify(
        globals
            .iter()
            .map(|global| (global.interface.as_str(), global.version)),
    )
    .map_err(|error| match error {
        registry_probe::Error::SessionLockUnavailable => Error::SessionLockUnavailable,
        registry_probe::Error::SessionLockVersion {
            advertised,
            required,
        } => Error::SessionLockVersion {
            advertised,
            required,
        },
        registry_probe::Error::NoOutputs => Error::NoOutputs,
    })?;

    Ok(Capabilities {
        session_lock_version: capabilities.session_lock_version,
        output_count: capabilities.output_count,
    })
}

struct ProbeState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` collects the initial snapshot before returning.
        // A real provider must replace this no-op with hotplug handling.
    }
}

#[derive(Debug)]
pub enum Error {
    Connect(wayland_client::ConnectError),
    ReadRegistry(GlobalError),
    SessionLockUnavailable,
    SessionLockVersion { advertised: u32, required: u32 },
    NoOutputs,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(_) => formatter.write_str("cannot connect to the Wayland compositor"),
            Self::ReadRegistry(_) => formatter.write_str("cannot read the Wayland global registry"),
            Self::SessionLockUnavailable => {
                formatter.write_str("compositor does not advertise ext-session-lock-v1")
            }
            Self::SessionLockVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "compositor advertises session-lock version {advertised}, but version {required} is required"
            ),
            Self::NoOutputs => formatter.write_str("compositor does not advertise any outputs"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) => Some(error),
            Self::ReadRegistry(error) => Some(error),
            Self::SessionLockUnavailable | Self::SessionLockVersion { .. } | Self::NoOutputs => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_generated_protocol_interface_names() {
        assert_eq!(
            ExtSessionLockManagerV1::interface().name,
            registry_probe::MANAGER_INTERFACE
        );
        assert_eq!(WlOutput::interface().name, registry_probe::OUTPUT_INTERFACE);
    }
}
