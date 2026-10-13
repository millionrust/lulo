//! Lulo's desktop: the wallpaper and, on it, the Desktop folder's icons,
//! Stacks, the desktop menu and widgets. One view for every platform (ADR
//! 0023, "Phase 3 revised: shared shell views"): Lulo OS runs it as the
//! `wallpaper` background layer surface, Windows inside `lulo-shell`, each
//! through `rmac-shell-layer`'s surfaces.

#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
mod surface;

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub use surface::run;
#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
pub use surface::{asset, asset_names, start};
