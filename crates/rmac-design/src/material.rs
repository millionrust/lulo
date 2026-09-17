//! Functional glass materials: tint over compositor blur, with an opaque
//! reduced-transparency fallback.

use rmac_appearance::{Contrast, ResolvedColorScheme};

use crate::color::{Colors, Rgba};

/// One material surface.
///
/// The compositor supplies the blur behind the surface (niri `background-effect
/// { blur true }`); the surface paints `tint` and an optional inner highlight
/// border. `fallback` is used when Reduce Transparency is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub tint: Rgba,
    pub blur: bool,
    pub border: Rgba,
    pub highlight: Rgba,
    pub fallback: Rgba,
}

/// Every material role in the product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Materials {
    pub menu: Material,
    pub popover: Material,
    pub hud: Material,
    pub dock: Material,
    pub sidebar: Material,
    pub menubar: Material,
    pub tooltip: Material,
}

impl Materials {
    pub fn resolve(scheme: ResolvedColorScheme, contrast: Contrast, colors: Colors) -> Self {
        let high = contrast == Contrast::Higher;
        match scheme {
            ResolvedColorScheme::Light => {
                let menu = Material {
                    tint: Rgba::from_rgba(0xf6f6f6d9),
                    blur: true,
                    border: Rgba::from_rgba(0x0000001a),
                    highlight: Rgba::from_rgba(0xffffff66),
                    fallback: colors.surface_raised,
                };
                let popover = Material {
                    tint: Rgba::from_rgba(0xf2f2f2cc),
                    ..menu
                };
                let hud = Material {
                    tint: Rgba::from_rgba(0xffffffb3),
                    ..menu
                };
                let dock = Material {
                    tint: Rgba::from_rgba(0xffffff40),
                    blur: true,
                    border: Rgba::from_rgba(0xffffff59),
                    highlight: Rgba::from_rgba(0x00000014),
                    fallback: Rgba::from_rgba(0xf0f0f0f2),
                };
                let sidebar = Material {
                    tint: Rgba::from_rgba(0xf2f2f2e0),
                    blur: true,
                    border: Rgba::TRANSPARENT,
                    highlight: Rgba::TRANSPARENT,
                    fallback: colors.surface_sidebar_opaque,
                };
                let menubar = Material {
                    tint: Rgba::TRANSPARENT,
                    blur: false,
                    border: Rgba::TRANSPARENT,
                    highlight: Rgba::TRANSPARENT,
                    fallback: Rgba::from_rgba(0xf6f6f6f2),
                };
                let tooltip = Material {
                    tint: Rgba::from_rgba(0xfafafaf2),
                    blur: false,
                    border: colors.separator,
                    highlight: Rgba::TRANSPARENT,
                    fallback: Rgba::from_rgba(0xfafafaf2),
                };
                Self {
                    menu: harden(menu, high, colors),
                    popover: harden(popover, high, colors),
                    hud: harden(hud, high, colors),
                    dock: harden(dock, high, colors),
                    sidebar: harden(sidebar, high, colors),
                    menubar: harden(menubar, high, colors),
                    tooltip: harden(tooltip, high, colors),
                }
            }
            ResolvedColorScheme::Dark => {
                let menu = Material {
                    tint: Rgba::from_rgba(0x28282bd9),
                    blur: true,
                    border: Rgba::from_rgba(0xffffff1f),
                    highlight: Rgba::from_rgba(0xffffff14),
                    fallback: colors.surface_raised,
                };
                let popover = Material {
                    tint: Rgba::from_rgba(0x232326cc),
                    ..menu
                };
                let hud = Material {
                    tint: Rgba::from_rgba(0x1c1c1eb3),
                    ..menu
                };
                let dock = Material {
                    tint: Rgba::from_rgba(0x00000033),
                    blur: true,
                    border: Rgba::from_rgba(0xffffff59),
                    highlight: Rgba::from_rgba(0x00000014),
                    fallback: Rgba::from_rgba(0x2a2a2df2),
                };
                let sidebar = Material {
                    tint: Rgba::from_rgba(0x242426e0),
                    blur: true,
                    border: Rgba::TRANSPARENT,
                    highlight: Rgba::TRANSPARENT,
                    fallback: colors.surface_sidebar_opaque,
                };
                let menubar = Material {
                    tint: Rgba::TRANSPARENT,
                    blur: false,
                    border: Rgba::TRANSPARENT,
                    highlight: Rgba::TRANSPARENT,
                    fallback: Rgba::from_rgba(0x1e1e20f2),
                };
                let tooltip = Material {
                    tint: Rgba::from_rgba(0x2c2c2ef2),
                    blur: false,
                    border: colors.separator,
                    highlight: Rgba::TRANSPARENT,
                    fallback: Rgba::from_rgba(0x2c2c2ef2),
                };
                Self {
                    menu: harden(menu, high, colors),
                    popover: harden(popover, high, colors),
                    hud: harden(hud, high, colors),
                    dock: harden(dock, high, colors),
                    sidebar: harden(sidebar, high, colors),
                    menubar: harden(menubar, high, colors),
                    tooltip: harden(tooltip, high, colors),
                }
            }
        }
    }
}

/// High contrast forces near-opaque tints and a 1 px `label.secondary` border.
fn harden(material: Material, high: bool, colors: Colors) -> Material {
    if high {
        Material {
            tint: material.tint.with_alpha(0xf6),
            border: if material.tint.alpha() == 0 {
                material.border
            } else {
                colors.label_secondary
            },
            highlight: Rgba::TRANSPARENT,
            ..material
        }
    } else {
        material
    }
}
