//! Pointer drag of a Dock tile: live reordering and drag-off removal.
//!
//! Measured on macOS 26.2 (docs/dock-behaviour-2026-09-23.md) at the 64 pt
//! tile size:
//! - the dragged icon follows the pointer freely; its slot stays open in the
//!   kept-apps group and a neighbour slides over only once the pointer has
//!   passed that neighbour's centre, taking about 270 ms to slide;
//! - pulling the icon about 96 pt (1.5 tiles) beyond the shelf's inner edge
//!   shows a "Remove" label above it; releasing there removes the app, with
//!   the icon and label fading out over about 200 ms;
//! - releasing anywhere else drops the icon into its open slot (250 ms).
//!
//! This module holds only geometry-free decisions. The renderer supplies the
//! resting centres of the tiles it drew, so magnification or animation never
//! feed back into the thresholds.

/// Movement before a press becomes a drag (rmac value; the Mac's is similar).
pub const DRAG_THRESHOLD: f32 = crate::drag::DRAG_THRESHOLD;
/// Distance beyond the shelf's inner edge, in tiles, at which "Remove" shows.
pub const REMOVE_DISTANCE_TILES: f32 = 1.5;
/// Neighbour slide when the open slot moves.
pub const SLIDE_MS: u64 = 270;
/// Dropped icon settling into its slot.
pub const SETTLE_MS: u64 = 250;
/// Removed icon and its label fading out.
pub const REMOVE_FADE_MS: u64 = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileDrop {
    /// Never crossed the threshold: an ordinary click.
    Click,
    /// Dropped back into its own slot.
    NoChange,
    /// Dropped into another slot of the reorderable group.
    Move { from: usize, to: usize },
    /// Released past the removal distance.
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileDragState {
    pub active: bool,
    /// Index of the open slot in the reorderable group.
    pub destination: usize,
    /// "Remove" is showing; releasing now removes the app.
    pub remove_armed: bool,
    /// Something the renderer shows changed since the previous update.
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TileDrag {
    source: usize,
    centers: Vec<f32>,
    press_axis: f32,
    press_lift: f32,
    tile: f32,
    removable: bool,
    active: bool,
    destination: usize,
    remove_armed: bool,
}

impl TileDrag {
    /// Begin a press on tile `source` of a reorderable group whose resting
    /// centres (along the Dock axis, ascending) are `centers`. `lift` is the
    /// pointer's distance beyond the shelf's inner edge (negative inside the
    /// shelf). Returns `None` for malformed input.
    pub fn begin(
        source: usize,
        centers: Vec<f32>,
        press_axis: f32,
        press_lift: f32,
        tile: f32,
        removable: bool,
    ) -> Option<Self> {
        let usable = source < centers.len()
            && press_axis.is_finite()
            && press_lift.is_finite()
            && tile.is_finite()
            && tile > 0.0
            && centers.iter().all(|center| center.is_finite())
            && centers.windows(2).all(|pair| pair[0] < pair[1]);
        if !usable {
            return None;
        }
        Some(Self {
            source,
            centers,
            press_axis,
            press_lift,
            tile,
            removable,
            active: false,
            destination: source,
            remove_armed: false,
        })
    }

    pub fn source(&self) -> usize {
        self.source
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn state(&self) -> TileDragState {
        TileDragState {
            active: self.active,
            destination: self.destination,
            remove_armed: self.remove_armed,
            changed: false,
        }
    }

    /// Feed one pointer position.
    pub fn update(&mut self, axis: f32, lift: f32) -> TileDragState {
        if !axis.is_finite() || !lift.is_finite() {
            return self.state();
        }
        let before = (self.active, self.destination, self.remove_armed);
        if !self.active
            && ((axis - self.press_axis).abs() >= DRAG_THRESHOLD
                || (lift - self.press_lift).abs() >= DRAG_THRESHOLD)
        {
            self.active = true;
        }
        if self.active {
            self.destination = destination_for(&self.centers, self.source, axis);
            self.remove_armed = self.removable && lift >= REMOVE_DISTANCE_TILES * self.tile;
        }
        let mut state = self.state();
        state.changed = before != (self.active, self.destination, self.remove_armed);
        state
    }

    pub fn finish(&self) -> TileDrop {
        if !self.active {
            TileDrop::Click
        } else if self.remove_armed {
            TileDrop::Remove
        } else if self.destination == self.source {
            TileDrop::NoChange
        } else {
            TileDrop::Move {
                from: self.source,
                to: self.destination,
            }
        }
    }

    /// The group order the renderer shows: indices into the original group,
    /// with the dragged tile at its open slot.
    pub fn preview_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.centers.len()).collect();
        if self.active {
            let moved = order.remove(self.source);
            order.insert(self.destination, moved);
        }
        order
    }
}

/// The open slot for a pointer at `axis`: the dragged tile moves past a
/// neighbour once the pointer passes that neighbour's resting centre.
pub fn destination_for(centers: &[f32], source: usize, axis: f32) -> usize {
    centers
        .iter()
        .enumerate()
        .filter(|(index, center)| *index != source && **center < axis)
        .count()
        .min(centers.len().saturating_sub(1))
}

/// The tile whose slot contains `axis`, given resting centres and the slot
/// pitch (tile plus gap). Used for drop targets and hover.
pub fn slot_at(centers: &[f32], pitch: f32, axis: f32) -> Option<usize> {
    if !(axis.is_finite() && pitch.is_finite() && pitch > 0.0) {
        return None;
    }
    centers
        .iter()
        .position(|center| (axis - center).abs() <= pitch / 2.0)
}

/// Ease-out progress of a slide or settle animation.
pub fn ease_out(elapsed_ms: u64, duration_ms: u64) -> f32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return 1.0;
    }
    let t = elapsed_ms as f32 / duration_ms as f32;
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILE: f32 = 64.0;

    fn centers() -> Vec<f32> {
        // Five kept apps at the measured 68 pt pitch.
        (0..5).map(|index| 100.0 + 68.0 * index as f32).collect()
    }

    #[test]
    fn small_movement_is_a_click() {
        let mut drag = TileDrag::begin(1, centers(), 168.0, -30.0, TILE, true).unwrap();
        let state = drag.update(170.0, -29.0);
        assert!(!state.active);
        assert_eq!(drag.finish(), TileDrop::Click);
        assert_eq!(drag.preview_order(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn neighbour_moves_only_after_the_pointer_passes_its_centre() {
        let mut drag = TileDrag::begin(1, centers(), 168.0, -30.0, TILE, true).unwrap();
        // Over the right neighbour but short of its centre (236): no change.
        let state = drag.update(230.0, -30.0);
        assert!(state.active);
        assert_eq!(state.destination, 1);
        // Past its centre: the slot opens one to the right.
        let state = drag.update(240.0, -30.0);
        assert_eq!(state.destination, 2);
        assert!(state.changed);
        assert_eq!(drag.preview_order(), [0, 2, 1, 3, 4]);
        // Far right: last slot.
        assert_eq!(drag.update(900.0, -30.0).destination, 4);
        // Left of everything: first slot.
        assert_eq!(drag.update(10.0, -30.0).destination, 0);
        assert_eq!(drag.finish(), TileDrop::Move { from: 1, to: 0 });
    }

    #[test]
    fn returning_home_is_no_change() {
        let mut drag = TileDrag::begin(2, centers(), 236.0, -30.0, TILE, true).unwrap();
        drag.update(320.0, -30.0);
        drag.update(236.0, -30.0);
        assert_eq!(drag.finish(), TileDrop::NoChange);
    }

    #[test]
    fn remove_arms_at_one_and_a_half_tiles_beyond_the_shelf() {
        let mut drag = TileDrag::begin(0, centers(), 100.0, -30.0, TILE, true).unwrap();
        assert!(!drag.update(100.0, 95.0).remove_armed);
        let state = drag.update(100.0, 96.0);
        assert!(state.remove_armed && state.changed);
        assert_eq!(drag.finish(), TileDrop::Remove);
        // Back towards the Dock disarms it.
        assert!(!drag.update(100.0, 40.0).remove_armed);
        assert_eq!(drag.finish(), TileDrop::NoChange);
    }

    #[test]
    fn items_that_cannot_be_removed_never_arm() {
        let mut drag = TileDrag::begin(0, centers(), 100.0, -30.0, TILE, false).unwrap();
        assert!(!drag.update(100.0, 400.0).remove_armed);
        assert_eq!(drag.finish(), TileDrop::NoChange);
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(TileDrag::begin(5, centers(), 0.0, 0.0, TILE, true).is_none());
        assert!(TileDrag::begin(0, vec![10.0, 5.0], 0.0, 0.0, TILE, true).is_none());
        assert!(TileDrag::begin(0, centers(), f32::NAN, 0.0, TILE, true).is_none());
        assert!(TileDrag::begin(0, centers(), 0.0, 0.0, 0.0, true).is_none());
    }

    #[test]
    fn hit_testing_uses_half_the_pitch_either_side() {
        let centers = centers();
        assert_eq!(slot_at(&centers, 68.0, 100.0), Some(0));
        assert_eq!(slot_at(&centers, 68.0, 133.9), Some(0));
        assert_eq!(slot_at(&centers, 68.0, 135.0), Some(1));
        assert_eq!(slot_at(&centers, 68.0, 20.0), None);
        assert_eq!(slot_at(&centers, 68.0, f32::NAN), None);
    }

    #[test]
    fn ease_out_is_bounded_and_monotonic() {
        assert_eq!(ease_out(0, SLIDE_MS), 0.0);
        assert_eq!(ease_out(SLIDE_MS, SLIDE_MS), 1.0);
        assert!(ease_out(SLIDE_MS / 2, SLIDE_MS) > 0.5);
        let mut previous = 0.0;
        for elapsed in 0..=SLIDE_MS {
            let value = ease_out(elapsed, SLIDE_MS);
            assert!(value >= previous);
            previous = value;
        }
    }
}
