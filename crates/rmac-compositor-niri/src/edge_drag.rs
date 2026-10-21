//! Edge-drag tiling (macOS 26): dragging a window to a screen edge or corner
//! previews a tile there, and letting go tiles the window — a half from the
//! left or right edge, a quarter from a corner, the whole working area from
//! the top edge. Lulo's niri reports where the pointer is during a drag
//! (`WindowMoveEdgeChanged`, `WindowMoveEnded`;
//! docs/decisions/0029-edge-drag-tiling.md); the window service previews
//! and applies the tile with the same floating-frame actions as Window ▸
//! Move & Resize.

use super::*;

/// The niri events a patched niri sends during and at the end of a drag.
pub const MOVE_EDGE_EVENT: &str = "WindowMoveEdgeChanged";
pub const MOVE_ENDED_EVENT: &str = "WindowMoveEnded";

/// Where a window dropped at an edge goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeTile {
    /// The top edge: the whole working area, as Fill does.
    Fill,
    /// A side or a corner: that half or quarter.
    Region(domain::TileRegion),
}

impl EdgeTile {
    /// The niri edge name's tile. The bottom edge tiles nothing on the Mac
    /// (the Dock lives there), so it has none.
    pub fn from_edge(edge: &str) -> Option<Self> {
        use domain::TileRegion::*;
        Some(match edge {
            "top" => Self::Fill,
            "left" => Self::Region(Left),
            "right" => Self::Region(Right),
            "top-left" => Self::Region(TopLeft),
            "top-right" => Self::Region(TopRight),
            "bottom-left" => Self::Region(BottomLeft),
            "bottom-right" => Self::Region(BottomRight),
            _ => return None,
        })
    }

    /// `(x, y, width, height)` percentages of the working area.
    pub fn frame_percent(self) -> (f64, f64, f64, f64) {
        match self {
            Self::Fill => (0.0, 0.0, 100.0, 100.0),
            Self::Region(region) => region.frame_percent(),
        }
    }

    /// The action that puts `window` there.
    pub fn action(self, window: domain::WindowId) -> domain::Action {
        match self {
            Self::Fill => domain::Action::FillWindow { window },
            Self::Region(region) => domain::Action::TileWindow { window, region },
        }
    }
}

/// One step of a window drag at the screen's edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EdgeDrag {
    pub window: domain::WindowId,
    /// The tile under the pointer now, if any.
    pub tile: Option<EdgeTile>,
    /// The drag ended here: tile the window if `tile` is set.
    pub ended: bool,
}

/// The edge-drag step `event` reports, if it is one.
pub fn edge_drag(event: &domain::Event) -> Option<EdgeDrag> {
    let domain::Event::Unknown {
        source_kind,
        payload,
    } = event
    else {
        return None;
    };
    let ended = match source_kind.as_str() {
        MOVE_EDGE_EVENT => false,
        MOVE_ENDED_EVENT => true,
        _ => return None,
    };
    let window = domain::WindowId(payload.get("id")?.as_u64()?);
    let tile = match payload.get("edge")? {
        Value::Null => None,
        Value::String(edge) => EdgeTile::from_edge(edge),
        _ => return None,
    };
    Some(EdgeDrag {
        window,
        tile,
        ended,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(line: &str) -> domain::Event {
        let decoded = decode_event(line).unwrap();
        translate(
            decoded.event,
            decoded.source_kind,
            decoded.payload,
            &domain::State::default(),
        )
        .remove(0)
    }

    #[test]
    fn the_patched_niri_lines_decode_to_edge_drags() {
        // The exact lines 0003-ipc-report-window-move-edges.patch writes.
        assert_eq!(
            edge_drag(&decoded(
                r#"{"WindowMoveEdgeChanged":{"id":7,"edge":"left"}}"#
            )),
            Some(EdgeDrag {
                window: domain::WindowId(7),
                tile: Some(EdgeTile::Region(domain::TileRegion::Left)),
                ended: false,
            })
        );
        assert_eq!(
            edge_drag(&decoded(r#"{"WindowMoveEnded":{"id":7,"edge":"top"}}"#)),
            Some(EdgeDrag {
                window: domain::WindowId(7),
                tile: Some(EdgeTile::Fill),
                ended: true,
            })
        );
        assert_eq!(
            edge_drag(&decoded(
                r#"{"WindowMoveEdgeChanged":{"id":7,"edge":null}}"#
            ))
            .unwrap()
            .tile,
            None
        );
        // The bottom edge is the Dock's: no tile.
        assert_eq!(
            edge_drag(&decoded(r#"{"WindowMoveEnded":{"id":7,"edge":"bottom"}}"#))
                .unwrap()
                .tile,
            None
        );
        assert_eq!(
            edge_drag(&decoded(r#"{"WindowMinimizeRequested":{"id":7}}"#)),
            None
        );
    }

    #[test]
    fn corners_are_quarters_and_top_is_fill() {
        assert_eq!(
            EdgeTile::from_edge("bottom-right").unwrap().frame_percent(),
            (50.0, 50.0, 50.0, 50.0)
        );
        assert_eq!(
            EdgeTile::Fill.action(domain::WindowId(3)),
            domain::Action::FillWindow {
                window: domain::WindowId(3)
            }
        );
    }
}
