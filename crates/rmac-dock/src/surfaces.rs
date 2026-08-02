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
    let exclusive_zone = if settings.reserve_space {
        base_thickness
    } else {
        0.0
    };
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
