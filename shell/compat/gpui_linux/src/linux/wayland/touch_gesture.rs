//! rmac (docs/decisions/0028-touch-input.md): touchscreen gesture
//! recognition, kept free of Wayland types so it can be unit tested.
//!
//! One finger: a tap clicks, a rest of [`TOUCH_HOLD`] right-clicks, a swipe
//! scrolls (with the touchpad's kinetic momentum after a fling), and a move
//! on something draggable drags it. Two fingers: moving together scrolls,
//! spreading or closing pinches, and a quick two-finger tap right-clicks.
//! Three or more fingers are left alone.

use std::sync::OnceLock;
use std::time::Duration;

use gpui::{Pixels, Point, TouchDragMode, TouchPhase, point, px};

/// A finger moving less than this is still a tap or a hold.
pub(super) const TOUCH_MOVE_THRESHOLD: Pixels = px(8.0);
/// Resting this long without moving is a right-click (a long press).
pub(super) const TOUCH_HOLD: Duration = Duration::from_millis(500);
/// Resting this long before moving drags an `on_drag` element instead of
/// scrolling the list it sits in.
pub(super) const TOUCH_DRAG_HOLD: Duration = Duration::from_millis(160);
/// Two fingers whose midpoint moves this far scroll.
pub(super) const TWO_FINGER_SCROLL_SLOP: f32 = 10.0;
/// Two fingers whose spread changes by this fraction pinch.
pub(super) const PINCH_SLOP: f32 = 0.08;
/// A two-finger tap longer than this is not a right-click.
pub(super) const TWO_FINGER_TAP: Duration = Duration::from_millis(350);

/// `RMAC_TOUCH_TRACE=1` prints each contact's life to stderr, to see what a
/// touchscreen sends on a real PC. Read once per process.
pub(super) fn trace(args: std::fmt::Arguments<'_>) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("RMAC_TOUCH_TRACE").is_some()) {
        eprintln!("rmac-touch: {args}");
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TouchGesture {
    Pending,
    Scroll,
    PointerDrag,
    ContextMenu,
}

/// A quick swipe scrolls. Something marked to drag straight away (a title
/// bar, a slider, a Dock tile) drags; an `on_drag` element drags after a
/// short deliberate rest, so a list of draggable rows still scrolls.
pub(super) fn classify_touch_motion(
    start: Point<Pixels>,
    position: Point<Pixels>,
    elapsed: Duration,
    drag: Option<TouchDragMode>,
) -> TouchGesture {
    let delta = position - start;
    if delta.x.abs() <= TOUCH_MOVE_THRESHOLD && delta.y.abs() <= TOUCH_MOVE_THRESHOLD {
        return TouchGesture::Pending;
    }
    match drag {
        Some(TouchDragMode::Immediate) => TouchGesture::PointerDrag,
        Some(TouchDragMode::AfterHold) if elapsed >= TOUCH_DRAG_HOLD => TouchGesture::PointerDrag,
        _ => TouchGesture::Scroll,
    }
}

pub(super) fn classify_touch_hold(gesture: TouchGesture, elapsed: Duration) -> TouchGesture {
    if gesture == TouchGesture::Pending && elapsed >= TOUCH_HOLD {
        TouchGesture::ContextMenu
    } else {
        gesture
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TwoFingerKind {
    Pending,
    Scroll,
    Pinch,
}

/// What a two-finger contact turns into, in GPUI's terms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TwoFingerEvent {
    /// Natural scrolling: the content follows the fingers, the same sign
    /// as a one-finger swipe.
    Scroll {
        position: Point<Pixels>,
        delta: Point<Pixels>,
        phase: TouchPhase,
    },
    /// `delta` is the change in scale since the last event, as the
    /// touchpad pinch reports it.
    Pinch {
        position: Point<Pixels>,
        delta: f32,
        phase: TouchPhase,
    },
    /// A quick two-finger tap, which right-clicks.
    SecondaryTap { position: Point<Pixels> },
}

/// Two fingers on one window, from the second one landing until either
/// lifts.
#[derive(Clone, Debug)]
pub(super) struct TwoFinger {
    ids: [i32; 2],
    positions: [Point<Pixels>; 2],
    start_centroid: Point<Pixels>,
    start_spread: f32,
    last_centroid: Point<Pixels>,
    last_scale: f32,
    kind: TwoFingerKind,
}

fn centroid(positions: &[Point<Pixels>; 2]) -> Point<Pixels> {
    point(
        (positions[0].x + positions[1].x) / 2.0,
        (positions[0].y + positions[1].y) / 2.0,
    )
}

fn spread(positions: &[Point<Pixels>; 2]) -> f32 {
    let delta = positions[1] - positions[0];
    f32::from(delta.x).hypot(f32::from(delta.y)).max(1.0)
}

impl TwoFinger {
    pub(super) fn new(first: (i32, Point<Pixels>), second: (i32, Point<Pixels>)) -> Self {
        let positions = [first.1, second.1];
        let start_centroid = centroid(&positions);
        Self {
            ids: [first.0, second.0],
            positions,
            start_centroid,
            start_spread: spread(&positions),
            last_centroid: start_centroid,
            last_scale: 1.0,
            kind: TwoFingerKind::Pending,
        }
    }

    pub(super) fn contains(&self, id: i32) -> bool {
        self.ids.contains(&id)
    }

    pub(super) fn kind(&self) -> TwoFingerKind {
        self.kind
    }

    pub(super) fn centroid(&self) -> Point<Pixels> {
        centroid(&self.positions)
    }

    /// One finger moved. Up to two events: the gesture starting and its
    /// first movement.
    pub(super) fn update(&mut self, id: i32, position: Point<Pixels>) -> [Option<TwoFingerEvent>; 2] {
        let Some(index) = self.ids.iter().position(|known| *known == id) else {
            return [None, None];
        };
        self.positions[index] = position;
        let centroid = self.centroid();
        let scale = spread(&self.positions) / self.start_spread;
        let mut out = [None, None];
        if self.kind == TwoFingerKind::Pending {
            let moved = centroid - self.start_centroid;
            let scroll = f32::from(moved.x).hypot(f32::from(moved.y)) / TWO_FINGER_SCROLL_SLOP;
            let pinch = (scale - 1.0).abs() / PINCH_SLOP;
            if scroll < 1.0 && pinch < 1.0 {
                self.last_centroid = centroid;
                return out;
            }
            if pinch >= scroll {
                self.kind = TwoFingerKind::Pinch;
                out[0] = Some(TwoFingerEvent::Pinch {
                    position: centroid,
                    delta: 0.0,
                    phase: TouchPhase::Started,
                });
            } else {
                self.kind = TwoFingerKind::Scroll;
                let delta = centroid - self.last_centroid;
                self.last_centroid = centroid;
                out[0] = Some(TwoFingerEvent::Scroll {
                    position: centroid,
                    delta,
                    phase: TouchPhase::Started,
                });
                return out;
            }
        }
        match self.kind {
            TwoFingerKind::Scroll => {
                let delta = centroid - self.last_centroid;
                self.last_centroid = centroid;
                out[0] = Some(TwoFingerEvent::Scroll {
                    position: centroid,
                    delta,
                    phase: TouchPhase::Moved,
                });
            }
            TwoFingerKind::Pinch => {
                let delta = scale - self.last_scale;
                self.last_scale = scale;
                let event = TwoFingerEvent::Pinch {
                    position: centroid,
                    delta,
                    phase: TouchPhase::Moved,
                };
                if out[0].is_some() {
                    out[1] = Some(event);
                } else {
                    out[0] = Some(event);
                }
            }
            TwoFingerKind::Pending => {}
        }
        out
    }

    /// A finger lifted (or the contact was cancelled) after `elapsed`.
    pub(super) fn end(&self, elapsed: Duration, cancelled: bool) -> Option<TwoFingerEvent> {
        let position = self.centroid();
        match self.kind {
            TwoFingerKind::Pending if !cancelled && elapsed < TWO_FINGER_TAP => {
                Some(TwoFingerEvent::SecondaryTap { position })
            }
            TwoFingerKind::Pending => None,
            TwoFingerKind::Scroll => Some(TwoFingerEvent::Scroll {
                position,
                delta: point(px(0.0), px(0.0)),
                phase: TouchPhase::Ended,
            }),
            TwoFingerKind::Pinch => Some(TwoFingerEvent::Pinch {
                position,
                delta: 0.0,
                phase: TouchPhase::Ended,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn one_finger_threshold_and_drag_modes() {
        let start = at(10.0, 20.0);
        let near = at(18.0, 20.0);
        let moved = at(19.0, 20.0);
        let hold = Some(TouchDragMode::AfterHold);
        let now = Some(TouchDragMode::Immediate);
        assert_eq!(
            classify_touch_motion(start, near, Duration::ZERO, now),
            TouchGesture::Pending
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::ZERO, None),
            TouchGesture::Scroll
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::ZERO, now),
            TouchGesture::PointerDrag
        );
        assert_eq!(
            classify_touch_motion(start, moved, Duration::from_millis(159), hold),
            TouchGesture::Scroll
        );
        assert_eq!(
            classify_touch_motion(start, moved, TOUCH_DRAG_HOLD, hold),
            TouchGesture::PointerDrag
        );
    }

    #[test]
    fn hold_only_opens_context_menu_without_movement() {
        assert_eq!(
            classify_touch_hold(TouchGesture::Pending, TOUCH_HOLD - Duration::from_millis(1)),
            TouchGesture::Pending
        );
        assert_eq!(
            classify_touch_hold(TouchGesture::Pending, TOUCH_HOLD),
            TouchGesture::ContextMenu
        );
        assert_eq!(
            classify_touch_hold(TouchGesture::Scroll, TOUCH_HOLD),
            TouchGesture::Scroll
        );
    }

    #[test]
    fn two_fingers_moving_together_scroll_naturally() {
        let mut gesture = TwoFinger::new((1, at(100.0, 100.0)), (2, at(200.0, 100.0)));
        // Under the slop: nothing yet.
        assert_eq!(gesture.update(1, at(100.0, 108.0)), [None, None]);
        assert_eq!(gesture.update(2, at(200.0, 108.0)), [None, None]);
        assert_eq!(gesture.kind(), TwoFingerKind::Pending);
        let [started, none] = gesture.update(1, at(100.0, 130.0));
        assert_eq!(none, None);
        let Some(TwoFingerEvent::Scroll { delta, phase, position }) = started else {
            panic!("expected a scroll start, got {started:?}");
        };
        assert_eq!(phase, TouchPhase::Started);
        assert_eq!(position, at(150.0, 119.0));
        // Fingers moved down: the same sign as a one-finger swipe down.
        assert_eq!(delta, at(0.0, 11.0));
        let [moved, _] = gesture.update(2, at(200.0, 130.0));
        assert_eq!(
            moved,
            Some(TwoFingerEvent::Scroll {
                position: at(150.0, 130.0),
                delta: at(0.0, 11.0),
                phase: TouchPhase::Moved,
            })
        );
        assert_eq!(
            gesture.end(Duration::from_millis(400), false),
            Some(TwoFingerEvent::Scroll {
                position: at(150.0, 130.0),
                delta: at(0.0, 0.0),
                phase: TouchPhase::Ended,
            })
        );
    }

    #[test]
    fn spreading_fingers_pinch_with_incremental_scale() {
        let mut gesture = TwoFinger::new((1, at(100.0, 100.0)), (2, at(200.0, 100.0)));
        // 5% wider: under the pinch slop.
        assert_eq!(gesture.update(2, at(205.0, 100.0)), [None, None]);
        // The left finger moves out too: the spread outgrows the midpoint shift.
        let events = gesture.update(1, at(85.0, 100.0));
        assert_eq!(gesture.kind(), TwoFingerKind::Pinch);
        assert_eq!(
            events[0],
            Some(TwoFingerEvent::Pinch {
                position: at(145.0, 100.0),
                delta: 0.0,
                phase: TouchPhase::Started,
            })
        );
        let Some(TwoFingerEvent::Pinch { delta, phase, .. }) = events[1] else {
            panic!("expected a pinch update, got {events:?}");
        };
        assert_eq!(phase, TouchPhase::Moved);
        assert!((delta - 0.2).abs() < 1e-4, "{delta}");
        // Closing back to the start spread zooms out by the same amount.
        let [closing, _] = gesture.update(1, at(105.0, 100.0));
        let Some(TwoFingerEvent::Pinch { delta, .. }) = closing else {
            panic!("expected a pinch update, got {closing:?}");
        };
        assert!((delta + 0.2).abs() < 1e-4, "{delta}");
        // Once pinching, moving the midpoint does not turn it into a scroll.
        let [still_pinch, _] = gesture.update(2, at(205.0, 160.0));
        assert!(matches!(still_pinch, Some(TwoFingerEvent::Pinch { .. })));
        assert!(matches!(
            gesture.end(Duration::from_secs(1), false),
            Some(TwoFingerEvent::Pinch { phase: TouchPhase::Ended, .. })
        ));
    }

    #[test]
    fn quick_two_finger_tap_right_clicks_and_unknown_ids_are_ignored() {
        let mut gesture = TwoFinger::new((4, at(100.0, 100.0)), (9, at(140.0, 100.0)));
        assert_eq!(gesture.update(5, at(0.0, 0.0)), [None, None]);
        assert!(gesture.contains(9) && !gesture.contains(5));
        assert_eq!(
            gesture.end(Duration::from_millis(120), false),
            Some(TwoFingerEvent::SecondaryTap { position: at(120.0, 100.0) })
        );
        assert_eq!(gesture.end(TWO_FINGER_TAP, false), None);
        assert_eq!(gesture.end(Duration::from_millis(120), true), None);
    }

    #[test]
    fn fingers_landing_on_one_point_do_not_divide_by_zero() {
        let mut gesture = TwoFinger::new((1, at(50.0, 50.0)), (2, at(50.0, 50.0)));
        let events = gesture.update(2, at(80.0, 50.0));
        assert_eq!(gesture.kind(), TwoFingerKind::Pinch);
        assert!(events.iter().flatten().all(|event| match event {
            TwoFingerEvent::Pinch { delta, .. } => delta.is_finite(),
            _ => false,
        }));
    }
}
