use std::fmt;

#[derive(Clone)]
pub struct Context {
    invocation: rmac_shell_invocation::Invocation,
    compositor: rmac_compositor::Snapshot,
}

impl Context {
    pub(crate) fn new(
        invocation: rmac_shell_invocation::Invocation,
        compositor: rmac_compositor::Snapshot,
    ) -> Self {
        Self {
            invocation,
            compositor,
        }
    }

    pub fn invocation(&self) -> &rmac_shell_invocation::Invocation {
        &self.invocation
    }

    pub fn compositor(&self) -> &rmac_compositor::Snapshot {
        &self.compositor
    }

    pub fn top_right_bounds(
        &self,
        width: f64,
        height: f64,
        top_margin: f64,
        right_margin: f64,
    ) -> Result<LogicalBounds, PlacementError> {
        let logical = self.logical_output()?;
        validate_extent(width, height)?;
        validate_margin(top_margin)?;
        validate_margin(right_margin)?;
        if logical.size.width < width + right_margin || logical.size.height < height + top_margin {
            return Err(PlacementError::DoesNotFit);
        }
        LogicalBounds::new(
            logical.position.x + logical.size.width - width - right_margin,
            logical.position.y + top_margin,
            width,
            height,
        )
    }

    pub fn centered_bounds(
        &self,
        width: f64,
        height: f64,
    ) -> Result<LogicalBounds, PlacementError> {
        let logical = self.logical_output()?;
        validate_extent(width, height)?;
        if logical.size.width < width || logical.size.height < height {
            return Err(PlacementError::DoesNotFit);
        }
        LogicalBounds::new(
            logical.position.x + (logical.size.width - width) / 2.0,
            logical.position.y + (logical.size.height - height) / 2.0,
            width,
            height,
        )
    }

    fn logical_output(&self) -> Result<&rmac_compositor::LogicalOutput, PlacementError> {
        self.compositor
            .outputs
            .iter()
            .find(|output| &output.id == self.invocation.output() && output.enabled())
            .and_then(|output| output.logical.as_ref())
            .ok_or(PlacementError::OutputUnavailable)
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("invocation", &self.invocation)
            .field("outputs", &self.compositor.outputs.len())
            .field("windows", &self.compositor.windows.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LogicalBounds {
    fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, PlacementError> {
        Ok(Self {
            x: finite_f32(x)?,
            y: finite_f32(y)?,
            width: finite_f32(width)?,
            height: finite_f32(height)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlacementError {
    OutputUnavailable,
    InvalidGeometry,
    DoesNotFit,
    NotRepresentable,
}

impl fmt::Display for PlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutputUnavailable => "the invoking output is unavailable",
            Self::InvalidGeometry => "the surface geometry is invalid",
            Self::DoesNotFit => "the surface does not fit on the invoking output",
            Self::NotRepresentable => "the surface geometry cannot be represented by the host",
        })
    }
}

impl std::error::Error for PlacementError {}

fn validate_extent(width: f64, height: f64) -> Result<(), PlacementError> {
    if width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0 {
        Ok(())
    } else {
        Err(PlacementError::InvalidGeometry)
    }
}

fn validate_margin(margin: f64) -> Result<(), PlacementError> {
    if margin.is_finite() && margin >= 0.0 {
        Ok(())
    } else {
        Err(PlacementError::InvalidGeometry)
    }
}

fn finite_f32(value: f64) -> Result<f32, PlacementError> {
    if value.is_finite() && value >= f32::MIN as f64 && value <= f32::MAX as f64 {
        Ok(value as f32)
    } else {
        Err(PlacementError::NotRepresentable)
    }
}
