//! Per-output Dock surface projection.

use super::*;

pub const SHELF_PADDING: f32 = 8.0;
pub const HIDDEN_EDGE_THICKNESS: f32 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDescription {
    pub output: rmac_compositor::OutputId,
    pub placement: rmac_shell_settings::DockPlacement,
    /// Length along the Dock's item axis in logical pixels.
    pub output_axis_length: f64,
    pub output_scale: f64,
    pub base_thickness: f32,
    pub maximum_thickness: f32,
    /// A stable work-area reservation. It never grows during magnification.
    pub exclusive_zone: f32,
    pub reveal_edge_thickness: f32,
    pub keyboard_interactive: bool,
    pub autohide: bool,
    pub overview_visible: bool,
    pub magnification_enabled: bool,
    pub animate: bool,
    pub magnification: motion::MagnificationConfig,
}

pub fn surface_descriptions(
    compositor: &rmac_compositor::Snapshot,
    settings: &rmac_shell_settings::DockSettings,
    primary: Option<&rmac_compositor::OutputId>,
    reduced_motion: bool,
) -> Result<Vec<SurfaceDescription>, motion::ConfigError> {
    let magnification = motion::MagnificationConfig {
        maximum_scale: settings.magnification_scale,
        ..Default::default()
    }
    .validate()?;
    let selected: BTreeSet<_> = surface_outputs(compositor, &settings.outputs, primary)
        .into_iter()
        .collect();
    let magnification_enabled = settings.magnification && !reduced_motion;
    let base_thickness = magnification.icon_size + 2.0 * SHELF_PADDING;
    let maximum_thickness = if magnification_enabled {
        magnification.icon_size * magnification.maximum_scale + 2.0 * SHELF_PADDING
    } else {
        base_thickness
    };
    let exclusive_zone = reserved_thickness(settings);
    let reveal_edge_thickness = if settings.autohide {
        HIDDEN_EDGE_THICKNESS
    } else {
        0.0
    };
    let mut surfaces = compositor
        .outputs
        .iter()
        .filter(|output| selected.contains(&output.id))
        .filter_map(|output| {
            let logical = renderable_logical_output(output)?;
            let output_axis_length = match settings.placement {
                rmac_shell_settings::DockPlacement::Bottom => logical.size.width,
                rmac_shell_settings::DockPlacement::Left
                | rmac_shell_settings::DockPlacement::Right => logical.size.height,
            };
            Some(SurfaceDescription {
                output: output.id.clone(),
                placement: settings.placement,
                output_axis_length,
                output_scale: logical.scale,
                base_thickness,
                maximum_thickness,
                exclusive_zone,
                reveal_edge_thickness,
                keyboard_interactive: false,
                autohide: settings.autohide,
                overview_visible: compositor.overview_visible,
                magnification_enabled,
                animate: !reduced_motion,
                magnification,
            })
        })
        .collect::<Vec<_>>();
    surfaces.sort_by(|left, right| left.output.cmp(&right.output));
    Ok(surfaces)
}

/// The Dock's resting tile edge, clamped to the Size slider's range.
fn resting_tile_size(tile_size: f32) -> f32 {
    if tile_size.is_finite() {
        tile_size.clamp(
            rmac_shell_settings::MIN_DOCK_TILE_SIZE,
            rmac_shell_settings::MAX_DOCK_TILE_SIZE,
        )
    } else {
        rmac_shell_settings::DEFAULT_DOCK_TILE_SIZE
    }
}

/// The work area the Dock reserves at its edge: its shelf (the tile plus
/// 0.15625 of it on each side) and the shelf's margin to the screen edge
/// (0.078125 of the tile), 89 pt at the default 64 pt tiles — the Dock
/// surface's own `TileMetrics::exclusive_zone`. Nothing when the Dock hides
/// (macOS gives the space back to windows then) or "Reserve screen space"
/// is off.
pub fn reserved_thickness(settings: &rmac_shell_settings::DockSettings) -> f32 {
    if !settings.reserve_space || settings.autohide {
        return 0.0;
    }
    resting_tile_size(settings.tile_size) * (1.0 + 2.0 * 0.15625 + 0.078125)
}

/// The visible-frame reservation (`rmac_compositor::placement`) for these
/// Dock settings below a `menu_bar` tall menu bar.
pub fn work_area_reservation(
    settings: &rmac_shell_settings::DockSettings,
    menu_bar: f64,
) -> rmac_compositor::placement::Reservation {
    use rmac_compositor::placement::{DockEdge, DockOutputs, Reservation};
    Reservation {
        menu_bar,
        dock_edge: match settings.placement {
            rmac_shell_settings::DockPlacement::Bottom => DockEdge::Bottom,
            rmac_shell_settings::DockPlacement::Left => DockEdge::Left,
            rmac_shell_settings::DockPlacement::Right => DockEdge::Right,
        },
        dock: f64::from(reserved_thickness(settings)),
        dock_outputs: match &settings.outputs {
            rmac_shell_settings::OutputScope::All => DockOutputs::All,
            rmac_shell_settings::OutputScope::Primary => DockOutputs::Primary,
            rmac_shell_settings::OutputScope::Named(name) => {
                DockOutputs::Named(rmac_compositor::OutputId::from(name.as_str()))
            }
        },
    }
}

fn renderable_logical_output(
    output: &rmac_compositor::Output,
) -> Option<&rmac_compositor::LogicalOutput> {
    if !output.enabled() {
        return None;
    }
    let logical = output.logical.as_ref()?;
    (logical.size.is_valid()
        && logical.size.width > 0.0
        && logical.size.height > 0.0
        && logical.scale.is_finite()
        && logical.scale > 0.0)
        .then_some(logical)
}

pub fn surface_outputs(
    compositor: &rmac_compositor::Snapshot,
    scope: &rmac_shell_settings::OutputScope,
    primary: Option<&rmac_compositor::OutputId>,
) -> Vec<rmac_compositor::OutputId> {
    let enabled: BTreeSet<_> = compositor
        .outputs
        .iter()
        .filter(|output| renderable_logical_output(output).is_some())
        .map(|output| output.id.clone())
        .collect();
    match scope {
        rmac_shell_settings::OutputScope::All => enabled.into_iter().collect(),
        rmac_shell_settings::OutputScope::Primary => primary
            .filter(|output| enabled.contains(*output))
            .cloned()
            .into_iter()
            .collect(),
        rmac_shell_settings::OutputScope::Named(name) => {
            let output = rmac_compositor::OutputId::from(name.as_str());
            enabled
                .contains(&output)
                .then_some(output)
                .into_iter()
                .collect()
        }
    }
}
