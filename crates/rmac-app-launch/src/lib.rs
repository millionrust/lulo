//! Shared, private-safe application and document launch routing.
//!
//! On the supported niri session, argv is sent through direct compositor IPC
//! so niri can attach an XDG activation token to the child. Ordinary desktop
//! sessions and transient niri transport loss retain a shell-free direct-spawn
//! fallback. A compositor rejection is never bypassed.

mod application;
mod document;
mod model;
mod recent;
mod termination;

pub use application::launch;
pub use document::{
    all_applications, file_association, open_document, open_file_with, open_item,
    record_recent_document, reveal_application, reveal_item,
};
pub use model::{
    AssociationError, CatalogError, Delivery, Error, ErrorKind, ItemError, ItemOperation,
    OpenWithError, Outcome, RecentDocumentError,
};
pub use recent::{recent_app_ids, record_recent_launch, RecentLaunchError};
pub use termination::{terminate_application, TerminationError, TerminationKind};

#[cfg(test)]
mod tests;
