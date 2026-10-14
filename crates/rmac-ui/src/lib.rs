//! `rmac-ui` — the shared design system for the rmac desktop suite.
//!
//! Every rmac app depends on this crate so they share one look: macOS-style
//! window chrome, a common live theme, fonts, and application boot helpers.

mod about;
pub mod accessibility;
mod app_menu;
mod assets;
mod chrome;
mod components;
mod context_menu;
mod controls;
mod feedback;
#[cfg(windows)]
mod file_watch_windows;
pub mod gallery;
#[cfg(windows)]
mod instance_windows;
pub mod mac;
mod menu_strip;
mod menu_target;
#[cfg(windows)]
mod menubar_link;
mod platform;
mod runtime;
pub mod scroll;
pub mod session;
pub mod shortcuts;
mod speech;
mod svg_icon;
pub mod text_assist;
pub mod text_complete;
mod text_keys;
mod text_transform;
pub mod theme;
mod window;
#[cfg(windows)]
mod window_frame_windows;

pub use accessibility::AccessibleTextInput;
pub use app_menu::{
    set_menu_checked, set_menu_children, set_menu_enabled, set_menu_hidden, set_menu_label,
    set_menu_mixed, ShowAboutPanel,
};
pub use assets::{layered_assets, LayeredAssets};
pub use chrome::{
    body_bg, cycle_through_windows, double_click_title_bar_action, fill_focused_window,
    minimize_focused_window, page, quit_application, title_bar, title_bar_content,
    title_bar_drag_region, toggle_full_screen_focused_window, toolbar, toolbar_group,
    toolbar_title, traffic_lights, traffic_lights_active, traffic_lights_fixed_size,
    traffic_lights_origin, windowed_process_ids, TrafficLights,
};
pub use components::{
    alert, alert_cancel_default, alert_with_icon, dialog, dialog_button, Dialog, DialogButtonKind,
    DismissMenu, PasteAndMatchStyle, RequestClose,
};
pub use context_menu::{type_select_match, ContextMenu, ContextMenuState, MenuCheck, MenuTag};
pub use controls::{
    overlay_scrollbar, slider_bulge_lerp, uniform_list_scrollbar, Button, ButtonRole, Checkbox,
    CollectionState, DocumentTitleMenu, InputEvent, InputState, KeyboardAction, List, ListRow,
    NativeMenu, PopUpButton, PopupMenuItem, Position, Radio, RadioGroup, Rope, RopeExt,
    ScrollPosition, SearchField, SegmentedControl, SelectAll, Slider, SliderAxis, SliderBulge,
    SliderEvent, SliderState, SwitchSize, Table, Tabs, TextField, TextMenuBuilder, Toggle,
    ToggleState, Tree, TreeRow, SLIDER_BULGE_MS, SLIDER_PRESS_MS,
};
pub use controls::{tooltip_view, Column, ColumnSort, TableDelegate, TableEvent, TableState};
pub use feedback::{
    user_error_message, EmptyState, ErrorSurface, Progress, ProgressStatus, Spinner, Toast,
    ToastKind, Tooltip,
};
pub use gpui_component::{ActiveTheme, StyledExt};
pub use menu_strip::{height as menu_strip_height, MENU_STRIP_HEIGHT};
pub use menu_target::{register_menu_target, track_key_window};
pub use rmac_app_menu::Item as MenuItem;
/// Bigger hit targets while the PC is used by touch (ADR 0028).
pub use rmac_shell_ui::touch;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher_around;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher_around_with_escape;
#[cfg(target_os = "linux")]
pub use runtime::set_outside_click_catcher_hole;
pub use runtime::{
    defer_content_ready, init_application, install_app_instance, install_app_menu,
    install_surface_idle_exit, mark_content_ready, prepare_surface_window, shell_surface_root,
    text_px,
};
pub use shortcuts::bind_keys;
pub use speech::{speak, start_speaking, stop_speaking};
pub use svg_icon::{svg_icon, IconSource};
pub use text_assist::EditableText;

/// The text-field actions every rmac text field answers (the "Input" key
/// context's bindings and the Edit menu's `input::` rows), for an editor
/// that is not an [`InputState`] to answer the same keys and menu rows.
pub mod input_actions {
    pub use crate::controls::{
        InputBackspace as Backspace, InputCopy as Copy, InputCut as Cut, InputDelete as Delete,
        InputDeleteToBeginningOfLine as DeleteToBeginningOfLine,
        InputDeleteToEndOfLine as DeleteToEndOfLine,
        InputDeleteToNextWordEnd as DeleteToNextWordEnd,
        InputDeleteToPreviousWordStart as DeleteToPreviousWordStart, InputEnter as Enter,
        InputIndentInline as IndentInline, InputMoveDown as MoveDown, InputMoveEnd as MoveEnd,
        InputMoveHome as MoveHome, InputMoveLeft as MoveLeft, InputMovePageDown as MovePageDown,
        InputMovePageUp as MovePageUp, InputMoveRight as MoveRight, InputMoveToEnd as MoveToEnd,
        InputMoveToEndOfLine as MoveToEndOfLine, InputMoveToNextWord as MoveToNextWord,
        InputMoveToPreviousWord as MoveToPreviousWord, InputMoveToStart as MoveToStart,
        InputMoveToStartOfLine as MoveToStartOfLine, InputMoveUp as MoveUp, InputRedo as Redo,
        InputSelectToEnd as SelectToEnd, InputSelectToEndOfLine as SelectToEndOfLine,
        InputSelectToNextWordEnd as SelectToNextWordEnd,
        InputSelectToPreviousWordStart as SelectToPreviousWordStart,
        InputSelectToStart as SelectToStart, InputSelectToStartOfLine as SelectToStartOfLine,
        InputShowCharacterPalette as ShowCharacterPalette, InputUndo as Undo, Paste, SelectAll,
    };

    /// The key context those bindings live in.
    pub const KEY_CONTEXT: &str = "Input";
}
pub use text_transform::{transform_selection, TextTransformation};
pub use window::*;

/// Stable Linux desktop identities matching desktop files and Wayland app IDs.
pub mod app_id {
    pub use rmac_apps::identity::{
        APP_DRAWER, ARCHIVE_UTILITY, CALCULATOR, CALENDAR, CLOCK, FILES, MAIL, NOTES, PLAYER,
        PREVIEW, SYSTEM_MONITOR, SYSTEM_SETTINGS, TERMINAL, TEXT_EDITOR, WEATHER,
    };
}

/// The regular files among the paths dropped on a window, in drop order and
/// without repeats: what a document app opens or attaches. Folders and paths
/// that no longer exist are left out.
pub fn dropped_files(dropped: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for path in dropped {
        if path.is_file() && !files.contains(path) {
            files.push(path.clone());
        }
    }
    files
}

#[cfg(test)]
mod dropped_files_tests {
    use super::dropped_files;

    #[test]
    fn only_existing_regular_files_are_taken_once_in_drop_order() {
        let root = std::env::temp_dir().join(format!("rmac-dropped-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        let first = root.join("b.txt");
        let second = root.join("a.txt");
        std::fs::write(&first, "b").unwrap();
        std::fs::write(&second, "a").unwrap();
        let dropped = vec![
            first.clone(),
            root.join("folder"),
            root.join("missing.txt"),
            second.clone(),
            first.clone(),
        ];
        assert_eq!(dropped_files(&dropped), vec![first, second]);
        assert!(dropped_files(&[]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}

pub const UI_FONT: &str = "Inter";
pub const MONO_FONT: &str = "JetBrains Mono";

/// Construct GPUI with the platform backend that owns native display and
/// Wayland layer-shell integration (or, on Windows, Win32 and DirectX).
pub fn application() -> gpui::Application {
    platform::prepare_environment();
    #[cfg(windows)]
    fonts_windows::prepare();
    gpui_platform::application()
}

#[cfg(windows)]
mod fonts_windows;

#[cfg(test)]
mod tests;

/// Add an app-defined row to this process's `RMAC_FRAME_TRACE` (a no-op
/// without it, and on hosts other than Linux), so the speed sweep can time
/// app steps on the same clock as the frames.
pub fn trace_mark(event: &str) {
    #[cfg(target_os = "linux")]
    gpui_linux::trace_mark(event);
    #[cfg(not(target_os = "linux"))]
    let _ = event;
}
