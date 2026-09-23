//! Width-driven System Monitor toolbar geometry.

const COMPACT_TOOLBAR_MAX_WIDTH: f32 = 900.0;
/// The column chooser drops from the ⋯ capsule, whose bottom edge is at
/// 43.5 in the 52 pt toolbar, at every width.
const REGULAR_COLUMNS_MENU_TOP: f32 = 48.0;
const COMPACT_COLUMNS_MENU_TOP: f32 = 48.0;
const REGULAR_SEARCH_WIDTH: f32 = 220.0;
const COMPACT_SEARCH_WIDTH: f32 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ToolbarLayout {
    pub(super) compact: bool,
    pub(super) columns_menu_top: f32,
    pub(super) search_width: f32,
}

pub(super) fn toolbar_layout(window_width: f32) -> ToolbarLayout {
    let compact = !window_width.is_finite()
        || window_width <= 0.0
        || window_width < COMPACT_TOOLBAR_MAX_WIDTH;
    if compact {
        ToolbarLayout {
            compact: true,
            columns_menu_top: COMPACT_COLUMNS_MENU_TOP,
            search_width: COMPACT_SEARCH_WIDTH,
        }
    } else {
        ToolbarLayout {
            compact: false,
            columns_menu_top: REGULAR_COLUMNS_MENU_TOP,
            search_width: REGULAR_SEARCH_WIDTH,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_toolbar_reflows_controls_and_moves_its_menu_anchor() {
        let compact = toolbar_layout(640.0);
        assert!(compact.compact);
        assert_eq!(compact.search_width, COMPACT_SEARCH_WIDTH);
        assert_eq!(compact.columns_menu_top, COMPACT_COLUMNS_MENU_TOP);

        let regular = toolbar_layout(1_040.0);
        assert!(!regular.compact);
        assert_eq!(regular.search_width, REGULAR_SEARCH_WIDTH);
        assert_eq!(regular.columns_menu_top, REGULAR_COLUMNS_MENU_TOP);

        assert_eq!(toolbar_layout(f32::NAN), compact);
    }
}
