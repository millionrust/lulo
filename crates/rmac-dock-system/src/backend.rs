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
            let workspace = match store.forget(window) {
                Some(workspace) => {
                    if let Err(error) = store.save_default() {
                        eprintln!("could not save the parking set: {error}");
                    }
                    workspace
                }
                // Parked without a record (its minimize lost a race with
                // another writer of the set): bring it to the Space in view,
                // which is where a Mac restores a window whose Space is gone.
                None => {
                    let snapshot = blocking::unblock(|| {
                        futures_lite::future::block_on(rmac_compositor_niri::snapshot())
                    })
                    .await
                    .map_err(|error| {
                        BackendError::new(FailureKind::Unavailable, format!("{error:?}"))
                    })?;
                    restore_target(&snapshot).ok_or_else(|| {
                        BackendError::new(FailureKind::Unsupported, "no Space to restore to")
                    })?
                }
            };
            rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                id: request_id,
                action: rmac_compositor::Action::RestoreWindow { window, workspace },
            })
            .await
            .result
            .map_err(|error| BackendError::new(action_error_kind(error.kind), error.message))
        })
    }

    fn hide_windows(
        &self,
        request_id: rmac_compositor::ActivationId,
        windows: &[rmac_compositor::WindowId],
    ) -> BackendFuture<'_, Result<(), BackendError>> {
        let windows = windows.to_vec();
        Box::pin(async move {
            // The niri snapshot reader is not Send; read it on its own thread.
            let snapshot = blocking::unblock(|| {
                futures_lite::future::block_on(rmac_compositor_niri::snapshot())
            })
            .await
            .map_err(|error| BackendError::new(FailureKind::Unavailable, format!("{error:?}")))?;
            let mut store = rmac_compositor::ParkingStore::load_default();
            store.prune(&snapshot);
            store.record_from(&snapshot, &windows);
            // Record first: a window parked without its origin could not be
            // brought back to the right Space.
            store.save_default().map_err(|error| {
                BackendError::new(
                    FailureKind::Other,
                    format!("could not save where the hidden windows belong: {error}"),
                )
            })?;
            let mut failure = None;
            for window in windows {
                let result = rmac_compositor_niri::execute(rmac_compositor::ActionRequest {
                    id: request_id,
                    action: rmac_compositor::Action::MinimizeWindow { window },
                })
                .await
                .result;
                if let Err(error) = result {
                    failure = Some(BackendError::new(
                        action_error_kind(error.kind),
                        error.message,
                    ));
                }
            }
            failure.map_or(Ok(()), Err)
        })
    }

    fn show_all_windows(
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
            .map_err(|error| BackendError::new(action_error_kind(error.kind), error.message))?;
            // App Exposé shows the focused application's windows.
            blocking::unblock(|| {
                std::process::Command::new("/usr/libexec/rmac/rmac-mission-control")
                    .arg("app-windows")
                    .status()
            })
            .await
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))
            .and_then(|status| {
                status.success().then_some(()).ok_or_else(|| {
                    BackendError::new(FailureKind::Unavailable, "Mission Control is not running")
                })
            })
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

    fn update_stacks(
        &self,
        command: &rmac_dock::StackCommand,
    ) -> BackendFuture<'_, Result<Vec<rmac_shell_settings::DockStackEntry>, BackendError>> {
        let command = command.clone();
        Box::pin(async move {
            blocking::unblock(move || {
                let store = rmac_shell_settings::ShellSettingsStore::from_environment()
                    .map_err(settings_error)?;
                update_stacks_in_store(&store, &command)
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
                    "Files cannot open a directory whose path is not valid UTF-8",
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

pub(crate) fn update_stacks_in_store(
    store: &rmac_shell_settings::ShellSettingsStore,
    command: &rmac_dock::StackCommand,
) -> Result<Vec<rmac_shell_settings::DockStackEntry>, BackendError> {
    let mut settings = store.load().map_err(settings_error)?.settings;
    let next = rmac_dock::apply_stack_command(&settings.dock_stacks, command)
        .map_err(|error| BackendError::new(FailureKind::Unsupported, error.to_string()))?;
    if next != settings.dock_stacks {
        settings.dock_stacks = next;
        store.save(&settings).map_err(settings_error)?;
    }
    store
        .load()
        .map(|snapshot| snapshot.settings.dock_stacks)
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

/// Where a parked window with no recorded origin goes back to: the focused
/// Space, or failing that any Space in view that is not the parking one.
pub(crate) fn restore_target(
    snapshot: &rmac_compositor::Snapshot,
) -> Option<rmac_compositor::WorkspaceId> {
    let usable = |workspace: &&rmac_compositor::Workspace| {
        workspace.name.as_deref() != Some(rmac_compositor::PARKING_WORKSPACE)
    };
    snapshot
        .workspaces
        .iter()
        .filter(usable)
        .find(|workspace| workspace.focused)
        .or_else(|| {
            snapshot
                .workspaces
                .iter()
                .filter(usable)
                .find(|workspace| workspace.active)
        })
        .map(|workspace| workspace.id)
}
