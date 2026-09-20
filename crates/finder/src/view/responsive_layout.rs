//! Width-driven Files chrome policy with no persisted side effects.

const MIN_BROWSER_CONTENT_WIDTH: f32 = 400.0;
const TITLE_MIN_WIDTH: f32 = 760.0;
const VIEW_CONTROL_MIN_WIDTH: f32 = 840.0;
const WIDE_SEARCH_MIN_WIDTH: f32 = 960.0;
const COMPACT_SEARCH_WIDTH: f32 = 140.0;
// design-lab/windows.html §1: the Finder toolbar search capsule is 180 px.
const WIDE_SEARCH_WIDTH: f32 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ResponsiveLayout {
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_available: bool,
    pub(super) title_visible: bool,
    pub(super) view_control_visible: bool,
    pub(super) search_width: f32,
}

pub(super) fn responsive_layout(
    window_width: f32,
    sidebar_requested: bool,
    sidebar_width: f32,
) -> ResponsiveLayout {
    let valid_window = window_width.is_finite() && window_width > 0.0;
    let sidebar_available = valid_window
        && sidebar_width.is_finite()
        && sidebar_width > 0.0
        && window_width - sidebar_width >= MIN_BROWSER_CONTENT_WIDTH;
    ResponsiveLayout {
        sidebar_visible: sidebar_requested && sidebar_available,
        sidebar_available,
        title_visible: valid_window && window_width >= TITLE_MIN_WIDTH,
        view_control_visible: valid_window && window_width >= VIEW_CONTROL_MIN_WIDTH,
        search_width: if valid_window && window_width >= WIDE_SEARCH_MIN_WIDTH {
            WIDE_SEARCH_WIDTH
        } else {
            COMPACT_SEARCH_WIDTH
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_preserve_core_content_and_never_change_the_requested_preference() {
        let wide = responsive_layout(1_200.0, true, 360.0);
        assert!(wide.sidebar_visible);
        assert!(wide.title_visible);
        assert!(wide.view_control_visible);
        assert_eq!(wide.search_width, WIDE_SEARCH_WIDTH);

        let narrow_default = responsive_layout(720.0, true, 190.0);
        assert!(narrow_default.sidebar_visible);
        assert!(!narrow_default.title_visible);
        assert!(!narrow_default.view_control_visible);
        assert_eq!(narrow_default.search_width, COMPACT_SEARCH_WIDTH);

        let narrow_wide_sidebar = responsive_layout(720.0, true, 360.0);
        assert!(!narrow_wide_sidebar.sidebar_visible);
        assert!(!narrow_wide_sidebar.sidebar_available);

        let user_hidden = responsive_layout(1_200.0, false, 190.0);
        assert!(!user_hidden.sidebar_visible);
        assert!(user_hidden.sidebar_available);

        let invalid = responsive_layout(f32::NAN, true, 190.0);
        assert!(!invalid.sidebar_visible);
        assert!(!invalid.sidebar_available);
    }
}
