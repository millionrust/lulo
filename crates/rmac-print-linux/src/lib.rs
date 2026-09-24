//! Linux XDG Print portal transport for preformatted rmac PDF documents.
//!
//! The platform portal receives a readable file descriptor, not source text.
//! This adapter exports the exact initiating Wayland surface, requests only
//! PDF output, renders off-thread, and rejects a document generation that
//! changes between either portal boundary.

#![cfg_attr(not(unix), allow(dead_code))]

#[cfg(unix)]
mod linux;

#[cfg(unix)]
pub use linux::{
    print_document, print_prepared_document, Error, Outcome, PreparedPrintDocument, PrintDocument,
};
