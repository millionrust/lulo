#![cfg(target_os = "windows")]

mod clipboard;
mod destination_list;
mod direct_manipulation;
mod direct_write;
mod directx_atlas;
mod directx_devices;
mod directx_renderer;
mod dispatcher;
mod display;
mod events;
mod keyboard;
mod platform;
mod rmac_frame_loop;
mod rmac_input_region;
mod rmac_trace;
mod system_settings;
mod util;
mod vsync;
mod window;
mod wrapper;

pub(crate) use clipboard::*;
pub(crate) use destination_list::*;
pub(crate) use direct_write::*;
pub(crate) use directx_atlas::*;
pub(crate) use directx_devices::*;
pub(crate) use directx_renderer::*;
pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use events::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use system_settings::*;
pub(crate) use util::*;
pub(crate) use vsync::*;
pub(crate) use window::*;
pub(crate) use wrapper::*;

pub use platform::WindowsPlatform;

/// rmac: the family `.SystemUIFont` means, instead of Windows' message
/// font (Segoe UI), for an app that ships its own UI font: Lulo uses Inter
/// on every platform (ADR 0023, "Phase 3 revised: shared shell views").
/// Call before the platform is created; the app then adds the font's files
/// with `TextSystem::add_fonts`.
pub fn set_system_ui_font_family(family: &str) {
    let _ = direct_write::SYSTEM_UI_FONT_OVERRIDE.set(family.to_owned());
}

pub(crate) use windows::Win32::Foundation::HWND;
