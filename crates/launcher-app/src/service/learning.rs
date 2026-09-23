//! The local store of learned Spotlight choices.
//!
//! One small text file in the user's state directory
//! (`$XDG_STATE_HOME/rmac/spotlight-choices`, else
//! `~/.local/state/rmac/spotlight-choices`), readable only by its owner.
//! Nothing leaves this computer.

use super::*;

const FILE_NAME: &str = "spotlight-choices";

fn store_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local").join("state"))
        })?;
    Some(base.join("rmac").join(FILE_NAME))
}

pub(super) fn load() -> rmac_launcher::Learning {
    store_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| rmac_launcher::Learning::from_text(&text))
        .unwrap_or_default()
}

fn save(learning: &rmac_launcher::Learning) -> std::io::Result<()> {
    let Some(path) = store_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("part");
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(learning.to_text().as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, &path)
}

/// Remember that `id` was chosen for `query`, for this and later overlays.
pub(crate) fn learn(query: String, id: rmac_launcher::ResultId, cx: &mut App) {
    if !cx.has_global::<LauncherService>() {
        return;
    }
    let now = rmac_launcher_providers::locale::unix_now().max(0) as u64;
    let learning = cx.update_global::<LauncherService, _>(|service, _| {
        let mut learning = (*service.learning).clone();
        learning.record(&query, &id, now);
        service.learning = Arc::new(learning);
        service.learning.clone()
    });
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = save(&learning) {
                eprintln!("Spotlight could not save what it learned: {error}");
            }
        })
        .detach();
}
