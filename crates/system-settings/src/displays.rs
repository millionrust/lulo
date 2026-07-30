#[derive(Clone)]
pub(super) enum DisplayChange {
    Mode {
        output: rmac_display::Output,
        mode: rmac_display::Mode,
    },
    Scale {
        output: rmac_display::Output,
        scale: f64,
    },
    Transform {
        output: rmac_display::Output,
        transform: rmac_display::Transform,
    },
    Position {
        output: rmac_display::Output,
        x: i32,
        y: i32,
    },
}

impl DisplayChange {
    pub(super) fn apply(
        &self,
    ) -> std::result::Result<rmac_display::AppliedChange, rmac_display::Error> {
        match self {
            Self::Mode { output, mode } => rmac_display::set_mode(output, *mode),
            Self::Scale { output, scale } => rmac_display::set_scale(output, *scale),
            Self::Transform { output, transform } => rmac_display::set_transform(output, transform),
            Self::Position { output, x, y } => rmac_display::set_position(output, *x, *y),
        }
    }
}

#[derive(Clone)]
pub(super) struct DisplayConfirmation {
    pub(super) baseline: rmac_display::Snapshot,
    pub(super) applied: rmac_display::Snapshot,
    pub(super) generation: u64,
    pub(super) seconds_remaining: u8,
}

pub(super) const DISPLAY_CONFIRMATION_SECONDS: u8 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DisplayPlacement {
    Left,
    Right,
    Above,
    Below,
}

impl DisplayPlacement {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Left => "Left of Main",
            Self::Right => "Right of Main",
            Self::Above => "Above Main",
            Self::Below => "Below Main",
        }
    }
}

pub(super) fn relative_display_position(
    moving: &rmac_display::LogicalOutput,
    anchor: &rmac_display::LogicalOutput,
    placement: DisplayPlacement,
) -> Option<(i32, i32)> {
    match placement {
        DisplayPlacement::Left => Some((
            anchor.x.checked_sub(i32::try_from(moving.width).ok()?)?,
            anchor.y,
        )),
        DisplayPlacement::Right => Some((
            anchor.x.checked_add(i32::try_from(anchor.width).ok()?)?,
            anchor.y,
        )),
        DisplayPlacement::Above => Some((
            anchor.x,
            anchor.y.checked_sub(i32::try_from(moving.height).ok()?)?,
        )),
        DisplayPlacement::Below => Some((
            anchor.x,
            anchor.y.checked_add(i32::try_from(anchor.height).ok()?)?,
        )),
    }
}

pub(super) fn compositor_event_affects_displays(event: &rmac_compositor::Event) -> bool {
    matches!(
        event,
        rmac_compositor::Event::Snapshot { .. }
            | rmac_compositor::Event::OutputsReplaced { .. }
            | rmac_compositor::Event::WorkspacesReplaced { .. }
    ) || matches!(
        event,
        rmac_compositor::Event::Unknown { source_kind, .. } if source_kind == "ConfigLoaded"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrangement_places_edges_without_overlap() {
        let moving = rmac_display::LogicalOutput {
            x: 0,
            y: 0,
            width: 1440,
            height: 900,
            scale: 2.0,
            transform: rmac_display::Transform::Normal,
        };
        let anchor = rmac_display::LogicalOutput {
            x: 100,
            y: 200,
            width: 1920,
            height: 1080,
            scale: 1.0,
            transform: rmac_display::Transform::Normal,
        };

        assert_eq!(
            relative_display_position(&moving, &anchor, DisplayPlacement::Left),
            Some((-1340, 200))
        );
        assert_eq!(
            relative_display_position(&moving, &anchor, DisplayPlacement::Right),
            Some((2020, 200))
        );
        assert_eq!(
            relative_display_position(&moving, &anchor, DisplayPlacement::Above),
            Some((100, -700))
        );
        assert_eq!(
            relative_display_position(&moving, &anchor, DisplayPlacement::Below),
            Some((100, 1280))
        );
    }

    #[test]
    fn refresh_hints_ignore_unrelated_compositor_churn() {
        assert!(compositor_event_affects_displays(
            &rmac_compositor::Event::OutputsReplaced {
                outputs: Vec::new(),
            }
        ));
        assert!(compositor_event_affects_displays(
            &rmac_compositor::Event::Unknown {
                source_kind: "ConfigLoaded".into(),
                payload: Default::default(),
            }
        ));
        assert!(!compositor_event_affects_displays(
            &rmac_compositor::Event::WindowsReplaced {
                windows: Vec::new(),
            }
        ));
    }
}
