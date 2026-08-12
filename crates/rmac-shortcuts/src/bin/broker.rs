use std::path::PathBuf;
use std::process::ExitCode;

use rmac_shortcuts::{BackendStatus, Event};
use rmac_storage::atomic_write;

fn main() -> ExitCode {
    match async_io::block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("XDG_RUNTIME_DIR is not set to an absolute path")?;
    let status_path = runtime.join("rmac/shortcuts-status.json");
    let niri_config = std::env::var_os("NIRI_CONFIG")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("NIRI_CONFIG is not set to an absolute path")?;
    let niri_bindings_path = niri_config
        .parent()
        .ok_or("NIRI_CONFIG has no parent")?
        .join("shortcuts-generated.kdl");
    let executable = std::env::current_exe()?;
    let dispatcher = executable
        .parent()
        .ok_or("shortcut broker executable has no parent")?
        .join("rmac-shortcut-dispatch");
    let (sender, receiver) = async_channel::bounded(32);
    let watcher = rmac_shortcuts::watch(sender);
    let consumer = async {
        while let Ok(event) = receiver.recv().await {
            match &event {
                Event::Backend { status } => {
                    rmac_shortcuts::write_niri_binding_mode(
                        &niri_bindings_path,
                        &dispatcher,
                        status,
                    )?;
                    write_status(&status_path, status)?;
                }
                Event::Activated { id, .. } => {
                    if let Err(error) = rmac_shortcuts::dispatch(id) {
                        eprintln!("{error}");
                    }
                }
                Event::Bound { .. } | Event::Deactivated { .. } | Event::BindingsChanged { .. } => {
                }
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    futures_util::try_join!(async { watcher.await.map_err(Into::into) }, consumer)?;
    Ok(())
}

fn write_status(
    path: &std::path::Path,
    status: &BackendStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("shortcut status path has no parent")?;
    std::fs::create_dir_all(parent)?;
    atomic_write(path, &serde_json::to_vec_pretty(status)?)?;
    Ok(())
}
