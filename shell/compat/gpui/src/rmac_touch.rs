//! rmac (docs/decisions/0028-touch-input.md): what a touchscreen backend
//! needs from GPUI core.
//!
//! GPUI has no touch events. The Linux backend turns touch contacts into
//! pointer clicks, scrolls and pinches; it asks the window which kind of
//! element a contact landed on ([`TouchDragMode`]) and records that the
//! last input came from a finger, so surfaces can grow their hit targets
//! while (and only while) the PC is being used by touch.

use std::sync::atomic::{AtomicBool, Ordering};

static LAST_INPUT_TOUCH: AtomicBool = AtomicBool::new(false);

/// How a finger that moves after landing on an element is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum TouchDragMode {
    /// The element drags only after the finger rests a moment first; a
    /// quick swipe still scrolls. Any element with a drag listener
    /// (`on_drag`) gets this, so rows in a list stay scrollable.
    AfterHold,
    /// The finger drags the element as soon as it moves, as a held mouse
    /// button would: sliders, Dock tiles and other surfaces that do not
    /// scroll. Set with `InteractiveElement::touch_drag`.
    Immediate,
}

/// Whether the most recent pointing input in this process was a touch
/// contact rather than a mouse, touchpad or pointing stick.
pub fn last_input_was_touch() -> bool {
    LAST_INPUT_TOUCH.load(Ordering::Relaxed)
}

/// Record the modality of the latest pointing input. The backend calls
/// this; each window refreshes on its next event when the value changes.
/// Returns whether it changed.
pub fn set_last_input_was_touch(touch: bool) -> bool {
    LAST_INPUT_TOUCH.swap(touch, Ordering::Relaxed) != touch
}
