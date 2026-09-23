//! `rmac-ui` — the shared design system for the rmac desktop suite.
//!
//! Every rmac app depends on this crate so they share one look: macOS-style
//! window chrome, a common live theme, fonts, and application boot helpers.

mod chrome;
mod components;
mod controls;
mod feedback;
pub mod gallery;
pub mod mac;
mod runtime;
pub mod scroll;
pub mod shortcuts;
pub mod theme;
mod window;

pub use chrome::{
    body_bg, page, title_bar, toolbar, toolbar_group, traffic_lights, traffic_lights_active,
};
pub use components::{
    alert, dialog, dialog_button, type_select_match, ContextMenu, ContextMenuState,
    DialogButtonKind, DismissMenu, MenuCheck, RequestClose,
};
pub use controls::{
    Button, ButtonRole, Checkbox, CollectionState, InputEvent, InputState, List, ListRow,
    PopUpButton, Position, Radio, RopeExt, SearchField, SegmentedControl, SelectAll, Slider,
    SliderAxis, SliderEvent, SliderState, SwitchSize, Table, Tabs, TextField, Toggle, ToggleState,
    Tree, TreeRow,
};
pub use controls::{Column, ColumnSort, TableDelegate, TableEvent, TableState};
pub use feedback::{
    user_error_message, EmptyState, ErrorSurface, Progress, ProgressStatus, Spinner, Toast,
    ToastKind, Tooltip,
};
pub use gpui_component::{ActiveTheme, StyledExt};
pub use runtime::{
    init_application, install_app_menu, prepare_surface_window, shell_surface_root, text_px,
};
pub use window::*;

/// Stable Linux desktop identities matching desktop files and Wayland app IDs.
pub mod app_id {
    pub use rmac_apps::identity::{
        APP_DRAWER, FILES, NOTES, SYSTEM_MONITOR, SYSTEM_SETTINGS, TERMINAL, TEXT_EDITOR,
    };
}

pub const UI_FONT: &str = "Inter";
pub const MONO_FONT: &str = "JetBrains Mono";

/// Construct GPUI with the platform backend that owns native display and
/// Wayland layer-shell integration.
pub fn application() -> gpui::Application {
    gpui_platform::application()
}

#[cfg(test)]
mod tests;
