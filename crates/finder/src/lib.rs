//! Framework-neutral contracts for the Files application.

pub mod accessibility;
pub mod goto;
pub mod listing;
pub mod places;
// Files' Wayland clipboard file-copy path (ADR 0011). Public so other
// surfaces that need the same "Copy" behaviour for filesystem items — the
// desktop's context menu (DESK-01) — reuse it rather than re-implementing
// wl-clipboard handling.
pub mod pasteboard;
