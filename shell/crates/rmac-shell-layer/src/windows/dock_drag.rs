//! An app dragged out of Apps onto the Dock (macOS 26: drag from
//! Launchpad/Apps to keep an app in the Dock). On Windows Apps and the Dock
//! are windows of one process, `lulo-shell`, and GPUI has no drag source
//! there; Windows captures the pointer for the window that took the press,
//! so Apps keeps receiving the drag's motion after it leaves Apps and
//! reports it here in screen coordinates. The Dock registers the sink and
//! opens the slot under the pointer exactly as for a file dragged in.

use std::rc::Rc;

use gpui::{App, Global, Pixels, Point};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// The drag moved; the pointer is at `screen`.
    Hover,
    /// The button was released at `screen`.
    Drop,
    /// The drag ended without a drop.
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    /// The dragged application's id (its desktop entry's name).
    pub app_id: String,
    pub phase: Phase,
    /// Logical screen coordinates.
    pub screen: Point<Pixels>,
}

type SinkFn = dyn Fn(Hint, &mut App);

struct Sink(Rc<SinkFn>);

impl Global for Sink {}

/// The Dock receives every hint Apps sends from now on.
pub fn set_sink(cx: &mut App, sink: impl Fn(Hint, &mut App) + 'static) {
    cx.set_global(Sink(Rc::new(sink)));
}

/// Send a hint to the Dock, after the current event finishes so the Dock
/// is never updated from inside Apps' own update. Without a Dock nothing
/// happens.
pub fn send(cx: &mut App, hint: Hint) {
    let Some(sink) = cx.try_global::<Sink>().map(|sink| sink.0.clone()) else {
        return;
    };
    cx.defer(move |cx| sink(hint, cx));
}
