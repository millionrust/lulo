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
