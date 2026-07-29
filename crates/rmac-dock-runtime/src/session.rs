//! Process coordination between Dock runtime snapshots and a real layer host.

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

    /// Accept the complete newest runtime snapshot. Invalid plans retain every
    /// previously accepted desired and applied surface.
    pub fn apply(&mut self, update: &crate::Update) -> Transition {
        let changed = match self.registry.set_runtime_snapshot(&update.snapshot) {
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

    pub fn host_ready(&mut self) -> Transition {
        let changed = !self.host_ready;
        self.host_ready = true;
        self.issue_next(changed)
    }

    /// Host loss makes an exact in-flight platform operation uncertain.
    /// Record it as failed and issue nothing until another host is ready.
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
    use super::*;

    fn description(
        output: &str,
        placement: rmac_shell_settings::DockPlacement,
    ) -> rmac_dock::SurfaceDescription {
        rmac_dock::SurfaceDescription {
            output: output.into(),
            placement,
            output_axis_length: 1920.0,
            output_scale: 1.0,
            base_thickness: 64.0,
            maximum_thickness: 88.0,
            exclusive_zone: 64.0,
            reveal_edge_thickness: 0.0,
            keyboard_interactive: false,
            autohide: false,
            overview_visible: false,
            magnification_enabled: true,
            animate: true,
            magnification: rmac_dock::motion::MagnificationConfig::default(),
        }
    }

    fn update(outputs: &[&str], placement: rmac_shell_settings::DockPlacement) -> crate::Update {
        crate::Update {
            snapshot: crate::Snapshot {
                outputs: outputs.iter().map(|output| (*output).into()).collect(),
                surface_plan: Ok(outputs
                    .iter()
                    .map(|output| description(output, placement))
                    .collect()),
                ..Default::default()
            },
            visible: true,
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
    fn snapshots_queue_until_host_ready_then_drain_serially() {
        let mut session = Session::default();
        let queued = session.apply(&update(
            &["B", "A"],
            rmac_shell_settings::DockPlacement::Bottom,
        ));
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
    fn newer_policy_converges_after_an_inflight_create() {
        let mut session = ready_session();
        let create = session
            .apply(&update(&["A"], rmac_shell_settings::DockPlacement::Bottom))
            .command
            .unwrap();
        assert!(session
            .apply(&update(&["A"], rmac_shell_settings::DockPlacement::Left,))
            .command
            .is_none());
        let reconfigure = session
            .finish(create.id(), surfaces::CommandResult::Applied)
            .command
            .unwrap();
        assert!(matches!(
            reconfigure.kind,
            surfaces::CommandKind::Reconfigure { .. }
        ));
    }

    #[test]
    fn host_loss_stops_commands_until_a_replacement_is_ready() {
        let mut session = ready_session();
        let pending = session
            .apply(&update(
                &["A", "B"],
                rmac_shell_settings::DockPlacement::Bottom,
            ))
            .command
            .unwrap();
        let stopped = session.host_stopped();
        assert!(stopped.command.is_none());
        assert!(!stopped.snapshot.host_ready);
        assert_eq!(stopped.snapshot.surfaces.failures.len(), 1);
        assert!(
            !session
                .finish(pending.id(), surfaces::CommandResult::Applied)
                .changed
        );
        assert_eq!(session.host_ready().command.unwrap().kind.output().0, "B");
    }

    #[test]
    fn compositor_close_recreates_a_still_desired_dock() {
        let mut session = ready_session();
        let create = session
            .apply(&update(&["A"], rmac_shell_settings::DockPlacement::Bottom))
            .command
            .unwrap();
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
    fn invalid_plan_remains_visible_until_valid_recovery() {
        let mut session = ready_session();
        let accepted = update(&["A"], rmac_shell_settings::DockPlacement::Bottom);
        let create = session.apply(&accepted).command.unwrap();
        session.finish(create.id(), surfaces::CommandResult::Applied);

        let mut invalid = accepted;
        invalid.snapshot.surface_plan = Err(rmac_dock::motion::ConfigError::MaximumScale);
        let rejected = session.apply(&invalid);
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::InvalidPlan(
                rmac_dock::motion::ConfigError::MaximumScale
            ))
        );
        assert_eq!(rejected.snapshot.surfaces.applied.len(), 1);

        let recovered = session.apply(&update(&["A"], rmac_shell_settings::DockPlacement::Left));
        assert_eq!(recovered.snapshot.lifecycle_error, None);
        assert!(recovered.command.is_some());
    }
}
