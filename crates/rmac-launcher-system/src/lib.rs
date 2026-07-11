//! Private-safe execution boundary for launcher actions.

use std::fmt;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    LaunchApplication,
    OpenSetting,
    OpenFile,
    RevealFile,
    CopyText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureKind {
    InvalidAction,
    Io(std::io::ErrorKind),
    Unavailable,
    Rejected,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    pub kind: FailureKind,
    detail: String,
}

impl BackendError {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub activation: ActivationId,
    pub operation: Operation,
    pub kind: FailureKind,
    detail: String,
}

impl Error {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.operation {
            Operation::LaunchApplication => "Could not launch the application",
            Operation::OpenSetting => "Could not open Settings",
            Operation::OpenFile => "Could not open the file",
            Operation::RevealFile => "Could not reveal the file",
            Operation::CopyText => "Could not copy the result",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    ApplicationLaunched { process_id: u32 },
    SettingOpened,
    FileOpened,
    FileRevealed,
    TextCopied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub activation: ActivationId,
    pub outcome: Outcome,
}

pub trait Backend: Send + Sync + 'static {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<u32, BackendError>>;

    fn open_setting<'a>(&'a self, pane_id: &'a str) -> BackendFuture<'a, Result<(), BackendError>>;

    fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>>;

    fn reveal_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>>;

    fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>>;
}

/// UI-owned operations that require the live launcher application context.
pub trait Surface: Send + Sync + 'static {
    fn open_setting(&self, pane_id: &str) -> Result<(), BackendError>;
    fn copy_text(&self, text: &str) -> Result<(), BackendError>;
}

#[derive(Clone, Debug)]
pub struct SystemBackend<S> {
    surface: S,
}

impl<S> SystemBackend<S> {
    pub fn new(surface: S) -> Self {
        Self { surface }
    }
}

impl<S: Surface> Backend for SystemBackend<S> {
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

    fn open_setting<'a>(&'a self, pane_id: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.surface.open_setting(pane_id) })
    }

    fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { rmac_portal::open_item(path).await.map_err(portal_error) })
    }

    fn reveal_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { rmac_portal::show_item(path).await.map_err(portal_error) })
    }

    fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.surface.copy_text(text) })
    }
}

fn portal_error(error: rmac_portal::Error) -> BackendError {
    BackendError::new(FailureKind::Other, error.detail)
}

pub async fn execute(
    activation: ActivationId,
    action: &rmac_launcher::Action,
    backend: &impl Backend,
) -> Result<Receipt, Error> {
    let operation = operation(action);
    let outcome = match action {
        rmac_launcher::Action::LaunchApplication { app_id, spec } => {
            require_nonempty(app_id, "application ID")
                .map_err(|error| failure(activation, operation, error))?;
            backend
                .launch(spec)
                .await
                .map(|process_id| Outcome::ApplicationLaunched { process_id })
        }
        rmac_launcher::Action::OpenSetting { pane_id } => {
            require_nonempty(pane_id, "Settings pane ID")
                .map_err(|error| failure(activation, operation, error))?;
            backend
                .open_setting(pane_id)
                .await
                .map(|()| Outcome::SettingOpened)
        }
        rmac_launcher::Action::OpenFile { path } => {
            require_absolute(path).map_err(|error| failure(activation, operation, error))?;
            backend.open_file(path).await.map(|()| Outcome::FileOpened)
        }
        rmac_launcher::Action::RevealFile { path } => {
            require_absolute(path).map_err(|error| failure(activation, operation, error))?;
            backend
                .reveal_file(path)
                .await
                .map(|()| Outcome::FileRevealed)
        }
        rmac_launcher::Action::CopyText { text } => {
            require_nonempty(text, "copy text")
                .map_err(|error| failure(activation, operation, error))?;
            backend.copy_text(text).await.map(|()| Outcome::TextCopied)
        }
    }
    .map_err(|error| failure(activation, operation, error))?;

    Ok(Receipt {
        activation,
        outcome,
    })
}

fn operation(action: &rmac_launcher::Action) -> Operation {
    match action {
        rmac_launcher::Action::LaunchApplication { .. } => Operation::LaunchApplication,
        rmac_launcher::Action::OpenSetting { .. } => Operation::OpenSetting,
        rmac_launcher::Action::OpenFile { .. } => Operation::OpenFile,
        rmac_launcher::Action::RevealFile { .. } => Operation::RevealFile,
        rmac_launcher::Action::CopyText { .. } => Operation::CopyText,
    }
}

fn require_nonempty(value: &str, field: &str) -> Result<(), BackendError> {
    (!value.trim().is_empty())
        .then_some(())
        .ok_or_else(|| BackendError::new(FailureKind::InvalidAction, format!("{field} is empty")))
}

fn require_absolute(path: &Path) -> Result<(), BackendError> {
    path.is_absolute()
        .then_some(())
        .ok_or_else(|| BackendError::new(FailureKind::InvalidAction, "file path is not absolute"))
}

fn failure(activation: ActivationId, operation: Operation, error: BackendError) -> Error {
    Error {
        activation,
        operation,
        kind: error.kind,
        detail: error.detail,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
        failure: Mutex<Option<BackendError>>,
    }

    impl FakeBackend {
        fn complete(&self, call: String) -> Result<(), BackendError> {
            self.calls.lock().expect("calls lock").push(call);
            self.failure
                .lock()
                .expect("failure lock")
                .take()
                .map_or(Ok(()), Err)
        }
    }

    impl Backend for FakeBackend {
        fn launch<'a>(
            &'a self,
            spec: &'a rmac_apps::LaunchSpec,
        ) -> BackendFuture<'a, Result<u32, BackendError>> {
            Box::pin(async move {
                self.complete(format!("launch {spec:?}"))?;
                Ok(42)
            })
        }

        fn open_setting<'a>(
            &'a self,
            pane_id: &'a str,
        ) -> BackendFuture<'a, Result<(), BackendError>> {
            Box::pin(async move { self.complete(format!("setting {pane_id}")) })
        }

        fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
            Box::pin(async move { self.complete(format!("open {}", path.display())) })
        }

        fn reveal_file<'a>(
            &'a self,
            path: &'a Path,
        ) -> BackendFuture<'a, Result<(), BackendError>> {
            Box::pin(async move { self.complete(format!("reveal {}", path.display())) })
        }

        fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
            Box::pin(async move { self.complete(format!("copy {text}")) })
        }
    }

    fn run(action: rmac_launcher::Action, backend: &FakeBackend) -> Result<Receipt, Error> {
        futures_lite::future::block_on(execute(ActivationId(7), &action, backend))
    }

    #[test]
    fn every_typed_action_dispatches_once_and_returns_a_payload_free_receipt() {
        let backend = FakeBackend::default();
        let actions = [
            rmac_launcher::Action::LaunchApplication {
                app_id: "notes.desktop".into(),
                spec: rmac_apps::LaunchSpec::Command {
                    program: "notes".into(),
                    args: vec!["--new".into()],
                    working_dir: None,
                    terminal: false,
                },
            },
            rmac_launcher::Action::OpenSetting {
                pane_id: "sound".into(),
            },
            rmac_launcher::Action::OpenFile {
                path: "/home/alex/Report.txt".into(),
            },
            rmac_launcher::Action::RevealFile {
                path: "/home/alex/Report.txt".into(),
            },
            rmac_launcher::Action::CopyText { text: "42".into() },
        ];
        let expected = [
            Outcome::ApplicationLaunched { process_id: 42 },
            Outcome::SettingOpened,
            Outcome::FileOpened,
            Outcome::FileRevealed,
            Outcome::TextCopied,
        ];
        for (action, expected) in actions.into_iter().zip(expected) {
            let receipt = run(action, &backend).expect("action succeeds");
            assert_eq!(receipt.activation, ActivationId(7));
            assert_eq!(receipt.outcome, expected);
        }
        assert_eq!(backend.calls.lock().expect("calls lock").len(), 5);
    }

    #[test]
    fn invalid_actions_never_reach_the_backend() {
        let backend = FakeBackend::default();
        for action in [
            rmac_launcher::Action::OpenSetting {
                pane_id: " ".into(),
            },
            rmac_launcher::Action::OpenFile {
                path: "relative.txt".into(),
            },
            rmac_launcher::Action::CopyText {
                text: String::new(),
            },
        ] {
            let error = run(action, &backend).expect_err("action is rejected");
            assert_eq!(error.kind, FailureKind::InvalidAction);
        }
        assert!(backend.calls.lock().expect("calls lock").is_empty());
    }

    #[test]
    fn default_error_text_redacts_file_paths_copy_text_and_backend_details() {
        let backend = FakeBackend::default();
        *backend.failure.lock().expect("failure lock") = Some(BackendError::new(
            FailureKind::Rejected,
            "secret /home/alex/Report.txt",
        ));
        let error = run(
            rmac_launcher::Action::OpenFile {
                path: "/home/alex/Report.txt".into(),
            },
            &backend,
        )
        .expect_err("backend rejects action");
        assert_eq!(error.to_string(), "Could not open the file");
        assert!(!error.to_string().contains("alex"));
        assert!(error.detail().contains("alex"));
    }
}
