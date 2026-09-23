//! Launcher validated Settings and clipboard system-surface adapter.

use super::*;

#[derive(Clone)]
pub(super) struct SurfaceBridge {
    pub(super) clipboard: async_channel::Sender<String>,
}

impl Surface for SurfaceBridge {
    fn open_setting(&self, pane_id: &str) -> Result<(), BackendError> {
        if !rmac_launcher_providers::system_settings_entries()
            .iter()
            .any(|entry| entry.pane_id == pane_id)
        {
            return Err(BackendError::new(
                FailureKind::InvalidAction,
                "unknown Settings destination",
            ));
        }
        let executable = std::env::current_exe()
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))?
            .with_file_name("rmac-system-settings");
        Command::new(executable)
            .arg("--pane")
            .arg(pane_id)
            .spawn()
            .map(|_| ())
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))
    }

    /// "Search in Files": Files opens on the home folder searching for the
    /// query.
    fn search_files(&self, query: &str) -> Result<(), BackendError> {
        let executable = std::env::current_exe()
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))?
            .with_file_name("rmac-files");
        Command::new(executable)
            .arg("--search")
            .arg(query)
            .spawn()
            .map(|_| ())
            .map_err(|error| BackendError::new(FailureKind::Io(error.kind()), error.to_string()))
    }

    fn copy_text(&self, text: &str) -> Result<(), BackendError> {
        self.clipboard.try_send(text.to_owned()).map_err(|_| {
            BackendError::new(FailureKind::Unavailable, "clipboard surface is unavailable")
        })
    }
}
