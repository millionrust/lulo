//! Typed launch and niri-focus execution for Dock activation outcomes.

pub mod dispatch;
pub mod icons;
pub mod interaction;

use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Launch,
    Focus,
    Close,
    UpdatePins,
    Resolve,
    OpenPlace,
    ReviewTrash,
    EmptyTrash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    Io(std::io::ErrorKind),
    Unavailable,
    Transport,
    Protocol,
    Rejected,
    Unsupported,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    pub kind: FailureKind,
    pub detail: String,
}

impl BackendError {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Launch => "launch application",
            Self::Focus => "focus application window",
            Self::Close => "close application window",
            Self::UpdatePins => "update pinned applications",
            Self::Resolve => "resolve Dock activation",
            Self::OpenPlace => "open Dock place",
            Self::ReviewTrash => "review Empty Trash",
            Self::EmptyTrash => "empty Trash",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: FailureKind,
    pub app_id: String,
    detail: String,
}

impl Error {
    fn new(
        operation: Operation,
        kind: FailureKind,
        app_id: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            kind,
            app_id: app_id.into(),
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not {} {}: {}",
            self.operation, self.app_id, self.detail
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Launched {
        app_id: String,
        launch: rmac_app_launch::Outcome,
    },
    FocusRequested {
        window: rmac_compositor::WindowId,
    },
    CloseRequested {
        window: rmac_compositor::WindowId,
    },
    PinsUpdated {
        pinned: Vec<rmac_shell_settings::AppId>,
    },
    PlaceOpened {
        kind: rmac_dock::SpecialItemKind,
    },
    TrashEmptied {
        remaining_items: usize,
    },
    NoAction,
}

pub trait Backend: Send + Sync + 'static {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>>;

    fn focus_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>>;

    fn close_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>>;

    fn update_pins(
        &self,
        command: &rmac_dock::PinCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    fn reorder_pins(
        &self,
        reorder: &rmac_dock::drag::RevalidatedReorder,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>>;

    fn open_directory(&self, path: &Path) -> BackendFuture<'_, Result<(), BackendError>>;

    fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>> {
        let spec = spec.clone();
        Box::pin(async move {
            rmac_app_launch::launch(spec).await.map_err(|error| {
                BackendError::new(
                    match error.kind {
                        rmac_app_launch::ErrorKind::Io(kind) => FailureKind::Io(kind),
                        rmac_app_launch::ErrorKind::Rejected => FailureKind::Rejected,
                        rmac_app_launch::ErrorKind::Protocol => FailureKind::Protocol,
                    },
                    error.to_string(),
                )
            })
        })
    }

    fn focus_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        Box::pin(async move {
            rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                id: request_id,
                action: rmac_compositor::Action::FocusWindow { window },
            })
            .await
            .result
            .map_err(|error| BackendError::new(action_error_kind(error.kind), error.message))
        })
    }

    fn close_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        Box::pin(async move {
            rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                id: request_id,
                action: rmac_compositor::Action::CloseWindow { window },
            })
            .await
            .result
            .map_err(|error| BackendError::new(action_error_kind(error.kind), error.message))
        })
    }

    fn update_pins(
        &self,
        command: &rmac_dock::PinCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>> {
        let command = command.clone();
        Box::pin(async move {
            blocking::unblock(move || {
                let store = rmac_shell_settings::ShellSettingsStore::from_environment()
                    .map_err(settings_error)?;
                update_pins_in_store(&store, &command)
            })
            .await
        })
    }

    fn reorder_pins(
        &self,
        reorder: &rmac_dock::drag::RevalidatedReorder,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>> {
        let reorder = reorder.clone();
        Box::pin(async move {
            blocking::unblock(move || {
                let store = rmac_shell_settings::ShellSettingsStore::from_environment()
                    .map_err(settings_error)?;
                reorder_pins_in_store(&store, reorder.command(), reorder.expected_order())
            })
            .await
        })
    }

    fn open_directory(&self, path: &Path) -> BackendFuture<'_, Result<(), BackendError>> {
        let path = path.to_path_buf();
        Box::pin(async move {
            rmac_portal::show_item(&path).await.map_err(|_| {
                BackendError::new(
                    FailureKind::Other,
                    "the desktop portal could not open the selected directory",
                )
            })
        })
    }

    fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>> {
        Box::pin(async move {
            rmac_portal::open_trash().await.map_err(|_| {
                BackendError::new(FailureKind::Other, "the desktop could not open Trash")
            })
        })
    }
}

fn update_pins_in_store(
    store: &rmac_shell_settings::ShellSettingsStore,
    command: &rmac_dock::PinCommand,
) -> Result<Vec<rmac_shell_settings::AppId>, BackendError> {
    let mut settings = store.load().map_err(settings_error)?.settings;
    let next = rmac_dock::apply_pin_command(&settings.pinned_apps, command)
        .map_err(|error| BackendError::new(FailureKind::Unsupported, error.to_string()))?;
    if next != settings.pinned_apps {
        settings.pinned_apps = next;
        store.save(&settings).map_err(settings_error)?;
    }
    store
        .load()
        .map(|snapshot| snapshot.settings.pinned_apps)
        .map_err(settings_error)
}

fn reorder_pins_in_store(
    store: &rmac_shell_settings::ShellSettingsStore,
    command: &rmac_dock::PinCommand,
    expected_order: &[rmac_shell_settings::AppId],
) -> Result<Vec<rmac_shell_settings::AppId>, BackendError> {
    let mut settings = store.load().map_err(settings_error)?.settings;
    if settings.pinned_apps != expected_order {
        return Err(BackendError::new(
            FailureKind::Rejected,
            "the pinned application order changed before the drag completed",
        ));
    }
    let next = rmac_dock::apply_pin_command(&settings.pinned_apps, command)
        .map_err(|error| BackendError::new(FailureKind::Unsupported, error.to_string()))?;
    if next != settings.pinned_apps {
        settings.pinned_apps = next;
        store.save(&settings).map_err(settings_error)?;
    }
    store
        .load()
        .map(|snapshot| snapshot.settings.pinned_apps)
        .map_err(settings_error)
}

fn settings_error(error: rmac_shell_settings::Error) -> BackendError {
    BackendError::new(FailureKind::Io(error.error_kind), error.to_string())
}

fn action_error_kind(kind: rmac_compositor::ActionErrorKind) -> FailureKind {
    match kind {
        rmac_compositor::ActionErrorKind::Unavailable => FailureKind::Unavailable,
        rmac_compositor::ActionErrorKind::Transport => FailureKind::Transport,
        rmac_compositor::ActionErrorKind::Protocol => FailureKind::Protocol,
        rmac_compositor::ActionErrorKind::Rejected => FailureKind::Rejected,
        rmac_compositor::ActionErrorKind::Unsupported => FailureKind::Unsupported,
    }
}

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

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
        failure: Mutex<Option<BackendError>>,
    }

    #[derive(Default)]
    struct FakeTrashBackend {
        entries: Mutex<Vec<rmac_places_system::TrashEntryId>>,
        purged: Mutex<Vec<rmac_places_system::TrashEntryId>>,
    }

    impl rmac_places_system::Backend for FakeTrashBackend {
        fn home(&self) -> Option<std::path::PathBuf> {
            Some("/home/alex".into())
        }

        fn config_home(&self) -> Option<std::path::PathBuf> {
            None
        }

        fn read_optional(&self, _: &Path) -> io::Result<Option<String>> {
            Ok(None)
        }

        fn exists(&self, _: &Path) -> io::Result<bool> {
            Ok(true)
        }

        fn trash_count(&self) -> Result<usize, String> {
            Ok(self.entries.lock().expect("entries lock").len())
        }

        fn trash_entries(&self) -> Result<Vec<rmac_places_system::TrashEntryId>, String> {
            Ok(self.entries.lock().expect("entries lock").clone())
        }

        fn purge_trash(&self, reviewed: &[rmac_places_system::TrashEntryId]) -> Result<(), String> {
            let mut entries = self.entries.lock().expect("entries lock");
            if reviewed.iter().any(|reviewed| !entries.contains(reviewed)) {
                return Err("Trash changed after review".into());
            }
            self.purged
                .lock()
                .expect("purged lock")
                .extend_from_slice(reviewed);
            entries.retain(|entry| !reviewed.contains(entry));
            Ok(())
        }
    }

    impl FakeBackend {
        fn result<T>(&self, value: T) -> Result<T, BackendError> {
            match self.failure.lock().expect("failure lock").take() {
                Some(detail) => Err(detail),
                None => Ok(value),
            }
        }
    }

    impl Backend for FakeBackend {
        fn launch<'a>(
            &'a self,
            spec: &'a rmac_apps::LaunchSpec,
        ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("launch {spec:?}"));
                self.result(rmac_app_launch::Outcome {
                    process_id: Some(4242),
                    delivery: rmac_app_launch::Delivery::DirectFallback,
                })
            })
        }

        fn focus_window(
            &self,
            request_id: rmac_compositor::ActivationId,
            window: rmac_compositor::WindowId,
        ) -> BackendFuture<'_, Result<(), BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("focus {} request {}", window.0, request_id.0));
                self.result(())
            })
        }

        fn close_window(
            &self,
            request_id: rmac_compositor::ActivationId,
            window: rmac_compositor::WindowId,
        ) -> BackendFuture<'_, Result<(), BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("close {} request {}", window.0, request_id.0));
                self.result(())
            })
        }

        fn update_pins(
            &self,
            command: &rmac_dock::PinCommand,
        ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>> {
            let command = command.clone();
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("pins {command:?}"));
                self.result(vec![rmac_shell_settings::AppId(
                    command.app_id().to_owned(),
                )])
            })
        }

        fn reorder_pins(
            &self,
            reorder: &rmac_dock::drag::RevalidatedReorder,
        ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, BackendError>> {
            let reorder = reorder.clone();
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("reorder {reorder:?}"));
                self.result(reorder.expected_order().to_vec())
            })
        }

        fn open_directory(&self, _: &Path) -> BackendFuture<'_, Result<(), BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push("open directory <private>".into());
                self.result(())
            })
        }

        fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push("open Trash".into());
                self.result(())
            })
        }
    }

    fn launch_activation() -> rmac_dock::Activation {
        rmac_dock::Activation::Launch {
            app_id: "dev.rmac.Terminal.desktop".into(),
            spec: rmac_apps::LaunchSpec::Command {
                program: "rmac-terminal".into(),
                args: vec!["--new-window".into()],
                working_dir: None,
                terminal: false,
            },
        }
    }

    #[test]
    fn launch_uses_the_exact_catalog_spec_and_returns_only_a_receipt() {
        let backend = FakeBackend::default();
        let outcome = futures_lite::future::block_on(execute(
            &launch_activation(),
            rmac_compositor::ActivationId(9),
            &backend,
        ))
        .expect("launch succeeds");
        assert_eq!(
            outcome,
            Outcome::Launched {
                app_id: "dev.rmac.Terminal.desktop".into(),
                launch: rmac_app_launch::Outcome {
                    process_id: Some(4242),
                    delivery: rmac_app_launch::Delivery::DirectFallback,
                },
            }
        );
        let calls = backend.calls.into_inner().expect("calls");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("rmac-terminal"));
        assert!(calls[0].contains("--new-window"));
    }

    #[test]
    fn focus_uses_the_exact_window_and_activation_identity() {
        let backend = FakeBackend::default();
        let outcome = futures_lite::future::block_on(execute(
            &rmac_dock::Activation::FocusWindow(rmac_compositor::WindowId(77)),
            rmac_compositor::ActivationId(12),
            &backend,
        ))
        .expect("focus succeeds");
        assert_eq!(
            outcome,
            Outcome::FocusRequested {
                window: rmac_compositor::WindowId(77)
            }
        );
        assert_eq!(
            backend.calls.into_inner().expect("calls"),
            ["focus 77 request 12"]
        );
    }

    #[test]
    fn platform_failure_keeps_operation_and_identity_context() {
        let backend = FakeBackend::default();
        *backend.failure.lock().expect("failure lock") = Some(BackendError::new(
            FailureKind::Io(std::io::ErrorKind::PermissionDenied),
            "permission denied",
        ));
        let error = futures_lite::future::block_on(execute(
            &launch_activation(),
            rmac_compositor::ActivationId(1),
            &backend,
        ))
        .expect_err("launch fails");
        assert_eq!(error.operation, Operation::Launch);
        assert_eq!(
            error.kind,
            FailureKind::Io(std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(error.app_id, "dev.rmac.Terminal.desktop");
        assert_eq!(error.detail(), "permission denied");
    }

    #[test]
    fn unavailable_and_noop_outcomes_never_touch_platform_services() {
        let backend = FakeBackend::default();
        assert_eq!(
            futures_lite::future::block_on(execute(
                &rmac_dock::Activation::NoAction,
                rmac_compositor::ActivationId(1),
                &backend,
            )),
            Ok(Outcome::NoAction)
        );
        let error = futures_lite::future::block_on(execute(
            &rmac_dock::Activation::Unavailable {
                app_id: "gone.desktop".into(),
                detail: "not installed".into(),
            },
            rmac_compositor::ActivationId(2),
            &backend,
        ))
        .expect_err("unavailable remains unavailable");
        assert_eq!(error.operation, Operation::Resolve);
        assert!(backend.calls.into_inner().expect("calls").is_empty());
    }

    #[test]
    fn context_close_uses_the_exact_window_request() {
        let backend = FakeBackend::default();
        let outcome = futures_lite::future::block_on(execute_context(
            &rmac_dock::ContextAction::CloseWindow {
                app_id: "terminal.desktop".into(),
                window: rmac_compositor::WindowId(88),
            },
            rmac_compositor::ActivationId(14),
            &backend,
        ))
        .expect("close succeeds");
        assert_eq!(
            outcome,
            Outcome::CloseRequested {
                window: rmac_compositor::WindowId(88)
            }
        );
        assert_eq!(
            backend.calls.into_inner().expect("calls"),
            ["close 88 request 14"]
        );
    }

    #[test]
    fn pin_receipt_does_not_mutate_the_dock_model() {
        let backend = FakeBackend::default();
        let command = rmac_dock::PinCommand::Pin {
            app_id: "music.desktop".into(),
        };
        let outcome = futures_lite::future::block_on(execute_context(
            &rmac_dock::ContextAction::UpdatePins(command.clone()),
            rmac_compositor::ActivationId(1),
            &backend,
        ))
        .expect("pin persists");
        assert_eq!(
            outcome,
            Outcome::PinsUpdated {
                pinned: vec![rmac_shell_settings::AppId("music.desktop".into())]
            }
        );
        assert_eq!(
            backend.calls.into_inner().expect("calls"),
            [format!("pins {command:?}")]
        );
    }

    #[test]
    fn special_places_open_exact_authority_without_disclosing_paths() {
        let backend = FakeBackend::default();
        let outcome = futures_lite::future::block_on(execute_special(
            &rmac_dock::SpecialActivation::OpenDirectory {
                kind: rmac_dock::SpecialItemKind::Downloads,
                path: "/home/alex/Private/Downloads".into(),
            },
            &backend,
        ))
        .expect("Downloads opens");
        assert_eq!(
            outcome,
            Outcome::PlaceOpened {
                kind: rmac_dock::SpecialItemKind::Downloads
            }
        );
        assert_eq!(
            backend.calls.into_inner().expect("calls"),
            ["open directory <private>"]
        );

        let backend = FakeBackend::default();
        assert_eq!(
            futures_lite::future::block_on(execute_special(
                &rmac_dock::SpecialActivation::OpenTrash,
                &backend,
            )),
            Ok(Outcome::PlaceOpened {
                kind: rmac_dock::SpecialItemKind::Trash
            })
        );
        assert_eq!(backend.calls.into_inner().expect("calls"), ["open Trash"]);
    }

    #[test]
    fn unavailable_special_place_never_touches_platform_services() {
        let backend = FakeBackend::default();
        let error = futures_lite::future::block_on(execute_special(
            &rmac_dock::SpecialActivation::Unavailable {
                kind: rmac_dock::SpecialItemKind::Downloads,
                detail: "the Downloads directory is unavailable".into(),
            },
            &backend,
        ))
        .expect_err("unavailable place is rejected");
        assert_eq!(error.operation, Operation::Resolve);
        assert_eq!(error.app_id, "Downloads");
        assert!(backend.calls.into_inner().expect("calls").is_empty());
    }

    #[test]
    fn empty_trash_execution_deletes_only_the_reviewed_authority() {
        let reviewed = [
            rmac_places_system::TrashEntryId::from_authority_bytes(b"first"),
            rmac_places_system::TrashEntryId::from_authority_bytes(b"second"),
        ];
        let backend = FakeTrashBackend {
            entries: Mutex::new(reviewed.to_vec()),
            ..Default::default()
        };
        let review = prepare_special_context(
            &rmac_dock::SpecialContextAction::EmptyTrash {
                expected_item_count: 2,
            },
            &backend,
        )
        .expect("exact review prepares");
        assert_eq!(review.item_count(), 2);

        let added = rmac_places_system::TrashEntryId::from_authority_bytes(b"added later");
        backend.entries.lock().expect("entries lock").push(added);
        let outcome = execute_empty_trash(
            rmac_places_system::confirm_empty_trash(review, true).expect("confirmed"),
            &backend,
        )
        .expect("reviewed entries purge");
        assert_eq!(outcome, Outcome::TrashEmptied { remaining_items: 1 });
        assert_eq!(*backend.entries.lock().expect("entries lock"), [added]);
        assert_eq!(backend.purged.lock().expect("purged lock").len(), 2);
    }

    #[test]
    fn changed_trash_count_refuses_review_without_private_diagnostics() {
        let backend = FakeTrashBackend {
            entries: Mutex::new(vec![
                rmac_places_system::TrashEntryId::from_authority_bytes(b"only item"),
            ]),
            ..Default::default()
        };
        let error = prepare_special_context(
            &rmac_dock::SpecialContextAction::EmptyTrash {
                expected_item_count: 2,
            },
            &backend,
        )
        .expect_err("stale menu count is rejected");
        assert_eq!(error.operation, Operation::ReviewTrash);
        assert_eq!(error.app_id, "Trash");
        assert_eq!(error.detail(), "Trash changed before the deletion review");
    }

    #[test]
    fn pin_store_transaction_preserves_unrelated_shell_settings() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("rmac-dock-system-{}-{unique}", std::process::id()));
        let store = rmac_shell_settings::ShellSettingsStore::new(directory.join("shell.json"));
        let settings = rmac_shell_settings::ShellSettings {
            dock: rmac_shell_settings::DockSettings {
                autohide: true,
                ..Default::default()
            },
            providers: [(
                rmac_shell_settings::ProviderId("files".into()),
                rmac_shell_settings::ProviderPolicy::default(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        store.save(&settings).expect("seed settings");

        let pinned = update_pins_in_store(
            &store,
            &rmac_dock::PinCommand::Pin {
                app_id: "terminal.desktop".into(),
            },
        )
        .expect("pin persists");
        assert_eq!(
            pinned,
            [rmac_shell_settings::AppId("terminal.desktop".into())]
        );
        let loaded = store.load().expect("reload settings").settings;
        assert!(loaded.dock.autohide);
        assert!(loaded
            .providers
            .contains_key(&rmac_shell_settings::ProviderId("files".into())));

        std::fs::remove_dir_all(&directory).unwrap_or_else(|error| {
            panic!("remove Dock settings test directory {directory:?}: {error}")
        });
    }

    #[test]
    fn drag_store_transaction_rechecks_the_exact_accepted_order() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("rmac-dock-reorder-{}-{unique}", std::process::id()));
        let store = rmac_shell_settings::ShellSettingsStore::new(directory.join("shell.json"));
        let original = [
            rmac_shell_settings::AppId("finder.desktop".into()),
            rmac_shell_settings::AppId("terminal.desktop".into()),
            rmac_shell_settings::AppId("notes.desktop".into()),
        ];
        store
            .save(&rmac_shell_settings::ShellSettings {
                pinned_apps: original.to_vec(),
                ..Default::default()
            })
            .expect("seed pin order");

        let moved = reorder_pins_in_store(
            &store,
            &rmac_dock::PinCommand::MoveTo {
                app_id: "finder.desktop".into(),
                index: 2,
            },
            &original,
        )
        .expect("matching authority reorders");
        assert_eq!(
            moved,
            [
                rmac_shell_settings::AppId("terminal.desktop".into()),
                rmac_shell_settings::AppId("notes.desktop".into()),
                rmac_shell_settings::AppId("finder.desktop".into()),
            ]
        );

        let error = reorder_pins_in_store(
            &store,
            &rmac_dock::PinCommand::MoveTo {
                app_id: "terminal.desktop".into(),
                index: 2,
            },
            &original,
        )
        .expect_err("stale accepted order is rejected");
        assert_eq!(error.kind, FailureKind::Rejected);
        assert_eq!(
            store.load().expect("reload pins").settings.pinned_apps,
            moved
        );

        std::fs::remove_dir_all(&directory).unwrap_or_else(|error| {
            panic!("remove Dock reorder test directory {directory:?}: {error}")
        });
    }
}
