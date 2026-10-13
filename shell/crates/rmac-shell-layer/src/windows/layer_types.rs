//! `gpui::layer_shell`'s vocabulary on Windows, where GPUI has no layer
//! shell: the shell's views describe every surface the same way on both
//! platforms, and [`super::open_layer_window`] turns the description into a
//! Win32 window (ADR 0023, "Phase 3 revised: shared shell views").

use std::ops::{BitAnd, BitOr, BitOrAssign};

use gpui::Pixels;

/// The layer the surface is drawn on, bottom to top.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layer {
    /// The wallpaper and the desktop: just above Explorer's desktop, below
    /// every app window.
    Background,
    /// Below app windows, above the background.
    Bottom,
    /// Above app windows (the Dock, menu materials).
    Top,
    /// Above everything else (the menu bar and its menus).
    #[default]
    Overlay,
}

/// The screen edges a surface is anchored to; both of two opposite edges
/// stretch it across the screen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor(u32);

impl Anchor {
    pub const TOP: Self = Self(1);
    pub const BOTTOM: Self = Self(2);
    pub const LEFT: Self = Self(4);
    pub const RIGHT: Self = Self(8);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Anchor {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Anchor {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitAnd for Anchor {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// How the surface takes the keyboard.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyboardInteractivity {
    /// Never: a click leaves the app in front with the keyboard.
    None,
    /// At once, as soon as it opens.
    Exclusive,
    /// When clicked, as a normal window.
    #[default]
    OnDemand,
}

/// A surface's place on the screen, as `gpui::layer_shell::LayerShellOptions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerShellOptions {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    /// A strip this deep along the anchored edge is kept free of app
    /// windows (an AppBar); negative ignores other surfaces' strips.
    pub exclusive_zone: Option<Pixels>,
    pub exclusive_edge: Option<Anchor>,
    /// Top, right, bottom, left.
    pub margin: Option<(Pixels, Pixels, Pixels, Pixels)>,
    pub keyboard_interactivity: KeyboardInteractivity,
}
