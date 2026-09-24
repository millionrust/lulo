//! `rmac-ui` — the shared design system for the rmac desktop suite.
//!
//! Every rmac app depends on this crate so they share one look: macOS-style
//! window chrome, a common live theme, fonts, and application boot helpers.

mod assets;
mod chrome;
mod components;
mod controls;
mod feedback;
pub mod gallery;
pub mod mac;
mod menu_target;
mod runtime;
pub mod scroll;
pub mod shortcuts;
mod text_keys;
pub mod theme;
mod window;

pub use assets::{layered_assets, LayeredAssets};
pub use chrome::{
    body_bg, minimize_focused_window, page, title_bar, title_bar_content, toolbar, toolbar_group,
    toolbar_title, traffic_lights, traffic_lights_active, traffic_lights_fixed_size,
    traffic_lights_origin, TrafficLights,
};
pub use components::{
    alert, alert_with_icon, dialog, dialog_button, type_select_match, ContextMenu,
    ContextMenuState, DialogButtonKind, DismissMenu, MenuCheck, RequestClose,
};
pub use controls::{
    Button, ButtonRole, Checkbox, CollectionState, InputEvent, InputState, List, ListRow,
    PopUpButton, Position, Radio, RadioGroup, RopeExt, SearchField, SegmentedControl, SelectAll,
    Slider, SliderAxis, SliderEvent, SliderState, SwitchSize, Table, Tabs, TextField, Toggle,
    ToggleState, Tree, TreeRow,
};
pub use controls::{Column, ColumnSort, TableDelegate, TableEvent, TableState};
pub use feedback::{
    user_error_message, EmptyState, ErrorSurface, Progress, ProgressStatus, Spinner, Toast,
    ToastKind, Tooltip,
};
pub use gpui_component::{ActiveTheme, StyledExt};
pub use menu_target::register_menu_target;
pub use runtime::{
    defer_content_ready, init_application, install_app_menu, mark_content_ready,
    prepare_surface_window, shell_surface_root, text_px,
};
pub use window::*;

/// Stable Linux desktop identities matching desktop files and Wayland app IDs.
pub mod app_id {
    pub use rmac_apps::identity::{
        APP_DRAWER, CALCULATOR, CLOCK, FILES, NOTES, PLAYER, PREVIEW, SYSTEM_MONITOR,
        SYSTEM_SETTINGS, TERMINAL, TEXT_EDITOR, WEATHER,
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
