//! Typed launch and niri-focus execution for Dock activation outcomes.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Launch,
    Focus,
    Close,
    UpdatePins,
    Resolve,
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
        process_id: u32,
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
    NoAction,
}

pub trait Backend: Send + Sync + 'static {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<u32, BackendError>>;

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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<u32, BackendError>> {
        let spec = spec.clone();
        Box::pin(async move {
            blocking::unblock(move || {
                rmac_apps::launch(&spec)
                    .map(|child| child.id())
                    .map_err(|error| {
                        BackendError::new(FailureKind::Io(error.kind()), error.to_string())
                    })
            })
            .await
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
            .map(|process_id| Outcome::Launched {
                app_id: app_id.clone(),
                process_id,
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
            .map(|process_id| Outcome::Launched {
                app_id: app_id.clone(),
                process_id,
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
        failure: Mutex<Option<BackendError>>,
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
        ) -> BackendFuture<'a, Result<u32, BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("calls lock")
                    .push(format!("launch {spec:?}"));
                self.result(4242)
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
                process_id: 4242,
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
}
