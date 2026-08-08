//! Width-driven System Settings master/detail policy.

const SPLIT_VIEW_MIN_WIDTH: f32 = 760.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SettingsLayout {
    pub(crate) compact: bool,
    pub(crate) sidebar_visible: bool,
    pub(crate) detail_visible: bool,
}

pub(crate) fn responsive_layout(window_width: f32, compact_sidebar_open: bool) -> SettingsLayout {
    let compact =
        !window_width.is_finite() || window_width <= 0.0 || window_width < SPLIT_VIEW_MIN_WIDTH;
    if compact {
        SettingsLayout {
            compact: true,
            sidebar_visible: compact_sidebar_open,
            detail_visible: !compact_sidebar_open,
        }
    } else {
        SettingsLayout {
            compact: false,
            sidebar_visible: true,
            detail_visible: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_mode_exposes_exactly_one_surface_and_split_mode_exposes_both() {
        let detail = responsive_layout(640.0, false);
        assert!(detail.compact);
        assert!(!detail.sidebar_visible);
        assert!(detail.detail_visible);

        let sidebar = responsive_layout(640.0, true);
        assert!(sidebar.compact);
        assert!(sidebar.sidebar_visible);
        assert!(!sidebar.detail_visible);

        let split = responsive_layout(1_000.0, true);
        assert!(!split.compact);
        assert!(split.sidebar_visible);
        assert!(split.detail_visible);

        let invalid = responsive_layout(f32::NAN, false);
        assert_eq!(invalid, detail);
    }
}
