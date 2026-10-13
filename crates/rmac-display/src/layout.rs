//! Platform-neutral layout checks shared by the niri and Windows backends.

use super::*;

pub(super) fn same_enabled_output_identities(expected: &Snapshot, current: &Snapshot) -> bool {
    let expected = expected
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    let current = current
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    expected.len() == current.len()
        && expected.iter().all(|expected| {
            current.iter().any(|current| {
                current.id == expected.id
                    && current.connector == expected.connector
                    && current.serial == expected.serial
                    && current.name == expected.name
            })
        })
}

pub(super) fn same_complete_layout(expected: &Snapshot, current: &Snapshot) -> bool {
    if !same_enabled_output_identities(expected, current) {
        return false;
    }
    let expected_primary = expected
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .map(|output| output.id.as_str());
    let current_primary = current
        .outputs
        .iter()
        .find(|output| output.primary && output.logical.is_some())
        .map(|output| output.id.as_str());
    if expected_primary != current_primary {
        return false;
    }
    let Some(primary) = expected_primary.or_else(|| {
        expected
            .outputs
            .iter()
            .find(|output| output.logical.is_some())
            .map(|output| output.id.as_str())
    }) else {
        return false;
    };
    match (
        current_layout(expected, primary),
        current_layout(current, primary),
    ) {
        (Ok(expected), Ok(current)) => expected == current,
        _ => false,
    }
}

pub(super) fn validate_layout(layout: &Layout, snapshot: &Snapshot) -> Result<(), Error> {
    let enabled = snapshot
        .outputs
        .iter()
        .filter(|output| output.logical.is_some() && output.current_mode().is_some())
        .collect::<Vec<_>>();
    if layout.outputs.is_empty()
        || layout.outputs.len() > MAX_OUTPUTS
        || layout.outputs.len() != enabled.len()
    {
        return Err(Error::new(
            "validate the display layout",
            "the layout must contain every enabled display exactly once",
        ));
    }
    let mut identities = HashSet::new();
    for configured in &layout.outputs {
        validate_output_id(&configured.id)?;
        if configured.id.len() > MAX_OUTPUT_ID_BYTES || !identities.insert(&configured.id) {
            return Err(Error::new(
                "validate the display layout",
                "display identities must be bounded and unique",
            ));
        }
        if !configured.scale.is_finite()
            || !(0.5..=4.0).contains(&configured.scale)
            || !configured.transform.is_configurable()
            || configured.logical_width == 0
            || configured.logical_height == 0
            || configured.x.unsigned_abs() > MAX_LAYOUT_COORDINATE as u32
            || configured.y.unsigned_abs() > MAX_LAYOUT_COORDINATE as u32
        {
            return Err(Error::new(
                "validate the display layout",
                "display scale, transform, size, or position is outside the safe range",
            ));
        }
        let current = enabled
            .iter()
            .find(|output| output.id == configured.id)
            .ok_or_else(|| {
                Error::new(
                    "validate the display layout",
                    "a configured display is no longer enabled",
                )
            })?;
        if !current.modes.contains(&configured.mode) {
            return Err(Error::new(
                "validate the display layout",
                "a configured mode is no longer advertised by its display",
            ));
        }
        let logical = current.logical.as_ref().expect("enabled output checked");
        if current.current_mode() != Some(configured.mode)
            || (logical.scale - configured.scale).abs() > 0.001
            || logical.transform != configured.transform
            || logical.x != configured.x
            || logical.y != configured.y
            || logical.width != configured.logical_width
            || logical.height != configured.logical_height
        {
            return Err(Error::new(
                "validate the display layout",
                "the compositor state changed before the layout could be saved",
            ));
        }
    }
    if !identities.contains(&layout.primary) {
        return Err(Error::new(
            "validate the display layout",
            "the main display must be one of the enabled displays",
        ));
    }
    for (index, left) in layout.outputs.iter().enumerate() {
        for right in layout.outputs.iter().skip(index + 1) {
            if rectangles_overlap(left, right) {
                return Err(Error::new(
                    "validate the display layout",
                    "display rectangles must not overlap",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn rectangles_overlap(left: &OutputConfiguration, right: &OutputConfiguration) -> bool {
    let left_x2 = i64::from(left.x) + i64::from(left.logical_width);
    let left_y2 = i64::from(left.y) + i64::from(left.logical_height);
    let right_x2 = i64::from(right.x) + i64::from(right.logical_width);
    let right_y2 = i64::from(right.y) + i64::from(right.logical_height);
    i64::from(left.x) < right_x2
        && i64::from(right.x) < left_x2
        && i64::from(left.y) < right_y2
        && i64::from(right.y) < left_y2
}
