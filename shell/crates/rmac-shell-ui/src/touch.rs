//! Bigger hit targets while the PC is used by touch
//! (docs/decisions/0028-touch-input.md).
//!
//! A finger is less precise than a pointer, so small targets (traffic
//! lights, menu bar items) reach a little further around themselves, but
//! only while the latest input was a touch on a PC whose hardware profile
//! has a touchscreen (`rmac_hardware::Adaptations::touch_targets`). A mouse
//! move puts them back. Nothing is drawn differently: callers grow the hit
//! box and pull it back in with an equal negative margin, so the layout
//! and the visuals stay put.

use std::sync::OnceLock;

use gpui::{px, Styled};

/// How far a small target's hit box reaches past its drawn edge on touch.
pub const TOUCH_SLOP: f32 = 6.0;

/// The hardware profile says this PC has a touchscreen. Without a profile
/// (detection not run in this process yet, or none stored) a touch that
/// already happened is proof enough.
fn hardware_has_touchscreen() -> bool {
    static STORED: OnceLock<bool> = OnceLock::new();
    if let Some(profile) = rmac_hardware::current() {
        return profile.adaptations().touch_targets;
    }
    *STORED.get_or_init(|| {
        rmac_hardware::store::load().is_none_or(|profile| profile.adaptations().touch_targets)
    })
}

/// The pure rule, for tests: touch targets only after a touch, and only on
/// touchscreen hardware.
pub fn touch_targets_for(last_input_was_touch: bool, touchscreen: impl FnOnce() -> bool) -> bool {
    last_input_was_touch && touchscreen()
}

/// Whether small targets should grow right now.
pub fn touch_targets() -> bool {
    touch_targets_for(gpui::last_input_was_touch(), hardware_has_touchscreen)
}

/// The extra reach on each side right now: [`TOUCH_SLOP`] or nothing.
pub fn touch_slop() -> f32 {
    if touch_targets() {
        TOUCH_SLOP
    } else {
        0.0
    }
}

/// Size `element` to `width` x `height`, reaching `slop` further on each
/// side for input without moving anything: the box grows and an equal
/// negative margin keeps its footprint. Its content stays centred only if
/// the element centres it (flex `items_center`/`justify_center`).
pub fn hit_size<E: Styled>(element: E, width: f32, height: f32, slop: f32) -> E {
    let element = element.w(px(width + 2.0 * slop)).h(px(height + 2.0 * slop));
    if slop > 0.0 {
        element.m(px(-slop))
    } else {
        element
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_grow_only_after_touch_on_touch_hardware() {
        assert!(touch_targets_for(true, || true));
        assert!(!touch_targets_for(true, || false));
        assert!(!touch_targets_for(false, || true));
        // The hardware is not even asked while a mouse is in use.
        assert!(!touch_targets_for(false, || panic!(
            "hardware read on mouse input"
        )));
    }

    #[test]
    fn no_slop_without_touch() {
        gpui::set_last_input_was_touch(false);
        assert_eq!(touch_slop(), 0.0);
    }
}
