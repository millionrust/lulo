//! Lulo's UI fonts on Windows (ADR 0023, "Phase 3 revised: shared shell
//! views"). Lulo OS draws everything in Inter, and Terminal and code in
//! JetBrains Mono (the `fonts-inter` and `fonts-jetbrains-mono` packages,
//! made the system faces by packaging/fontconfig/60-rmac.conf). Windows has
//! neither: GPUI fell back to Segoe UI, whose wider, differently spaced
//! letters made every Lulo surface look like Windows, and Terminal's grid
//! fell back to a proportional face. The faces Lulo uses, the same files
//! Lulo OS installs, ship with each Windows build, are added to the app's
//! text system before its first window, and Inter stands in for the system
//! UI font, so text lays out exactly as it does on Lulo OS. `gpui_windows`
//! lends DirectWrite these bytes rather than copying them, and rasterizes
//! their glyphs as Lulo OS does (ADR 0025, "Text").
//!
//! Inter's Thin and Light are here too: Calculator's display, Clock's dials
//! and Weather's temperatures ask for them, and without them DirectWrite
//! drew those numbers in Regular, much heavier than on Lulo OS.
//!
//! The faces are files in a `fonts` folder beside the exes, mapped into
//! memory, not compiled into every exe (WIN-OS-99). Compiled in, they were
//! 4.8 MB of each app's 20 MB, and Microsoft Defender reads the whole exe
//! before a process may start whenever its scan cache has let it go: about
//! 22 ms a megabyte on the reference laptop. A mapped file costs nothing
//! until DirectWrite reads a table, its pages are shared by every Lulo
//! process, and the scan of a font file is a one-off. A build run from the
//! repository (`cargo run`, CI) finds them in `assets/fonts` instead.

use std::{borrow::Cow, path::PathBuf};

use gpui::App;
use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, OPEN_EXISTING,
        },
        System::Memory::{CreateFileMappingW, MapViewOfFile, FILE_MAP_READ, PAGE_READONLY},
    },
};

/// Each face as (its folder under `assets/fonts`, its file name). Installed
/// builds keep them all in one `fonts` folder beside the exes
/// (packaging/windows/generate_apps_wxs.py).
const FACES: [(&str, &str); 10] = [
    ("inter", "Inter-Thin.otf"),
    ("inter", "Inter-Light.otf"),
    ("inter", "Inter-Regular.otf"),
    ("inter", "Inter-Medium.otf"),
    ("inter", "Inter-SemiBold.otf"),
    ("inter", "Inter-Bold.otf"),
    ("jetbrains-mono", "JetBrainsMono-Regular.ttf"),
    ("jetbrains-mono", "JetBrainsMono-Bold.ttf"),
    ("jetbrains-mono", "JetBrainsMono-Italic.ttf"),
    ("jetbrains-mono", "JetBrainsMono-BoldItalic.ttf"),
];

/// Before the platform starts: `.SystemUIFont` is Inter (the vendored
/// `gpui_windows` reads `RMAC_SYSTEM_UI_FONT`).
pub(crate) fn prepare() {
    if std::env::var_os("RMAC_SYSTEM_UI_FONT").is_none() {
        std::env::set_var("RMAC_SYSTEM_UI_FONT", crate::UI_FONT);
    }
}

/// Add Lulo's faces to the app's text system.
pub(crate) fn install(cx: &mut App) {
    let faces: Vec<Cow<'static, [u8]>> = FACES
        .iter()
        .filter_map(|&(folder, file)| face(folder, file))
        .map(Cow::Borrowed)
        .collect();
    if faces.len() < FACES.len() {
        eprintln!(
            "rmac: {} of Lulo's {} font files were not found beside the app or in the repository",
            FACES.len() - faces.len(),
            FACES.len()
        );
    }
    if faces.is_empty() {
        eprintln!(
            "rmac: Lulo's fonts could not be added (no font files); text falls back to Segoe UI"
        );
        return;
    }
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("rmac: Lulo's fonts could not be added ({error}); text falls back to Segoe UI");
    }
}

/// The places a face may be, installed layout first.
fn candidates(folder: &str, file: &str) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(2);
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("fonts")))
    {
        paths.push(dir.join(file));
    }
    paths.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fonts")
            .join(folder)
            .join(file),
    );
    paths
}

fn face(folder: &str, file: &str) -> Option<&'static [u8]> {
    candidates(folder, file)
        .iter()
        .find_map(|path| map_read_only(path))
}

/// The whole file, mapped read-only for the life of the process (the text
/// system keeps the faces until it exits). Other processes may still read,
/// rename or replace it (an installer upgrade); the mapping keeps this
/// process's view.
fn map_read_only(path: &std::path::Path) -> Option<&'static [u8]> {
    let length = std::fs::metadata(path).ok()?.len();
    if length == 0 || length > u64::from(u32::MAX) {
        return None;
    }
    let name = HSTRING::from(path.as_os_str());
    // SAFETY: plain Win32 calls on a path we own; every handle is closed
    // below, and the view outlives them by design (it is never unmapped, so
    // the slice is valid for 'static). The file is opened with share-read
    // only for writers' sake: nothing may write to it while mapped.
    unsafe {
        let file = CreateFileW(
            &name,
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?;
        let mapping = CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, PCWSTR::null());
        let _ = CloseHandle(file);
        let mapping = mapping.ok()?;
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        let _ = CloseHandle(mapping);
        if view.Value.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(
            view.Value as *const u8,
            length as usize,
        ))
    }
}
