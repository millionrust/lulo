//! Lulo Intelligence (ADR 0024), everything except the model runtime.
//!
//! - [`Task`]: the closed list of jobs the `org.rmac.Intelligence1` service
//!   accepts. There is no free-form prompt API.
//! - [`Intent`]: the typed action a Spotlight request maps to, its strict
//!   JSON wire form, the row it shows and the confirmation tier.
//! - [`prompt`] and [`decode`]: the fixed system prompt and the
//!   schema-guided decoder that makes invalid output impossible.
//! - [`manifest`]: the checksum-pinned models; [`gate`]: which one a PC
//!   gets; [`config`]: the user's choice; [`fetch`]: the verified download.
//! - [`guard`]: deterministic checks on the model's answer; [`fuzzy`]:
//!   light typo correction for app names.
//! - `client` (feature `client`): the session-bus client.

pub mod config;
pub mod decode;
pub mod eval;
pub mod fetch;
pub mod fuzzy;
pub mod gate;
pub mod guard;
mod intent;
pub mod manifest;
pub mod paths;
pub mod prefix_state;
pub mod prompt;
mod task;
pub mod verify;

#[cfg(feature = "client")]
pub mod client;

pub use intent::*;
pub use task::*;

/// The well-known bus name, object path and interface of the service.
pub const BUS_NAME: &str = "org.rmac.Intelligence1";
pub const OBJECT_PATH: &str = "/org/rmac/Intelligence1";
pub const INTERFACE_NAME: &str = "org.rmac.Intelligence1";

/// The longest request text the intent task reads, in bytes. A Spotlight
/// query is a sentence; anything longer is not a command.
pub const MAX_REQUEST_BYTES: usize = 200;

/// The service exits this long after its last request (ADR 0024 §4), so
/// idle cost is zero: no process, no memory, no wake-ups.
pub const IDLE_EXIT_SECONDS: u64 = 60;
