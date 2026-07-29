//! Process coordination between Quick Settings plans and its layer host.

use crate::surface;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub surface: surface::Snapshot,
    pub lifecycle_error: Option<surface::LifecycleError>,
    pub host_ready: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub snapshot: Snapshot,
    /// At most one exact platform command. The host must acknowledge it before
    /// the session issues another.
    pub command: Option<surface::Command>,
    pub changed: bool,
}

#[derive(Default)]
pub struct Session {
    registry: surface::Registry,
    lifecycle_error: Option<surface::LifecycleError>,
    host_ready: bool,
}

impl Session {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            surface: self.registry.snapshot(),
            lifecycle_error: self.lifecycle_error,
            host_ready: self.host_ready,
        }
    }

    /// Accept one complete invocation plan. Invalid input preserves the last
    /// accepted desired and applied surface.
    pub fn open(&mut self, plan: &Result<surface::Description, surface::PlanError>) -> Transition {
        let changed = match self.registry.open(plan) {
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

    pub fn close(&mut self) -> Transition {
        let recovered = self.lifecycle_error.take().is_some();
        let changed = self.registry.close() || recovered;
        self.issue_next(changed)
    }

    /// Desired state remains queued until the real host explicitly reports
    /// readiness.
    pub fn host_ready(&mut self) -> Transition {
        let changed = !self.host_ready;
        self.host_ready = true;
        self.issue_next(changed)
    }

    /// Wayland surfaces cannot survive their owning host. Retire every pending
    /// and applied physical identity while preserving the desired plan, then
    /// wait for a replacement host to report ready before recreating it.
    pub fn host_stopped(&mut self) -> Transition {
        let mut changed = self.host_ready;
        self.host_ready = false;
        let snapshot = self.registry.snapshot();
        let pending = snapshot
            .pending
            .as_ref()
            .map(|command| command.kind.surface());
        let applied = snapshot.applied.as_ref().map(|item| item.surface);
        if let Some(surface) = pending {
            changed |= self.registry.surface_closed(surface).visible;
        }
        if let Some(surface) = applied.filter(|surface| Some(*surface) != pending) {
            changed |= self.registry.surface_closed(surface).visible;
        }
        Transition {
            snapshot: self.snapshot(),
            command: None,
            changed,
        }
    }

    pub fn finish(&mut self, id: surface::CommandId, result: surface::CommandResult) -> Transition {
        let transition = self.registry.finish(id, result);
        self.issue_next(transition.visible)
    }

    pub fn surface_closed(&mut self, surface: surface::SurfaceId) -> Transition {
        let transition = self.registry.surface_closed(surface);
        self.issue_next(transition.visible)
    }

    pub fn retry(&mut self) -> Transition {
        let transition = self.registry.retry();
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
    use super::*;

    fn plan(output: &str, seat: &str) -> Result<surface::Description, surface::PlanError> {
        let snapshot = rmac_compositor::Snapshot {
            outputs: vec![rmac_compositor::Output {
                id: output.into(),
                make: String::new(),
                model: String::new(),
                serial: None,
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
            }],
            ..Default::default()
        };
        surface::plan(
            &output.into(),
            surface::SeatId::new(seat).unwrap(),
            &snapshot,
        )
    }

    fn ready_session() -> Session {
        let mut session = Session::default();
        let transition = session.host_ready();
        assert!(transition.changed);
        assert!(transition.command.is_none());
        session
    }

    #[test]
    fn invocation_waits_for_host_readiness() {
        let mut session = Session::default();
        let queued = session.open(&plan("A", "seat-a"));
        assert!(queued.command.is_none());
        assert!(!queued.snapshot.host_ready);
        let create = session.host_ready().command.unwrap();
        assert_eq!(create.kind.output().0, "A");
    }

    #[test]
    fn newer_output_converges_after_inflight_create() {
        let mut session = ready_session();
        let create = session.open(&plan("A", "seat-a")).command.unwrap();
        assert!(session.open(&plan("B", "seat-b")).command.is_none());
        let remove = session
            .finish(create.id(), surface::CommandResult::Applied)
            .command
            .unwrap();
        assert!(matches!(remove.kind, surface::CommandKind::Remove { .. }));
        let replacement = session
            .finish(remove.id(), surface::CommandResult::Applied)
            .command
            .unwrap();
        assert_eq!(replacement.kind.output().0, "B");
    }

    #[test]
    fn host_loss_retires_applied_identity_and_recreates_after_readiness() {
        let mut session = ready_session();
        let create = session.open(&plan("A", "seat-a")).command.unwrap();
        let old_surface = create.kind.surface();
        session.finish(create.id(), surface::CommandResult::Applied);

        let stopped = session.host_stopped();
        assert!(!stopped.snapshot.host_ready);
        assert!(stopped.snapshot.surface.applied.is_none());
        assert!(stopped.snapshot.surface.desired.is_some());
        assert!(stopped.command.is_none());

        let replacement = session.host_ready().command.unwrap();
        assert!(matches!(
            replacement.kind,
            surface::CommandKind::Create { .. }
        ));
        assert_ne!(replacement.kind.surface(), old_surface);
    }

    #[test]
    fn host_loss_cancels_uncertain_pending_work_and_stale_ack() {
        let mut session = ready_session();
        let pending = session.open(&plan("A", "seat-a")).command.unwrap();
        session.host_stopped();
        assert!(
            !session
                .finish(pending.id(), surface::CommandResult::Applied)
                .changed
        );
        assert!(session.host_ready().command.is_some());
    }

    #[test]
    fn close_removes_applied_surface_and_clears_plan_error() {
        let mut session = ready_session();
        let create = session.open(&plan("A", "seat-a")).command.unwrap();
        session.finish(create.id(), surface::CommandResult::Applied);
        assert_eq!(
            session
                .open(&Err(surface::PlanError::OutputMissing))
                .snapshot
                .lifecycle_error,
            Some(surface::LifecycleError::InvalidPlan(
                surface::PlanError::OutputMissing
            ))
        );
        let remove = session.close().command.unwrap();
        assert!(matches!(remove.kind, surface::CommandKind::Remove { .. }));
        assert!(session.snapshot().lifecycle_error.is_none());

        let mut failed = ready_session();
        let create = failed.open(&plan("A", "seat-a")).command.unwrap();
        failed.finish(create.id(), surface::CommandResult::Failed);
        let closed = failed.close();
        assert!(closed.snapshot.surface.desired.is_none());
        assert!(closed.snapshot.surface.failure.is_none());
    }

    #[test]
    fn diagnostics_redact_invocation_identity() {
        let mut session = Session::default();
        let snapshot = session
            .open(&plan("private-output-82", "private-seat-51"))
            .snapshot;
        let diagnostics = format!("{snapshot:?}");
        assert!(!diagnostics.contains("private-output-82"));
        assert!(!diagnostics.contains("private-seat-51"));
    }
}
