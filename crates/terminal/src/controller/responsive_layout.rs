//! Width-driven Terminal chrome geometry.

const COMPACT_MAX_WIDTH: f32 = 720.0;
const REGULAR_TITLE_MAX_WIDTH: f32 = 360.0;
const COMPACT_TITLE_MAX_WIDTH: f32 = 220.0;
const REGULAR_TAB_TITLE_MAX_WIDTH: f32 = 180.0;
const COMPACT_TAB_TITLE_MAX_WIDTH: f32 = 120.0;
const REGULAR_FIND_WIDTH: f32 = 240.0;
const COMPACT_FIND_WIDTH: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TerminalLayout {
    pub(super) title_max_width: f32,
    pub(super) tab_title_max_width: f32,
    pub(super) find_width: f32,
}

pub(super) fn terminal_layout(window_width: f32) -> TerminalLayout {
    let compact =
        !window_width.is_finite() || window_width <= 0.0 || window_width < COMPACT_MAX_WIDTH;
    if compact {
        TerminalLayout {
            title_max_width: COMPACT_TITLE_MAX_WIDTH,
            tab_title_max_width: COMPACT_TAB_TITLE_MAX_WIDTH,
            find_width: COMPACT_FIND_WIDTH,
        }
    } else {
        TerminalLayout {
            title_max_width: REGULAR_TITLE_MAX_WIDTH,
            tab_title_max_width: REGULAR_TAB_TITLE_MAX_WIDTH,
            find_width: REGULAR_FIND_WIDTH,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_terminal_returns_space_to_grid_and_find_content() {
        let compact = terminal_layout(640.0);
        assert_eq!(compact.title_max_width, COMPACT_TITLE_MAX_WIDTH);
        assert_eq!(compact.tab_title_max_width, COMPACT_TAB_TITLE_MAX_WIDTH);
        assert_eq!(compact.find_width, COMPACT_FIND_WIDTH);

        let regular = terminal_layout(820.0);
        assert_eq!(regular.title_max_width, REGULAR_TITLE_MAX_WIDTH);
        assert_eq!(regular.tab_title_max_width, REGULAR_TAB_TITLE_MAX_WIDTH);
        assert_eq!(regular.find_width, REGULAR_FIND_WIDTH);

        assert_eq!(terminal_layout(f32::NAN), compact);
    }
}
