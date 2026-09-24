use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    CompositorActivation,
    DirectFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Outcome {
    pub process_id: Option<u32>,
    pub delivery: Delivery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(std::io::ErrorKind),
    Rejected,
    Protocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::Io(std::io::ErrorKind::NotFound) => {
                "the application executable is unavailable"
            }
            ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => {
                "permission to start the application was denied"
            }
            ErrorKind::Io(_) => "the application could not be started",
            ErrorKind::Rejected => "the compositor rejected application startup",
            ErrorKind::Protocol => "the compositor could not accept application startup",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemOperation {
    Open,
    Reveal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ItemError {
    pub operation: ItemOperation,
}

impl fmt::Display for ItemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.operation {
            ItemOperation::Open => "the item could not be opened",
            ItemOperation::Reveal => "the item could not be revealed",
        })
    }
}

impl std::error::Error for ItemError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssociationError;

impl fmt::Display for AssociationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("compatible applications could not be loaded")
    }
}

impl std::error::Error for AssociationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogError;

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the application catalog could not be loaded")
    }
}

impl std::error::Error for CatalogError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenWithError {
    /// The XDG default changed successfully before application startup failed.
    pub default_changed: bool,
}

impl fmt::Display for OpenWithError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.default_changed {
            "the default application changed, but the file could not be opened"
        } else {
            "the file could not be opened with the selected application"
        })
    }
}

impl std::error::Error for OpenWithError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecentDocumentError;

impl fmt::Display for RecentDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the document could not be added to Recents")
    }
}

impl std::error::Error for RecentDocumentError {}
