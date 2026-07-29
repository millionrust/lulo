//! Process coordination between top-bar projections and a real layer host.

use crate::surfaces;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub surfaces: surfaces::Snapshot,
    pub lifecycle_error: Option<surfaces::LifecycleError>,
    pub host_ready: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub snapshot: Snapshot,
    /// At most one command. The host must acknowledge it before another can
    /// be issued.
    pub command: Option<surfaces::Command>,
    pub changed: bool,
}

#[derive(Default)]
pub struct Session {
    registry: surfaces::Registry,
    lifecycle_error: Option<surfaces::LifecycleError>,
    host_ready: bool,
}

impl Session {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            surfaces: self.registry.snapshot(),
            lifecycle_error: self.lifecycle_error,
            host_ready: self.host_ready,
        }
    }

    /// Accept the newest complete runtime projection. Invalid updates preserve
    /// every previously accepted desired and applied surface.
    pub fn apply(&mut self, update: &rmac_top_bar::Update) -> Transition {
        let changed = match self.registry.set_update(update) {
            Ok(surface_changed) => {
                let recovered = self.lifecycle_error.take().is_some();
                surface_changed || recovered
            }
            Err(error) => {
                let changed = self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
                changed
            }
        };
        self.issue_next(changed)
    }

    /// Commands remain queued until the real layer host explicitly reports
    /// readiness.
    pub fn host_ready(&mut self) -> Transition {
        let changed = !self.host_ready;
        self.host_ready = true;
        self.issue_next(changed)
    }

    /// A disconnected host leaves the exact pending platform mutation
    /// uncertain. Mark it failed and issue nothing else until a new host is
    /// ready.
    pub fn host_stopped(&mut self) -> Transition {
        let mut changed = self.host_ready;
        self.host_ready = false;
        if let Some(command) = self.registry.snapshot().pending {
            changed |= self
                .registry
                .finish(command.id(), surfaces::CommandResult::Failed)
                .visible;
        }
        Transition {
            snapshot: self.snapshot(),
            command: None,
            changed,
        }
    }

    pub fn finish(
        &mut self,
        id: surfaces::CommandId,
        result: surfaces::CommandResult,
    ) -> Transition {
        let transition = self.registry.finish(id, result);
        self.issue_next(transition.visible)
    }

    /// Forward an authoritative compositor close. If the output remains
    /// desired, a ready host receives a fresh physical identity immediately.
    pub fn surface_closed(&mut self, surface: surfaces::SurfaceId) -> Transition {
        let transition = self.registry.surface_closed(surface);
        self.issue_next(transition.visible)
    }

    pub fn retry(&mut self, output: &rmac_compositor::OutputId) -> Transition {
        let transition = self.registry.retry(output);
        self.issue_next(transition.visible)
    }

    fn issue_next(&mut self, mut changed: bool) -> Transition {
        if !self.host_ready {
            return Transition {
                snapshot: self.snapshot(),
                command: None,
                changed,
            };
        }
        let command = match self.registry.next_command() {
            Ok(command) => {
                changed |= command.is_some();
                command
            }
            Err(error) => {
                changed |= self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
                None
            }
        };
        Transition {
            snapshot: self.snapshot(),
            command,
            changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn update(outputs: &[&str], minute: u32) -> rmac_top_bar::Update {
        let status = rmac_shell_status::Snapshot {
            outputs: outputs
                .iter()
                .map(|output| rmac_shell_status::OutputContext {
                    id: (*output).into(),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 1.0,
                })
                .collect(),
            ..Default::default()
        };
        let now = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 29, 12, minute, 0)
            .single()
            .unwrap();
        rmac_top_bar::Update {
            projection: rmac_top_bar::project(
                &status,
                now,
                rmac_top_bar::LocaleHourCycle::TwentyFourHour,
            ),
            redraw: true,
            next_clock_update: std::time::Duration::from_secs(60),
        }
    }

    fn ready_session() -> Session {
        let mut session = Session::default();
        let ready = session.host_ready();
        assert!(ready.changed);
        assert!(ready.command.is_none());
        session
    }

    #[test]
    fn projections_queue_until_host_ready_then_drain_serially() {
        let mut session = Session::default();
        let queued = session.apply(&update(&["B", "A"], 0));
        assert!(queued.command.is_none());
        assert!(!queued.snapshot.host_ready);

        let first = session.host_ready().command.unwrap();
        assert_eq!(first.kind.output().0, "A");
        let second = session
            .finish(first.id(), surfaces::CommandResult::Applied)
            .command
            .unwrap();
        assert_eq!(second.kind.output().0, "B");
        let finished = session.finish(second.id(), surfaces::CommandResult::Applied);
        assert!(finished.command.is_none());
        assert_eq!(finished.snapshot.surfaces.applied.len(), 2);
    }

    #[test]
    fn newer_content_converges_after_an_inflight_create() {
        let mut session = ready_session();
        let create = session.apply(&update(&["A"], 0)).command.unwrap();
        assert!(session.apply(&update(&["A"], 1)).command.is_none());
        let present = session
            .finish(create.id(), surfaces::CommandResult::Applied)
            .command
            .unwrap();
        assert!(matches!(
            present.kind,
            surfaces::CommandKind::Reconfigure { .. }
        ));
    }

    #[test]
    fn host_loss_stops_commands_until_a_new_host_is_ready() {
        let mut session = ready_session();
        let pending = session.apply(&update(&["A", "B"], 0)).command.unwrap();
        let stopped = session.host_stopped();
        assert!(stopped.command.is_none());
        assert!(!stopped.snapshot.host_ready);
        assert_eq!(stopped.snapshot.surfaces.failures.len(), 1);
        assert!(
            !session
                .finish(pending.id(), surfaces::CommandResult::Applied)
                .changed
        );

        let resumed = session.host_ready();
        assert_eq!(resumed.command.unwrap().kind.output().0, "B");
    }

    #[test]
    fn authoritative_close_recreates_with_a_new_identity() {
        let mut session = ready_session();
        let create = session.apply(&update(&["A"], 0)).command.unwrap();
        let surface = create.kind.surface();
        session.finish(create.id(), surfaces::CommandResult::Applied);

        let replacement = session.surface_closed(surface).command.unwrap();
        assert!(matches!(
            replacement.kind,
            surfaces::CommandKind::Create { .. }
        ));
        assert_ne!(replacement.kind.surface(), surface);
    }

    #[test]
    fn invalid_update_stays_visible_until_a_valid_projection_arrives() {
        let mut session = ready_session();
        let accepted = update(&["A"], 0);
        let create = session.apply(&accepted).command.unwrap();
        session.finish(create.id(), surfaces::CommandResult::Applied);

        let mut invalid = accepted;
        invalid
            .projection
            .surfaces
            .push(invalid.projection.surfaces[0].clone());
        let rejected = session.apply(&invalid);
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::DuplicateOutput)
        );
        assert_eq!(rejected.snapshot.surfaces.applied.len(), 1);

        let recovered = session.apply(&update(&["A"], 1));
        assert_eq!(recovered.snapshot.lifecycle_error, None);
        assert!(recovered.command.is_some());
    }
}
