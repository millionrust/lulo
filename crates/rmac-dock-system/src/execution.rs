use crate::{Backend, Error, FailureKind, Operation, Outcome};

/// Execute one already-resolved Dock activation. Success never changes the
/// Dock model directly; catalog and compositor events remain authoritative.
pub async fn execute(
    activation: &rmac_dock::Activation,
    request_id: rmac_compositor::ActivationId,
    backend: &impl Backend,
) -> Result<Outcome, Error> {
    match activation {
        rmac_dock::Activation::Launch { app_id, spec } => backend
            .launch(spec)
            .await
            .map(|launch| Outcome::Launched {
                app_id: app_id.clone(),
                launch,
            })
            .map_err(|error| Error::new(Operation::Launch, error.kind, app_id, error.detail)),
        rmac_dock::Activation::FocusWindow(window) => backend
            .focus_window(request_id, *window)
            .await
            .map(|()| Outcome::FocusRequested { window: *window })
            .map_err(|error| {
                Error::new(
                    Operation::Focus,
                    error.kind,
                    format!("window {}", window.0),
                    error.detail,
                )
            }),
        rmac_dock::Activation::NoAction => Ok(Outcome::NoAction),
        rmac_dock::Activation::Unavailable { app_id, detail } => Err(Error::new(
            Operation::Resolve,
            FailureKind::Unsupported,
            app_id,
            detail,
        )),
    }
}

/// Execute one context-menu action. Window and pin state still changes only
/// when the niri/settings watchers publish their authoritative result.
pub async fn execute_context(
    action: &rmac_dock::ContextAction,
    request_id: rmac_compositor::ActivationId,
    backend: &impl Backend,
) -> Result<Outcome, Error> {
    match action {
        rmac_dock::ContextAction::LaunchNew { app_id, spec } => backend
            .launch(spec)
            .await
            .map(|launch| Outcome::Launched {
                app_id: app_id.clone(),
                launch,
            })
            .map_err(|error| Error::new(Operation::Launch, error.kind, app_id, error.detail)),
        rmac_dock::ContextAction::FocusWindow { app_id, window } => backend
            .focus_window(request_id, *window)
            .await
            .map(|()| Outcome::FocusRequested { window: *window })
            .map_err(|error| Error::new(Operation::Focus, error.kind, app_id, error.detail)),
        rmac_dock::ContextAction::CloseWindow { app_id, window } => backend
            .close_window(request_id, *window)
            .await
            .map(|()| Outcome::CloseRequested { window: *window })
            .map_err(|error| Error::new(Operation::Close, error.kind, app_id, error.detail)),
        rmac_dock::ContextAction::RevealApplication { app_id, source } => backend
            .reveal_application(source)
            .await
            .map(|()| Outcome::ApplicationRevealed {
                app_id: app_id.clone(),
            })
            .map_err(|error| Error::new(Operation::Reveal, error.kind, app_id, error.detail)),
        rmac_dock::ContextAction::TerminateApplication { app_id, pids, kind } => backend
            .terminate_application(pids, *kind)
            .await
            .map(|()| Outcome::TerminationRequested {
                app_id: app_id.clone(),
                kind: *kind,
            })
            .map_err(|error| Error::new(Operation::Terminate, error.kind, app_id, error.detail)),
        rmac_dock::ContextAction::UpdatePins(command) => backend
            .update_pins(command)
            .await
            .map(|pinned| Outcome::PinsUpdated { pinned })
            .map_err(|error| {
                Error::new(
                    Operation::UpdatePins,
                    error.kind,
                    command.app_id(),
                    error.detail,
                )
            }),
    }
}

/// Execute an already-projected Files, Downloads, or Trash activation. The
/// activation retains private paths, while receipts and default Debug output
/// contain only the public special-item identity.
pub async fn execute_special(
    activation: &rmac_dock::SpecialActivation,
    backend: &impl Backend,
) -> Result<Outcome, Error> {
    match activation {
        rmac_dock::SpecialActivation::OpenDirectory { kind, path } => backend
            .open_directory(path)
            .await
            .map(|()| Outcome::PlaceOpened { kind: *kind })
            .map_err(|error| {
                Error::new(
                    Operation::OpenPlace,
                    error.kind,
                    special_item_id(*kind),
                    error.detail,
                )
            }),
        rmac_dock::SpecialActivation::OpenTrash => backend
            .open_trash()
            .await
            .map(|()| Outcome::PlaceOpened {
                kind: rmac_dock::SpecialItemKind::Trash,
            })
            .map_err(|error| {
                Error::new(
                    Operation::OpenPlace,
                    error.kind,
                    special_item_id(rmac_dock::SpecialItemKind::Trash),
                    error.detail,
                )
            }),
        rmac_dock::SpecialActivation::Unavailable { kind, detail } => Err(Error::new(
            Operation::Resolve,
            FailureKind::Unavailable,
            special_item_id(*kind),
            detail,
        )),
    }
}

fn special_item_id(kind: rmac_dock::SpecialItemKind) -> &'static str {
    match kind {
        rmac_dock::SpecialItemKind::Files => "Files",
        rmac_dock::SpecialItemKind::Downloads => "Downloads",
        rmac_dock::SpecialItemKind::Trash => "Trash",
    }
}

/// Revalidate the count projected into the context menu and retain the exact,
/// path-free Trash identities that may be deleted after explicit confirmation.
/// This is blocking filesystem work and belongs on a worker thread.
pub fn prepare_special_context(
    action: &rmac_dock::SpecialContextAction,
    backend: &impl rmac_places_system::Backend,
) -> Result<rmac_places_system::EmptyTrashReview, Error> {
    match action {
        rmac_dock::SpecialContextAction::EmptyTrash {
            expected_item_count,
        } => {
            let snapshot = rmac_places::TrashSnapshot {
                available: true,
                empty: *expected_item_count == 0,
                item_count: *expected_item_count,
            };
            rmac_places_system::prepare_empty_trash(&snapshot, backend)
                .map_err(|_| {
                    Error::new(
                        Operation::ReviewTrash,
                        FailureKind::Rejected,
                        "Trash",
                        "Trash changed before the deletion review",
                    )
                })?
                .ok_or_else(|| {
                    Error::new(
                        Operation::ReviewTrash,
                        FailureKind::Rejected,
                        "Trash",
                        "Trash is already empty",
                    )
                })
        }
    }
}

/// Permanently delete only the exact identities retained by the confirmed
/// review. Items added later remain in Trash. This is blocking filesystem work
/// and belongs on a worker thread.
pub fn execute_empty_trash(
    confirmation: rmac_places_system::EmptyTrashConfirmation,
    backend: &impl rmac_places_system::Backend,
) -> Result<Outcome, Error> {
    rmac_places_system::empty_trash(confirmation, backend)
        .map(|snapshot| Outcome::TrashEmptied {
            remaining_items: snapshot.item_count,
        })
        .map_err(|_| {
            Error::new(
                Operation::EmptyTrash,
                FailureKind::Rejected,
                "Trash",
                "Trash changed or could not be emptied",
            )
        })
}
