//! One zbus connection per bus, shared by every client and watcher in this
//! process, instead of a fresh connection for each of them.
//!
//! Before this crate existed, a single System Settings process opened
//! about nine system-bus connections and several session-bus ones — one
//! per crate that talked to NetworkManager, UPower, logind,
//! AccountsService, localed, bluez, and so on — which pushed the user over
//! the bus's `max_connections_per_user` (SES-02). Any rmac crate that
//! wants the system or session bus should call [`system`]/[`session`] (or
//! their blocking equivalents) instead of `zbus::Connection::system()` /
//! `zbus::Connection::session()` directly.
//!
//! The connection is opened once per bus and kept for the life of the
//! process; every later call gets a clone of it (zbus connections are
//! cheap, `Arc`-backed handles). Real D-Bus services only exist on Linux,
//! but several callers of this crate compile unconditionally (their own
//! platform split happens elsewhere, or they fall back gracefully at
//! runtime), so this crate builds — and its cache is exercised by its own
//! tests — on every platform rather than gating itself behind Linux.

mod shared;

pub use shared::{session, session_blocking, system, system_blocking};
