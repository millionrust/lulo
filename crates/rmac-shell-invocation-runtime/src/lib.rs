//! Coherent live compositor and Wayland-seat authority for shell invocations.

use std::fmt;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use futures_util::FutureExt as _;

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
    compositor: Option<rmac_compositor::Snapshot>,
    seats: Option<rmac_shell_invocation::SeatInventory>,
    health: Health,
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

#[derive(Default)]
pub struct Coordinator {
    compositor: rmac_compositor::State,
    compositor_snapshot: Option<rmac_compositor::Snapshot>,
    seats: Option<rmac_shell_invocation::SeatInventory>,
    health: Health,
}

impl Coordinator {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            compositor: self.compositor_snapshot.clone(),
            seats: self.seats.clone(),
            health: self.health,
        }
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        let establishes_snapshot = matches!(event, rmac_compositor::Event::Snapshot { .. });
        match &event {
            rmac_compositor::Event::ConnectionChanged { state } => {
                self.health.compositor = match state {
                    rmac_compositor::ConnectionState::Connecting
                        if self.compositor_snapshot.is_none() =>
                    {
                        SourceHealth::Starting
                    }
                    rmac_compositor::ConnectionState::Connected => SourceHealth::Healthy,
                    rmac_compositor::ConnectionState::Connecting
                    | rmac_compositor::ConnectionState::Disconnected
                    | rmac_compositor::ConnectionState::Reconnecting => SourceHealth::Unavailable,
                };
            }
            rmac_compositor::Event::Snapshot { .. } => {}
            _ if self.compositor_snapshot.is_none() => return false,
            _ => {}
        }
        self.compositor.apply(event);
        if establishes_snapshot || self.compositor_snapshot.is_some() {
            self.compositor_snapshot = Some(self.compositor.snapshot());
        }
        if self.compositor.connection == rmac_compositor::ConnectionState::Connected {
            self.health.compositor = if self.compositor_snapshot.is_some() {
                SourceHealth::Healthy
            } else {
                SourceHealth::Starting
            };
        }
        before != self.snapshot()
    }

    pub fn apply_seats(&mut self, event: SeatEvent) -> bool {
        let before = self.snapshot();
        match event {
            SeatEvent::Connecting if self.seats.is_none() => {
                self.health.seats = SourceHealth::Starting;
            }
            SeatEvent::Connecting | SeatEvent::Unavailable => {
                self.health.seats = SourceHealth::Unavailable;
            }
            SeatEvent::Snapshot(seats) => {
                self.seats = Some(seats);
                self.health.seats = SourceHealth::Healthy;
            }
        }
        before != self.snapshot()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SeatEvent {
    Connecting,
    Snapshot(rmac_shell_invocation::SeatInventory),
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchCompositor,
    WatchSeats,
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
        formatter.write_str("Could not maintain shell invocation state")
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<Snapshot>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (seat_tx, seat_rx) = async_channel::bounded(4);
    let compositor = watch_compositor(compositor_tx);
    let seats = watch_seats(seat_tx);
    let consumer = consume(sender, compositor_rx, seat_rx);
    let (_, _, _) = futures_util::try_join!(compositor, seats, consumer)?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn watch_compositor(
    sender: async_channel::Sender<rmac_compositor::Event>,
) -> Result<(), Error> {
    loop {
        let watcher = rmac_compositor_niri::watch(sender.clone()).fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(watcher, closed);
        futures_util::select! {
            result = watcher => match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let _ = error;
                    if sender.send(rmac_compositor::Event::ConnectionChanged {
                        state: rmac_compositor::ConnectionState::Disconnected,
                    }).await.is_err() {
                        return Ok(());
                    }
                }
            },
            _ = closed => return Ok(()),
        }
        wait_or_closed(&sender, Duration::from_secs(1)).await;
        if sender.is_closed() {
            return Ok(());
        }
    }
}

#[cfg(target_os = "linux")]
async fn watch_seats(sender: async_channel::Sender<SeatEvent>) -> Result<(), Error> {
    loop {
        if sender.send(SeatEvent::Connecting).await.is_err() {
            return Ok(());
        }
        let (updates_tx, updates_rx) = async_channel::bounded(4);
        let watcher = rmac_shell_invocation::wayland::watch(updates_tx).fuse();
        futures_util::pin_mut!(watcher);
        loop {
            let update = updates_rx.recv().fuse();
            let closed = sender.closed().fuse();
            futures_util::pin_mut!(update, closed);
            futures_util::select! {
                result = watcher => {
                    match result {
                        Ok(()) if sender.is_closed() => return Ok(()),
                        Ok(()) | Err(_) => {
                            if sender.send(SeatEvent::Unavailable).await.is_err() {
                                return Ok(());
                            }
                            break;
                        }
                    }
                },
                update = update => match update {
                    Ok(snapshot) => {
                        if sender.send(SeatEvent::Snapshot(snapshot)).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(_) => {
                        if sender.send(SeatEvent::Unavailable).await.is_err() {
                            return Ok(());
                        }
                        break;
                    }
                },
                _ = closed => return Ok(()),
            }
        }
        wait_or_closed(&sender, Duration::from_secs(1)).await;
        if sender.is_closed() {
            return Ok(());
        }
    }
}

#[cfg(target_os = "linux")]
async fn wait_or_closed<T>(sender: &async_channel::Sender<T>, duration: Duration) {
    let timer = async_io::Timer::after(duration).fuse();
    let closed = sender.closed().fuse();
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

#[cfg(target_os = "linux")]
async fn consume(
    sender: async_channel::Sender<Snapshot>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    seats: async_channel::Receiver<SeatEvent>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published: Option<Snapshot> = None;
    loop {
        let compositor_event = compositor.recv().fuse();
        let seat_event = seats.recv().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(compositor_event, seat_event, closed);
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new(
                    Operation::WatchCompositor,
                    "the compositor source stopped",
                ))?;
                coordinator.apply_compositor(event);
            },
            event = seat_event => {
                let event = event.map_err(|_| Error::new(
                    Operation::WatchSeats,
                    "the Wayland seat source stopped",
                ))?;
                coordinator.apply_seats(event);
            },
            _ = closed => return Ok(()),
        }
        let next = coordinator.snapshot();
        if published.as_ref() == Some(&next) {
            continue;
        }
        if sender.send(next.clone()).await.is_err() {
            return Ok(());
        }
        published = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "private make".into(),
            model: "private model".into(),
            serial: Some("private serial".into()),
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    fn window(id: u64, focused: bool) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: rmac_compositor::WindowId(id),
            title: Some("private window title".into()),
            app_id: Some("private.app".into()),
            pid: None,
            workspace: None,
            focused,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: rmac_compositor::WindowLayout::default(),
        }
    }

    fn compositor(output_id: &str, window_id: u64) -> rmac_compositor::Snapshot {
        rmac_compositor::Snapshot {
            outputs: vec![output(output_id)],
            windows: vec![window(window_id, true)],
            focus: rmac_compositor::FocusState {
                output: Some(output_id.into()),
                window: Some(rmac_compositor::WindowId(window_id)),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn connect(coordinator: &mut Coordinator, output_id: &str, seat_ids: &[&str]) -> Snapshot {
        coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
            snapshot: compositor(output_id, 9),
        });
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Connected,
        });
        coordinator.apply_seats(SeatEvent::Snapshot(
            rmac_shell_invocation::SeatInventory::new(
                seat_ids.iter().map(|seat| (*seat).to_string()),
            )
            .unwrap(),
        ));
        coordinator.snapshot()
    }

    #[test]
    fn resolution_waits_for_both_live_sources() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
            snapshot: compositor("private-output", 9),
        });
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Connected,
        });
        assert_eq!(
            coordinator.snapshot().global_shortcut(),
            Err(ResolveError::SeatsUnavailable)
        );
        let ready = connect(&mut coordinator, "private-output", &["private-seat"]);
        assert!(ready.ready());
        assert_eq!(
            ready.global_shortcut().unwrap().restore_window(),
            Some(rmac_compositor::WindowId(9))
        );
    }

    #[test]
    fn source_loss_retains_last_known_good_but_fails_new_invocations() {
        let mut coordinator = Coordinator::default();
        let ready = connect(
            &mut coordinator,
            "old-private-output",
            &["old-private-seat"],
        );
        assert!(ready.global_shortcut().is_ok());

        coordinator.apply_seats(SeatEvent::Unavailable);
        let degraded = coordinator.snapshot();
        assert!(degraded.compositor().is_some());
        assert!(degraded.seats().is_some());
        assert_eq!(
            degraded.global_shortcut(),
            Err(ResolveError::SeatsUnavailable)
        );

        coordinator.apply_seats(SeatEvent::Snapshot(
            rmac_shell_invocation::SeatInventory::new(vec!["new-private-seat".into()]).unwrap(),
        ));
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Reconnecting,
        });
        assert_eq!(
            coordinator.snapshot().global_shortcut(),
            Err(ResolveError::CompositorUnavailable)
        );

        let recovered = connect(
            &mut coordinator,
            "new-private-output",
            &["new-private-seat"],
        );
        assert_eq!(
            recovered.global_shortcut().unwrap().output().0,
            "new-private-output"
        );
    }

    #[test]
    fn multi_seat_shortcuts_fail_while_exact_pointer_context_resolves() {
        let mut coordinator = Coordinator::default();
        let snapshot = connect(
            &mut coordinator,
            "private-output",
            &["private-seat-a", "private-seat-b"],
        );
        assert_eq!(
            snapshot.global_shortcut(),
            Err(ResolveError::Context(
                rmac_shell_invocation::ResolveError::AmbiguousSeat
            ))
        );
        let seat = rmac_shell_invocation::SeatId::new("private-seat-b").unwrap();
        assert_eq!(
            snapshot
                .surface_control(&"private-output".into(), &seat)
                .unwrap()
                .seat(),
            &seat
        );
    }

    #[test]
    fn diagnostics_never_expose_output_seat_or_window_identity() {
        let mut coordinator = Coordinator::default();
        let snapshot = connect(
            &mut coordinator,
            "private-output-secret",
            &["private-seat-secret"],
        );
        let diagnostics = format!("{snapshot:?}");
        assert!(!diagnostics.contains("private-output-secret"));
        assert!(!diagnostics.contains("private-seat-secret"));
        assert!(!diagnostics.contains("private window title"));
        assert!(diagnostics.contains("outputs: 1"));
        assert!(diagnostics.contains("seats: 1"));
    }

    #[test]
    fn unchanged_source_updates_do_not_change_the_snapshot() {
        let mut coordinator = Coordinator::default();
        connect(&mut coordinator, "private-output", &["private-seat"]);
        assert!(!coordinator.apply_seats(SeatEvent::Snapshot(
            rmac_shell_invocation::SeatInventory::new(vec!["private-seat".into()]).unwrap()
        )));
        assert!(
            !coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
                state: rmac_compositor::ConnectionState::Connected,
            })
        );
    }
}
