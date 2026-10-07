//! The `org.rmac.Intelligence1` service (ADR 0024 §4).
//!
//! - D-Bus activated (`org.rmac.Intelligence1.service` with
//!   `SystemdService=rmac-intelligence.service`); nothing starts it at login.
//! - Exits [`rmac_intelligence::IDLE_EXIT_SECONDS`] after its last request,
//!   so idle cost is exactly zero. The only timer is that one deadline: it
//!   never polls.
//! - Runs only the closed task list ([`rmac_intelligence::Task`]) for
//!   same-user Lulo programs, only while the user has it turned on, and
//!   only on a model whose size and SHA-256 match the manifest.
//! - The model, its context and the saved prompt-prefix state live on one
//!   worker thread; D-Bus calls queue jobs to it.

pub mod caller;
pub mod engine;
pub mod fixture;
#[cfg(target_os = "linux")]
pub mod llama;
pub mod service;
