//! ⌃F3 keyboard navigation across the Dock's tiles.
//!
//! Measured on macOS 26 (design-lab/dock.html, scene 3): ⌃F3 focuses the
//! first tile; ← / → and Tab / ⇧Tab move one tile and stop at the ends
//! (← on the first tile stays put); Return or Space opens the tile; ↑ opens
//! its Dock menu; Esc gives the keyboard back to the previous window. A side
//! Dock uses ↑ / ↓ to move and the arrow pointing away from its edge for the
//! menu, as the menu opens that way.
//!
//! Nothing here touches GPUI, so the rules are tested on every platform.

/// The screen edge the Dock sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Previous,
    Next,
    Activate,
    OpenMenu,
    Escape,
    Other,
}

impl Key {
    /// Map a GPUI key name (`KeyDownEvent::keystroke.key`) for a Dock on
    /// `edge`.
    pub fn from_name(name: &str, shift: bool, edge: Edge) -> Self {
        match (name, edge) {
            ("tab", _) if shift => Self::Previous,
            ("tab", _) => Self::Next,
            ("enter" | "space", _) => Self::Activate,
            ("escape", _) => Self::Escape,
            ("left", Edge::Bottom) | ("up", Edge::Left | Edge::Right) => Self::Previous,
            ("right", Edge::Bottom) | ("down", Edge::Left | Edge::Right) => Self::Next,
            ("up", Edge::Bottom) | ("right", Edge::Left) | ("left", Edge::Right) => Self::OpenMenu,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The focused tile changed; announce and redraw.
    Moved,
    /// Nothing to do (an end was reached or the key is not the Dock's).
    Unchanged,
    /// Open the focused tile, then leave keyboard mode.
    Activate,
    /// Open the focused tile's Dock menu; keyboard mode continues in it.
    OpenMenu,
    /// Leave keyboard mode and restore the previous window's focus.
    Dismiss,
}

/// Which of the Dock's `len` tiles (applications, minimized windows, Trash,
/// in shelf order) holds keyboard focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Navigator {
    index: usize,
    len: usize,
}

impl Navigator {
    /// Focus the first tile. A Dock always has Trash, so `len` is never 0 in
    /// practice; `None` covers a Dock that has not loaded yet.
    pub fn new(len: usize) -> Option<Self> {
        (len > 0).then_some(Self { index: 0, len })
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// Follow the shelf changing under the focus (an app quit or launched):
    /// keep the index, clamped to the new last tile.
    pub fn set_len(&mut self, len: usize) -> bool {
        if len == 0 {
            return false;
        }
        self.len = len;
        self.index = self.index.min(len - 1);
        true
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        match key {
            Key::Previous if self.index > 0 => {
                self.index -= 1;
                Outcome::Moved
            }
            Key::Next if self.index + 1 < self.len => {
                self.index += 1;
                Outcome::Moved
            }
            Key::Previous | Key::Next | Key::Other => Outcome::Unchanged,
            Key::Activate => Outcome::Activate,
            Key::OpenMenu => Outcome::OpenMenu,
            Key::Escape => Outcome::Dismiss,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_starts_on_the_first_tile_and_stops_at_both_ends() {
        let mut navigator = Navigator::new(3).unwrap();
        assert_eq!(navigator.index(), 0);
        assert_eq!(navigator.handle(Key::Previous), Outcome::Unchanged);
        assert_eq!(navigator.handle(Key::Next), Outcome::Moved);
        assert_eq!(navigator.handle(Key::Next), Outcome::Moved);
        assert_eq!(navigator.index(), 2);
        assert_eq!(navigator.handle(Key::Next), Outcome::Unchanged);
        assert_eq!(navigator.index(), 2);
        assert!(Navigator::new(0).is_none());
    }

    #[test]
    fn keys_follow_the_mac_for_every_edge() {
        let bottom = Edge::Bottom;
        assert_eq!(Key::from_name("left", false, bottom), Key::Previous);
        assert_eq!(Key::from_name("right", false, bottom), Key::Next);
        assert_eq!(Key::from_name("tab", false, bottom), Key::Next);
        assert_eq!(Key::from_name("tab", true, bottom), Key::Previous);
        assert_eq!(Key::from_name("up", false, bottom), Key::OpenMenu);
        assert_eq!(Key::from_name("enter", false, bottom), Key::Activate);
        assert_eq!(Key::from_name("space", false, bottom), Key::Activate);
        assert_eq!(Key::from_name("escape", false, bottom), Key::Escape);
        assert_eq!(Key::from_name("down", false, bottom), Key::Other);
        assert_eq!(Key::from_name("a", false, bottom), Key::Other);
        assert_eq!(Key::from_name("up", false, Edge::Left), Key::Previous);
        assert_eq!(Key::from_name("down", false, Edge::Left), Key::Next);
        assert_eq!(Key::from_name("right", false, Edge::Left), Key::OpenMenu);
        assert_eq!(Key::from_name("left", false, Edge::Right), Key::OpenMenu);
        assert_eq!(Key::from_name("right", false, Edge::Right), Key::Other);
    }

    #[test]
    fn a_shrinking_shelf_keeps_focus_on_a_real_tile() {
        let mut navigator = Navigator::new(5).unwrap();
        for _ in 0..4 {
            navigator.handle(Key::Next);
        }
        assert!(navigator.set_len(2));
        assert_eq!(navigator.index(), 1);
        assert!(!navigator.set_len(0));
        assert_eq!(navigator.handle(Key::Activate), Outcome::Activate);
        assert_eq!(navigator.handle(Key::OpenMenu), Outcome::OpenMenu);
        assert_eq!(navigator.handle(Key::Escape), Outcome::Dismiss);
    }
}
