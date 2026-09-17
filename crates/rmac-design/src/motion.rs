//! Motion tokens. Every animation is paired with a Reduce Motion alternative.

use rmac_appearance::MotionPreference;

/// Timing curve for a transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    EaseOut,
    EaseInOut,
    Spring { response_ms: u16, damping: f32 },
}

/// A transition with its Reduce Motion replacement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u16,
    pub reduce_duration_ms: u16,
    pub curve: Curve,
}

/// Every motion token in the product.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub reduced_motion: bool,

    pub fast: MotionSpec,
    pub standard: MotionSpec,
    pub deliberate: MotionSpec,
    pub spring_popover: MotionSpec,

    pub dock_magnify_follows_pointer: bool,

    pub menu_open_ms: u16,
    pub menu_fade_out_ms: u16,
    pub menu_select_blink_ms: u16,
    pub menu_select_blink_count: u8,

    pub tooltip_delay_ms: u16,
    pub tooltip_move_delay_ms: u16,

    pub osd_hold_ms: u16,
    pub osd_fade_ms: u16,
    pub osd_reduce_fade_ms: u16,

    pub banner_slide_ms: u16,
    pub banner_auto_dismiss_ms: u16,
    pub banner_reduce_fade_ms: u16,

    pub dock_bounce_ms: u16,
    pub dock_bounce_count: u8,
    pub dock_bounce_height_ratio: f32,
    pub dock_bounce_max_ms: u16,

    pub minimize_ms: u16,
    pub minimize_reduce_fade_ms: u16,

    pub cc_open_ms: u16,
    pub cc_open_reduce_fade_ms: u16,
    pub spotlight_open_ms: u16,
    pub spotlight_open_reduce_fade_ms: u16,
    pub apps_open_ms: u16,
    pub apps_open_reduce_fade_ms: u16,
}

impl Motion {
    pub fn resolve(preference: MotionPreference) -> Self {
        Self {
            reduced_motion: preference == MotionPreference::Reduced,

            fast: MotionSpec {
                duration_ms: 120,
                reduce_duration_ms: 80,
                curve: Curve::EaseOut,
            },
            standard: MotionSpec {
                duration_ms: 200,
                reduce_duration_ms: 100,
                curve: Curve::EaseInOut,
            },
            deliberate: MotionSpec {
                duration_ms: 350,
                reduce_duration_ms: 100,
                curve: Curve::EaseInOut,
            },
            spring_popover: MotionSpec {
                duration_ms: 300,
                reduce_duration_ms: 100,
                curve: Curve::Spring {
                    response_ms: 300,
                    damping: 0.85,
                },
            },

            dock_magnify_follows_pointer: true,

            menu_open_ms: 0,
            menu_fade_out_ms: 150,
            menu_select_blink_ms: 70,
            menu_select_blink_count: 2,

            tooltip_delay_ms: 700,
            tooltip_move_delay_ms: 0,

            osd_hold_ms: 1600,
            osd_fade_ms: 250,
            osd_reduce_fade_ms: 100,

            banner_slide_ms: 350,
            banner_auto_dismiss_ms: 5000,
            banner_reduce_fade_ms: 100,

            dock_bounce_ms: 600,
            dock_bounce_count: 2,
            dock_bounce_height_ratio: 0.2,
            dock_bounce_max_ms: 10_000,

            minimize_ms: 350,
            minimize_reduce_fade_ms: 150,

            cc_open_ms: 250,
            cc_open_reduce_fade_ms: 100,
            spotlight_open_ms: 180,
            spotlight_open_reduce_fade_ms: 100,
            apps_open_ms: 300,
            apps_open_reduce_fade_ms: 100,
        }
    }
}
