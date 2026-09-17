use std::path::Path;

use crate::{Backend, BackendError, BackendFuture, FailureKind};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>> {
        let spec = spec.clone();
        Box::pin(async move { rmac_app_launch::launch(spec).await.map_err(launch_error) })
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

    fn restore_window(
        &self,
        request_id: rmac_compositor::ActivationId,
        window: rmac_compositor::WindowId,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        Box::pin(async move {
            let mut store = rmac_compositor::ParkingStore::load_default();
            let Some(workspace) = store.forget(window) else {
                return Err(BackendError::new(
                    FailureKind::Unsupported,
                    "the shell did not park this window",
                ));
            };
            if let Err(error) = store.save_default() {
                eprintln!("could not save the parking set: {error}");
            }
            rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                id: request_id,
                action: rmac_compositor::Action::RestoreWindow { window, workspace },
            })
            .await
            .result
            .map_err(|error| BackendError::new(action_error_kind(error.kind), error.message))
        })
    }

    fn reveal_application(&self, source: &Path) -> BackendFuture<'_, Result<(), BackendError>> {
        let source = source.to_path_buf();
        Box::pin(async move {
            rmac_app_launch::reveal_item(source).await.map_err(|_| {
                BackendError::new(
                    FailureKind::Other,
                    "the desktop portal could not reveal the application",
                )
            })
        })
    }

    fn terminate_application(
        &self,
        pids: &[u32],
        kind: rmac_dock::TerminationKind,
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let pids = pids.to_vec();
        let kind = match kind {
            rmac_dock::TerminationKind::Quit => rmac_app_launch::TerminationKind::Quit,
            rmac_dock::TerminationKind::ForceQuit => rmac_app_launch::TerminationKind::ForceQuit,
        };
        Box::pin(async move {
            rmac_app_launch::terminate_application(pids, kind)
                .await
                .map_err(|error| BackendError::new(FailureKind::Rejected, error.to_string()))
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
            let path = path.into_os_string().into_string().map_err(|_| {
                BackendError::new(
                    FailureKind::Unsupported,
                    "Finder cannot open a directory whose path is not valid UTF-8",
                )
            })?;
            launch_finder(vec!["--path".into(), path]).await
        })
    }

    fn open_trash(&self) -> BackendFuture<'_, Result<(), BackendError>> {
        Box::pin(async move { launch_finder(vec!["--trash".into()]).await })
    }
}

async fn launch_finder(args: Vec<String>) -> Result<(), BackendError> {
    rmac_app_launch::launch(rmac_apps::LaunchSpec::Command {
        program: "/usr/bin/rmac-files".into(),
        args,
        working_dir: None,
        terminal: false,
    })
    .await
    .map(|_| ())
    .map_err(launch_error)
}

fn launch_error(error: rmac_app_launch::Error) -> BackendError {
    BackendError::new(
        match error.kind {
            rmac_app_launch::ErrorKind::Io(kind) => FailureKind::Io(kind),
            rmac_app_launch::ErrorKind::Rejected => FailureKind::Rejected,
            rmac_app_launch::ErrorKind::Protocol => FailureKind::Protocol,
        },
        error.to_string(),
    )
}

pub(crate) fn update_pins_in_store(
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

pub(crate) fn reorder_pins_in_store(
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
