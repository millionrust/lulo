//! Lulo's UI font on Windows (ADR 0023, "Phase 3 revised: shared shell
//! views"). Lulo OS draws everything in Inter (the `fonts-inter` package,
//! made the system face by packaging/fontconfig/60-rmac.conf). Windows has
//! no Inter, and GPUI fell back to Segoe UI, whose wider, differently
//! spaced letters made every Lulo surface look like Windows. The same four
//! Inter faces Lulo OS installs ship inside each Windows build, are added
//! to the app's text system before its first window, and stand in for the
//! system UI font, so text lays out exactly as it does on Lulo OS.

use std::borrow::Cow;

use gpui::App;

const FACES: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/inter/Inter-Regular.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Medium.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Bold.otf"),
];

/// Before the platform starts: `.SystemUIFont` is Inter.
pub(crate) fn prepare() {
    gpui_windows::set_system_ui_font_family(crate::UI_FONT);
}

/// Add Inter's faces to the app's text system.
pub(crate) fn install(cx: &mut App) {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("rmac: Inter could not be added ({error}); text falls back to Segoe UI");
    }
}
