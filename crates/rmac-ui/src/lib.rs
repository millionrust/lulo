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
mod controls;
mod feedback;
pub mod gallery;
pub mod mac;
mod menu_target;
mod runtime;
pub mod scroll;
pub mod session;
pub mod shortcuts;
mod speech;
pub mod text_assist;
mod text_keys;
mod text_transform;
pub mod theme;
mod window;

pub use accessibility::AccessibleTextInput;
pub use app_menu::{
    set_menu_checked, set_menu_children, set_menu_enabled, set_menu_label, set_menu_mixed,
    ShowAboutPanel,
};
pub use assets::{layered_assets, LayeredAssets};
pub use chrome::{
    body_bg, double_click_title_bar_action, minimize_focused_window, page, quit_application,
    title_bar, title_bar_content, title_bar_drag_region, toolbar, toolbar_group, toolbar_title,
    traffic_lights, traffic_lights_active, traffic_lights_fixed_size, traffic_lights_origin,
    TrafficLights,
};
pub use components::{
    alert, alert_cancel_default, alert_with_icon, dialog, dialog_button, type_select_match,
    ContextMenu, ContextMenuState, Dialog, DialogButtonKind, DismissMenu, MenuCheck,
    PasteAndMatchStyle, RequestClose,
};
pub use controls::{
    slider_bulge_lerp, uniform_list_scrollbar, Button, ButtonRole, Checkbox, CollectionState,
    DocumentTitleMenu, InputEvent, InputState, KeyboardAction, List, ListRow, PopUpButton,
    Position, Radio, RadioGroup, Rope, RopeExt, SearchField, SegmentedControl, SelectAll, Slider,
    SliderAxis, SliderBulge, SliderEvent, SliderState, SwitchSize, Table, Tabs, TextField, Toggle,
    ToggleState, Tree, TreeRow, SLIDER_BULGE_MS,
};
pub use controls::{tooltip_view, Column, ColumnSort, TableDelegate, TableEvent, TableState};
pub use feedback::{
    user_error_message, EmptyState, ErrorSurface, Progress, ProgressStatus, Spinner, Toast,
    ToastKind, Tooltip,
};
pub use gpui_component::{ActiveTheme, StyledExt};
pub use menu_target::{register_menu_target, track_key_window};
pub use rmac_app_menu::Item as MenuItem;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher_around;
#[cfg(target_os = "linux")]
pub use runtime::open_outside_click_catcher_around_with_escape;
pub use runtime::{
    defer_content_ready, init_application, install_app_instance, install_app_menu,
    install_surface_idle_exit, mark_content_ready, prepare_surface_window, shell_surface_root,
    text_px,
};
pub use speech::{speak, start_speaking, stop_speaking};
pub use text_transform::{transform_selection, TextTransformation};
pub use window::*;

/// Stable Linux desktop identities matching desktop files and Wayland app IDs.
pub mod app_id {
    pub use rmac_apps::identity::{
        APP_DRAWER, CALCULATOR, CALENDAR, CLOCK, FILES, MAIL, NOTES, PLAYER, PREVIEW,
        SYSTEM_MONITOR, SYSTEM_SETTINGS, TERMINAL, TEXT_EDITOR, WEATHER,
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
