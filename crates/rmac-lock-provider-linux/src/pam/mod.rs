//! Checked Linux-PAM application boundary.
//!
//! Unsafe code is confined to raw ABI validation, C-owned response allocation,
//! and transaction calls. Successful callback responses transfer to the trusted
//! Linux-PAM stack; all response memory still owned by rmac is overwritten.

mod callback;
mod transaction;

pub use transaction::{spawn_authentication, Error, Stage, Worker, WorkerError};
