use gpui::{FontWeight, Hsla};

// Surfaces
/// Window / editor content background.
pub fn window() -> Hsla {
    crate::theme::current().colors.window.hsla()
}
/// Raised card, popover, and compact overlay background.
pub fn raised() -> Hsla {
    crate::theme::current().colors.raised.hsla()
}
/// Regular Liquid Glass-style material for navigation and transient panels.
/// The compositor supplies the blurred pixels beneath it; rmac supplies the
/// adaptive tint and foreground contrast.
pub fn material() -> Hsla {
    crate::theme::current().materials.regular.hsla()
}
/// Clear material for compact floating controls over rich backgrounds.
pub fn material_clear() -> Hsla {
    crate::theme::current().materials.clear.hsla()
}
/// Regular material tuned for navigation sidebars.
pub fn material_sidebar() -> Hsla {
    crate::theme::current().materials.sidebar.hsla()
}
/// Absolute white, for switch thumbs and slider knobs.
pub fn white() -> Hsla {
    crate::theme::current().colors.white.hsla()
}

/// Absolute black.
pub fn black() -> Hsla {
    crate::theme::current().colors.black.hsla()
}

/// Opaque standard material for the content layer.
pub fn material_content() -> Hsla {
    crate::theme::current().materials.content.hsla()
}
/// Strong legibility material for heads-up displays and tooltips.
pub fn material_hud() -> Hsla {
    crate::theme::current().materials.hud.hsla()
}
/// Unified toolbar / window chrome.
pub fn chrome() -> Hsla {
    crate::theme::current().colors.chrome.hsla()
}
/// Source list (sidebar) background.
pub fn sidebar() -> Hsla {
    crate::theme::current().colors.sidebar.hsla()
}
/// Middle list column background.
pub fn list() -> Hsla {
    crate::theme::current().colors.list.hsla()
}

// Corner-radius hierarchy shared by every rmac surface. Keeping these here
// prevents each app from drifting into a different visual language.
pub fn radius_control() -> f32 {
    crate::theme::current().radii.control
}

pub fn radius_card() -> f32 {
    crate::theme::current().radii.card
}

pub fn radius_popover() -> f32 {
    crate::theme::current().radii.popover
}

pub fn radius_large_surface() -> f32 {
    crate::theme::current().radii.large_surface
}

pub fn radius_pill() -> f32 {
    crate::theme::current().radii.pill
}

// Shared desktop component metrics.
pub fn compact_control_height() -> f32 {
    crate::theme::current().metrics.compact_control_height
}
pub fn regular_control_height() -> f32 {
    crate::theme::current().metrics.regular_control_height
}
pub fn toolbar_height() -> f32 {
    crate::theme::current().metrics.toolbar_height
}
pub fn sidebar_row_height() -> f32 {
    crate::theme::current().metrics.sidebar_row_height
}
pub fn list_row_height() -> f32 {
    crate::theme::current().metrics.list_row_height
}
pub fn toggle_width() -> f32 {
    crate::theme::current().metrics.toggle_width
}
pub fn toggle_height() -> f32 {
    crate::theme::current().metrics.toggle_height
}
pub fn toggle_thumb() -> f32 {
    crate::theme::current().metrics.toggle_thumb
}

/// `(width, height, thumb)` for each switch size.
pub fn switch_regular() -> (f32, f32, f32) {
    let metrics = crate::theme::current().metrics;
    (
        metrics.switch_regular_width,
        metrics.switch_regular_height,
        metrics.switch_regular_thumb,
    )
}

pub fn switch_small() -> (f32, f32, f32) {
    let metrics = crate::theme::current().metrics;
    (
        metrics.switch_small_width,
        metrics.switch_small_height,
        metrics.switch_small_thumb,
    )
}

pub fn switch_mini() -> (f32, f32, f32) {
    let metrics = crate::theme::current().metrics;
    (
        metrics.switch_mini_width,
        metrics.switch_mini_height,
        metrics.switch_mini_thumb,
    )
}
pub fn traffic_light_hit_width() -> f32 {
    crate::theme::current().metrics.traffic_light_hit_width
}
pub fn traffic_light_hit_height() -> f32 {
    crate::theme::current().metrics.traffic_light_hit_height
}
pub fn traffic_light_diameter() -> f32 {
    crate::theme::current().metrics.traffic_light_diameter
}

// Text
/// Primary label color (near-black).
pub fn text() -> Hsla {
    crate::theme::current().colors.text.hsla()
}
/// Secondary label (systemGray).
pub fn text_secondary() -> Hsla {
    crate::theme::current().colors.text_secondary.hsla()
}
/// Tertiary label (section headers, counts).
pub fn text_tertiary() -> Hsla {
    crate::theme::current().colors.text_tertiary.hsla()
}

// Lines & fills
/// Hairline separator (~8% black).
pub fn separator() -> Hsla {
    crate::theme::current().colors.separator.hsla()
}
/// Hover fill on rows/controls.
pub fn hover() -> Hsla {
    crate::theme::current().colors.hover.hsla()
}
pub fn row_alternate() -> Hsla {
    crate::theme::current().colors.row_alternate.hsla()
}
pub fn control_fill() -> Hsla {
    crate::theme::current().colors.control_fill.hsla()
}
pub fn control_fill_hover() -> Hsla {
    crate::theme::current().colors.control_fill_hover.hsla()
}
/// Neutral (unfocused) selection fill in source lists.
pub fn sidebar_selection() -> Hsla {
    crate::theme::current().colors.selection_unfocused.hsla()
}

// Accents (shared by buttons, menus, selections)
/// System blue — primary actions, selection, focus.
pub fn accent() -> Hsla {
    crate::theme::current().colors.accent.hsla()
}
pub fn accent_subtle() -> Hsla {
    crate::theme::current().colors.accent_subtle.hsla()
}
pub fn accent_border() -> Hsla {
    crate::theme::current().colors.accent_border.hsla()
}
/// System red — destructive actions.
pub fn danger() -> Hsla {
    crate::theme::current().colors.danger.hsla()
}
/// Legible text over the destructive fill.
pub fn on_danger() -> Hsla {
    crate::theme::current().colors.on_danger.hsla()
}
pub fn error_background() -> Hsla {
    crate::theme::current().colors.error_background.hsla()
}
pub fn error_border() -> Hsla {
    crate::theme::current().colors.error_border.hsla()
}
pub fn warning_background() -> Hsla {
    crate::theme::current().colors.warning_background.hsla()
}
pub fn warning_border() -> Hsla {
    crate::theme::current().colors.warning_border.hsla()
}
pub fn warning_text() -> Hsla {
    crate::theme::current().colors.warning_text.hsla()
}
/// On-accent text (white).
pub fn on_accent() -> Hsla {
    crate::theme::current().colors.on_accent.hsla()
}
/// Scrim behind a modal dialog (~22% black).
pub fn scrim() -> Hsla {
    crate::theme::current().colors.scrim.hsla()
}

// Notes accent family (yellow)
pub fn notes_accent() -> Hsla {
    crate::theme::current().colors.notes_accent.hsla()
}
/// Soft yellow row highlight for the selected note (focused).
pub fn notes_selection() -> Hsla {
    crate::theme::current().colors.notes_selection.hsla()
}

// Type weights (SF on macOS via the system font)
pub const REGULAR: FontWeight = FontWeight::NORMAL;
pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
pub const SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
pub const BOLD: FontWeight = FontWeight::BOLD;
