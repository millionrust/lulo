use std::path::{Path, PathBuf};

use crate::{AssociationError, ItemError, ItemOperation, OpenWithError, RecentDocumentError};

/// Open one local item through the user-mediated desktop boundary.
///
/// The underlying portal may retain a private path and diagnostic, but neither
/// crosses this shared application boundary.
pub async fn open_item(path: PathBuf) -> Result<(), ItemError> {
    rmac_portal::open_item(&path).await.map_err(|_| ItemError {
        operation: ItemOperation::Open,
    })?;
    let _ = record_recent_document(path).await;
    Ok(())
}

/// Open one existing local regular document selected through a trusted
/// cross-application boundary.
///
/// Notification targets and similar deferred requests must not turn a
/// directory, relative path, or final symlink into a document activation.
pub async fn open_document(path: PathBuf) -> Result<(), ItemError> {
    let candidate = path.clone();
    let valid = blocking::unblock(move || is_regular_document(&candidate)).await;
    if !valid {
        return Err(ItemError {
            operation: ItemOperation::Open,
        });
    }
    open_item(path).await
}

/// Reveal one local item in the platform file manager.
///
/// Callers receive only the operation class so a private path or portal detail
/// cannot accidentally enter a launcher, notification, or application UI.
pub async fn reveal_item(path: PathBuf) -> Result<(), ItemError> {
    rmac_portal::show_item(&path).await.map_err(|_| ItemError {
        operation: ItemOperation::Reveal,
    })
}

/// Reveal the trusted source for one catalog application.
pub async fn reveal_application(application: rmac_apps::Application) -> Result<(), ItemError> {
    reveal_item(application.source).await
}

/// Resolve current XDG MIME handlers away from the UI executor.
pub async fn file_association(
    path: PathBuf,
) -> Result<rmac_apps::FileAssociation, AssociationError> {
    blocking::unblock(move || rmac_apps::file_association(&path))
        .await
        .map_err(|_| AssociationError)
}

/// Revalidate and open one file with an exact compatible desktop application.
///
/// The catalog authority still distinguishes a successfully retained default
/// change from a later launch failure. All private command and path details are
/// reduced before returning to application UI.
pub async fn open_file_with(
    path: PathBuf,
    expected_mime_type: String,
    application_id: String,
    make_default: bool,
) -> Result<(), OpenWithError> {
    blocking::unblock(move || {
        let result =
            rmac_apps::open_file_with(&path, &expected_mime_type, &application_id, make_default);
        if result.is_ok() {
            let _ = rmac_recent_documents::Store::from_environment()
                .and_then(|store| store.record(&path));
        }
        result
    })
    .await
    .map_err(|error| OpenWithError {
        default_changed: error.default_changed,
    })
}

/// Record one successfully opened or saved local document.
///
/// The store validates and canonicalizes a regular file, performs its
/// cross-process transaction off the async executor, and returns only
/// private-safe diagnostics.
pub async fn record_recent_document(
    path: PathBuf,
) -> Result<rmac_recent_documents::RecordOutcome, RecentDocumentError> {
    blocking::unblock(move || {
        rmac_recent_documents::Store::from_environment().and_then(|store| store.record(&path))
    })
    .await
    .map_err(|_| RecentDocumentError)
}

pub(crate) fn is_regular_document(path: &Path) -> bool {
    path.is_absolute()
        && std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}
