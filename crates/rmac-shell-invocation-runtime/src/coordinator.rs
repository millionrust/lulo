use crate::{Health, SeatEvent, Snapshot, SourceHealth};

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
