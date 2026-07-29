//! Process bridge from one shortcut endpoint to one coherent shell invocation.

use std::fmt;

#[derive(Clone)]
pub struct Context {
    invocation: rmac_shell_invocation::Invocation,
    compositor: rmac_compositor::Snapshot,
}

impl Context {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationError {
    NotActivated,
    Resolve(rmac_shell_invocation_runtime::ResolveError),
}

impl fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotActivated => formatter.write_str("the shortcut event is not an activation"),
            Self::Resolve(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ActivationError {}

#[derive(Clone)]
pub struct Activation {
    event: rmac_shortcuts::Event,
    context: Result<Context, ActivationError>,
}

impl Activation {
    pub fn event(&self) -> &rmac_shortcuts::Event {
        &self.event
    }

    pub fn context(&self) -> Result<&Context, ActivationError> {
        self.context.as_ref().map_err(|error| *error)
    }

    pub fn into_parts(self) -> (rmac_shortcuts::Event, Result<Context, ActivationError>) {
        (self.event, self.context)
    }
}

impl fmt::Debug for Activation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Activation")
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub enum Update {
    Ready,
    Activated(Box<Activation>),
}

#[derive(Default)]
pub struct Coordinator {
    endpoint_ready: bool,
    readiness_announced: bool,
    runtime: rmac_shell_invocation_runtime::Snapshot,
}

impl Coordinator {
    pub fn endpoint_ready(&mut self) -> bool {
        self.endpoint_ready = true;
        self.take_readiness()
    }

    pub fn apply_runtime(&mut self, snapshot: rmac_shell_invocation_runtime::Snapshot) -> bool {
        self.runtime = snapshot;
        self.take_readiness()
    }

    pub fn activate(&self, event: rmac_shortcuts::Event) -> Activation {
        let context = if matches!(event, rmac_shortcuts::Event::Activated { .. }) {
            self.runtime
                .global_shortcut()
                .map(|invocation| Context {
                    invocation,
                    compositor: self
                        .runtime
                        .compositor()
                        .expect("successful resolution requires a compositor snapshot")
                        .clone(),
                })
                .map_err(ActivationError::Resolve)
        } else {
            Err(ActivationError::NotActivated)
        };
        Activation { event, context }
    }

    fn take_readiness(&mut self) -> bool {
        if self.readiness_announced || !self.endpoint_ready || !self.runtime.ready() {
            return false;
        }
        self.readiness_announced = true;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchShortcut,
    WatchInvocation,
    Consume,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    operation: Operation,
    detail: String,
}

impl Error {
    #[cfg(target_os = "linux")]
    fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("operation", &self.operation)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not maintain the shell activation endpoint")
    }
}

impl std::error::Error for Error {}

/// Own one shortcut endpoint and publish activations paired with one immutable
/// live compositor/seat snapshot. Readiness is emitted exactly once after both
/// the endpoint and sources are usable.
#[cfg(target_os = "linux")]
pub async fn watch(
    shortcut: rmac_shortcuts::ShortcutId,
    sender: async_channel::Sender<Update>,
) -> Result<(), Error> {
    use futures_util::FutureExt as _;

    let (shortcut_tx, shortcut_rx) = async_channel::bounded(8);
    let (endpoint_tx, endpoint_rx) = async_channel::bounded(1);
    let (runtime_tx, runtime_rx) = async_channel::bounded(8);
    let shortcut_source = async {
        rmac_shortcuts::watch_dispatches_ready(shortcut, shortcut_tx, endpoint_tx)
            .await
            .map_err(|error| Error::new(Operation::WatchShortcut, error.to_string()))
    };
    let runtime_source = async {
        rmac_shell_invocation_runtime::watch(runtime_tx)
            .await
            .map_err(|error| Error::new(Operation::WatchInvocation, error.detail()))
    };
    let sources = async {
        let (_, _) = futures_util::try_join!(shortcut_source, runtime_source)?;
        Ok::<(), Error>(())
    }
    .fuse();
    let consumer = consume(sender, shortcut_rx, endpoint_rx, runtime_rx).fuse();
    futures_util::pin_mut!(sources, consumer);
    futures_util::select! {
        result = sources => result,
        result = consumer => result,
    }
}

#[cfg(target_os = "linux")]
async fn consume(
    sender: async_channel::Sender<Update>,
    shortcuts: async_channel::Receiver<rmac_shortcuts::Event>,
    endpoint: async_channel::Receiver<()>,
    runtime: async_channel::Receiver<rmac_shell_invocation_runtime::Snapshot>,
) -> Result<(), Error> {
    use futures_util::FutureExt as _;

    let mut coordinator = Coordinator::default();
    loop {
        let shortcut = shortcuts.recv().fuse();
        let endpoint = endpoint.recv().fuse();
        let runtime = runtime.recv().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(shortcut, endpoint, runtime, closed);
        let update = futures_util::select! {
            event = shortcut => Some(Update::Activated(Box::new(coordinator.activate(
                event.map_err(|_| {
                    Error::new(Operation::WatchShortcut, "the shortcut endpoint stopped")
                })?,
            )))),
            ready = endpoint => {
                ready.map_err(|_| Error::new(
                    Operation::WatchShortcut,
                    "the shortcut endpoint stopped before readiness",
                ))?;
                coordinator.endpoint_ready().then_some(Update::Ready)
            },
            snapshot = runtime => {
                let snapshot = snapshot.map_err(|_| Error::new(
                    Operation::WatchInvocation,
                    "the invocation source stopped",
                ))?;
                coordinator.apply_runtime(snapshot).then_some(Update::Ready)
            },
            _ = closed => return Ok(()),
        };
        if let Some(update) = update {
            sender
                .send(update)
                .await
                .map_err(|_| Error::new(Operation::Consume, "the activation consumer stopped"))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "private".into(),
            model: "private".into(),
            serial: Some("private".into()),
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(rmac_compositor::LogicalOutput {
                position: Default::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    fn runtime(seats: &[&str]) -> rmac_shell_invocation_runtime::Snapshot {
        let mut coordinator = rmac_shell_invocation_runtime::Coordinator::default();
        coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
            snapshot: rmac_compositor::Snapshot {
                outputs: vec![output("private-output")],
                focus: rmac_compositor::FocusState {
                    output: Some("private-output".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        });
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Connected,
        });
        coordinator.apply_seats(rmac_shell_invocation_runtime::SeatEvent::Snapshot(
            rmac_shell_invocation::SeatInventory::new(seats.iter().map(|seat| (*seat).to_owned()))
                .unwrap(),
        ));
        coordinator.snapshot()
    }

    fn activated() -> rmac_shortcuts::Event {
        rmac_shortcuts::Event::Activated {
            id: rmac_shortcuts::ShortcutId("launcher".into()),
            timestamp_ms: 1,
        }
    }

    #[test]
    fn readiness_requires_endpoint_and_both_live_sources_once() {
        let mut coordinator = Coordinator::default();
        assert!(!coordinator.endpoint_ready());
        assert!(coordinator.apply_runtime(runtime(&["private-seat"])));
        assert!(!coordinator.apply_runtime(runtime(&["private-seat"])));
    }

    #[test]
    fn activation_carries_one_exact_private_context() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_runtime(runtime(&["private-seat"]));
        let activation = coordinator.activate(activated());
        let context = activation.context().unwrap();
        assert_eq!(context.invocation().output().0, "private-output");
        let diagnostics = format!("{activation:?}");
        assert!(!diagnostics.contains("private-output"));
        assert!(!diagnostics.contains("private-seat"));
    }

    #[test]
    fn exact_output_geometry_drives_top_right_and_centered_bounds() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_runtime(runtime(&["private-seat"]));
        let activation = coordinator.activate(activated());
        let context = activation.context().unwrap();
        assert_eq!(
            context.top_right_bounds(380.0, 548.0, 44.0, 12.0),
            Ok(LogicalBounds {
                x: 1528.0,
                y: 44.0,
                width: 380.0,
                height: 548.0,
            })
        );
        assert_eq!(
            context.centered_bounds(720.0, 540.0),
            Ok(LogicalBounds {
                x: 600.0,
                y: 270.0,
                width: 720.0,
                height: 540.0,
            })
        );
    }

    #[test]
    fn degraded_and_multi_seat_activations_fail_explicitly() {
        let coordinator = Coordinator::default();
        assert!(matches!(
            coordinator.activate(activated()).context(),
            Err(ActivationError::Resolve(
                rmac_shell_invocation_runtime::ResolveError::CompositorUnavailable
            ))
        ));

        let mut coordinator = Coordinator::default();
        coordinator.apply_runtime(runtime(&["seat-a", "seat-b"]));
        assert!(matches!(
            coordinator.activate(activated()).context(),
            Err(ActivationError::Resolve(
                rmac_shell_invocation_runtime::ResolveError::Context(
                    rmac_shell_invocation::ResolveError::AmbiguousSeat
                )
            ))
        ));
    }
}
