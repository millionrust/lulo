use std::fmt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceHealth {
    #[default]
    Starting,
    Healthy,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Health {
    pub compositor: SourceHealth,
    pub seats: SourceHealth,
}

#[derive(Clone, Default, PartialEq)]
pub struct Snapshot {
    pub(crate) compositor: Option<rmac_compositor::Snapshot>,
    pub(crate) seats: Option<rmac_shell_invocation::SeatInventory>,
    pub(crate) health: Health,
}

impl Snapshot {
    pub fn health(&self) -> Health {
        self.health
    }

    pub fn ready(&self) -> bool {
        self.health.compositor == SourceHealth::Healthy
            && self.health.seats == SourceHealth::Healthy
            && self.compositor.is_some()
            && self.seats.is_some()
    }

    pub fn compositor(&self) -> Option<&rmac_compositor::Snapshot> {
        self.compositor.as_ref()
    }

    pub fn seats(&self) -> Option<&rmac_shell_invocation::SeatInventory> {
        self.seats.as_ref()
    }

    pub fn global_shortcut(&self) -> Result<rmac_shell_invocation::Invocation, ResolveError> {
        let (compositor, seats) = self.live_sources()?;
        rmac_shell_invocation::global_shortcut(compositor, seats).map_err(ResolveError::Context)
    }

    pub fn surface_control(
        &self,
        output: &rmac_compositor::OutputId,
        seat: &rmac_shell_invocation::SeatId,
    ) -> Result<rmac_shell_invocation::Invocation, ResolveError> {
        let (compositor, seats) = self.live_sources()?;
        rmac_shell_invocation::surface_control(output, seat, compositor, seats)
            .map_err(ResolveError::Context)
    }

    fn live_sources(
        &self,
    ) -> Result<
        (
            &rmac_compositor::Snapshot,
            &rmac_shell_invocation::SeatInventory,
        ),
        ResolveError,
    > {
        if self.health.compositor != SourceHealth::Healthy {
            return Err(ResolveError::CompositorUnavailable);
        }
        if self.health.seats != SourceHealth::Healthy {
            return Err(ResolveError::SeatsUnavailable);
        }
        let compositor = self
            .compositor
            .as_ref()
            .ok_or(ResolveError::CompositorUnavailable)?;
        let seats = self.seats.as_ref().ok_or(ResolveError::SeatsUnavailable)?;
        Ok((compositor, seats))
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("health", &self.health)
            .field(
                "outputs",
                &self
                    .compositor
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.outputs.len()),
            )
            .field(
                "windows",
                &self
                    .compositor
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.windows.len()),
            )
            .field(
                "seats",
                &self.seats.as_ref().map_or(0, |inventory| inventory.len()),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveError {
    CompositorUnavailable,
    SeatsUnavailable,
    Context(rmac_shell_invocation::ResolveError),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CompositorUnavailable => {
                formatter.write_str("live compositor invocation state is unavailable")
            }
            Self::SeatsUnavailable => {
                formatter.write_str("live Wayland seat invocation state is unavailable")
            }
            Self::Context(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ResolveError {}
