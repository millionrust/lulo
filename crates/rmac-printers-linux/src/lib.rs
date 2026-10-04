//! Printers for System Settings ▸ Printers & Scanners.
//!
//! * [`cups`] reads printers and print queues straight from cupsd's local
//!   socket with a small IPP client ([`ipp`]); reading needs no privilege.
//! * [`admin`] adds, removes, pauses and resumes printers and cancels jobs
//!   through cups-pk-helper, so polkit authorises every change.
//! * [`prefs`] keeps the per-user default printer and paper size in the
//!   files CUPS and libpaper already read.
//!
//! All calls block; callers own a worker thread. No new dependency: the IPP
//! subset is implemented here and D-Bus uses the workspace's zbus.

pub mod admin;
pub mod cups;
pub mod ipp;
pub mod model;
pub mod prefs;

pub use admin::PrinterAdmin;
pub use cups::Cups;
pub use model::{Device, Job, JobState, PaperSize, Printer, PrinterState};
pub use prefs::Preferences;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// cupsd or cups-pk-helper is not running.
    Unavailable,
    /// polkit (or CUPS) refused the change.
    NotAuthorized,
    Failed,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Error::Unavailable => {
                "Printing is unavailable. Check that CUPS is installed and running."
            }
            Error::NotAuthorized => "You need an administrator’s authorisation to change printers.",
            Error::Failed => "The printer change couldn’t be made. Try again.",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for Error {}
