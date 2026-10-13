//! Windows' own icons as picture files, for the shared views (ADR 0023,
//! "Phase 3 revised: shared shell views"): a desktop shortcut's target
//! (`rmac_shell_layer::system::file_icons`) and the Dock's running Windows
//! apps ([`watch_running`]).
//!
//! Each icon is read by a one-shot `lulo-shell --save-icon` process, as the
//! icon helper reads them (WIN-OS-53): the shell libraries and icon
//! handlers that reading an icon loads never come into lulo-shell, and a
//! crashing handler takes only that process down. The PNGs are cached under
//! Lulo's cache folder by source and modification time.

use std::hash::{Hash as _, Hasher as _};
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};

/// `lulo-shell`'s switch that writes one icon: `--save-icon <pixels>
/// <source> <png>`.
pub const SAVE_SWITCH: &str = "--save-icon";

/// The icon edge: the desktop's largest icon and the Dock's tile at up to
/// 150 % scale.
const PIXELS: u32 = 128;

/// `lulo-shell --save-icon`: write the icon and exit.
pub fn run_save() -> i32 {
    let mut arguments = std::env::args_os().skip(2);
    let (Some(pixels), Some(source), Some(out)) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        return 2;
    };
    let pixels = pixels.to_string_lossy().parse::<u32>().unwrap_or(PIXELS);
    super::catalog::init_com();
    let written = super::icons::save_png(&source.to_string_lossy(), pixels, Path::new(&out));
    i32::from(!written)
}

/// Lulo's cache folder for these icons.
fn cache() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(|local| PathBuf::from(local).join("Lulo").join("Cache"))
        })
        // `-2`: icons fitted into the icon grid (WIN-OS-71); the earlier
        // folder is removed by `watch_running`.
        .map(|cache| cache.join("lulo-icons-2"))
}

/// The cached icon of `source` (a file or a program), written by a helper
/// process when missing. Blocking.
pub fn icon(source: &Path) -> Option<PathBuf> {
    let folder = cache()?;
    let modified = std::fs::metadata(source)
        .and_then(|metadata| metadata.modified())
        .ok();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    modified.hash(&mut hasher);
    let out = folder.join(format!("{:016x}.png", hasher.finish()));
    if out.is_file() {
        return Some(out);
    }
    std::fs::create_dir_all(&folder).ok()?;
    let exe = std::env::current_exe().ok()?;
    let status = std::process::Command::new(exe)
        .arg(SAVE_SWITCH)
        .arg(PIXELS.to_string())
        .arg(source)
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
        .status()
        .ok()?;
    (status.success() && out.is_file()).then_some(out)
}

/// The icon of a running Windows app `app_id` (an executable's file name)
/// without a desktop entry, read from its program and registered for the
/// Dock (`rmac_apps::windows_apps::register_icon`). Blocking.
pub fn ensure_running_icon(app_id: &str, exe_path: &str) {
    if !app_id.ends_with(".exe")
        || exe_path.is_empty()
        || rmac_apps::windows_apps::app_for_exe(app_id).is_some()
        || rmac_apps::windows_apps::icon_for(app_id).is_some()
    {
        return;
    }
    let has_entry = rmac_apps::windows_apps::data_home().is_some_and(|home| {
        home.join("applications")
            .join(format!("{app_id}.desktop"))
            .is_file()
    });
    if has_entry {
        return;
    }
    if let Some(icon) = icon(Path::new(exe_path)) {
        rmac_apps::windows_apps::register_icon(app_id, icon);
        rmac_compositor_system::refresh();
        super::trace(|| format!("windows apps: running {app_id} shows its icon"));
    }
}

/// Give every running Windows app without a desktop entry its icon, as
/// its windows appear (the window list's own events; nothing polls).
pub fn watch_running(cx: &mut gpui::App) {
    // Icons cached before they were fitted into the grid.
    if let Some(old) = cache().and_then(|cache| cache.parent().map(|parent| parent.join("lulo-icons"))) {
        cx.background_executor()
            .spawn(async move {
                let _ = blocking::unblock(move || std::fs::remove_dir_all(old)).await;
            })
            .detach();
    }
    let (sender, receiver) = async_channel::bounded::<rmac_compositor::Event>(16);
    cx.background_executor()
        .spawn(async move {
            let _ = rmac_compositor_system::watch(sender).await;
        })
        .detach();
    cx.background_executor()
        .spawn(async move {
            let mut seen = std::collections::HashSet::<String>::new();
            while let Ok(event) = receiver.recv().await {
                // The watch sends a snapshot only when the windows changed.
                let rmac_compositor::Event::Snapshot { snapshot } = event else {
                    continue;
                };
                let fresh = snapshot
                    .windows
                    .iter()
                    .filter_map(|window| {
                        let app_id = window.app_id.clone()?;
                        let pid = u32::try_from(window.pid?).ok()?;
                        seen.insert(app_id.clone()).then_some((app_id, pid))
                    })
                    .collect::<Vec<_>>();
                if fresh.is_empty() {
                    continue;
                }
                blocking::unblock(move || {
                    for (app_id, pid) in fresh {
                        ensure_running_icon(&app_id, &super::process_path(pid));
                    }
                })
                .await;
            }
        })
        .detach();
}
