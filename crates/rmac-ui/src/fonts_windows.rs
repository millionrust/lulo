//! Lulo's UI fonts on Windows (ADR 0023, "Phase 3 revised: shared shell
//! views"). Lulo OS draws everything in Inter, and Terminal and code in
//! JetBrains Mono (the `fonts-inter` and `fonts-jetbrains-mono` packages,
//! made the system faces by packaging/fontconfig/60-rmac.conf). Windows has
//! neither: GPUI fell back to Segoe UI, whose wider, differently spaced
//! letters made every Lulo surface look like Windows, and Terminal's grid
//! fell back to a proportional face. The faces Lulo uses, the same files
//! Lulo OS installs, ship inside each Windows build, are added to the app's
//! text system before its first window, and Inter stands in for the system
//! UI font, so text lays out exactly as it does on Lulo OS. `gpui_windows`
//! lends DirectWrite these bytes rather than copying them, and rasterizes
//! their glyphs as Lulo OS does (ADR 0025, "Text").
//!
//! Inter's Thin and Light are here too: Calculator's display, Clock's dials
//! and Weather's temperatures ask for them, and without them DirectWrite
//! drew those numbers in Regular, much heavier than on Lulo OS.

use std::borrow::Cow;

use gpui::App;

const FACES: [&[u8]; 10] = [
    include_bytes!("../../../assets/fonts/inter/Inter-Thin.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Light.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Regular.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Medium.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.otf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Bold.otf"),
    include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMono-BoldItalic.ttf"),
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
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    if let Err(error) = cx.text_system().add_fonts(faces) {
        eprintln!("rmac: Lulo's fonts could not be added ({error}); text falls back to Segoe UI");
    }
}
