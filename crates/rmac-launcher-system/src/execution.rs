use std::path::Path;

use crate::{ActivationId, Backend, BackendError, Error, FailureKind, Operation, Outcome, Receipt};

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
                .map(|launch| Outcome::ApplicationLaunched { launch })
        }
        rmac_launcher::Action::RevealApplication { source } => {
            require_absolute(source).map_err(|error| failure(activation, operation, error))?;
            backend
                .reveal_file(source)
                .await
                .map(|()| Outcome::ApplicationRevealed)
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
        rmac_launcher::Action::RevealApplication { .. } => Operation::RevealApplication,
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
