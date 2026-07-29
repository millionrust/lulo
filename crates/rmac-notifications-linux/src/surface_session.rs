//! Process coordination between banner surface plans and a real layer host.

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

    /// Accept one complete measured banner plan. Invalid plans preserve every
    /// previously accepted desired and applied surface.
    pub fn apply(&mut self, plan: &Result<surfaces::Plan, surfaces::PlanError>) -> Transition {
        let changed = match self.registry.set_desired(plan) {
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

    /// Wayland surfaces cannot survive their owning host. Retire every pending
    /// and applied physical identity while preserving desired banner plans.
    pub fn host_stopped(&mut self) -> Transition {
        let mut changed = self.host_ready;
        self.host_ready = false;
        let snapshot = self.registry.snapshot();
        let mut physical = snapshot
            .applied
            .iter()
            .map(|applied| applied.surface)
            .collect::<Vec<_>>();
        if let Some(command) = snapshot.pending {
            physical.push(command.kind.surface());
        }
        physical.sort_unstable();
        physical.dedup();
        for surface in physical {
            changed |= self.registry.surface_closed(surface).visible;
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
    use rmac_notifications::NotificationId;

    use super::*;

    fn description(output: &str, focused: bool) -> surfaces::SurfaceDescription {
        let notification = NotificationId::from_protocol(1).unwrap();
        surfaces::SurfaceDescription {
            output: output.into(),
            namespace: surfaces::NAMESPACE,
            layer: surfaces::Layer::Overlay,
            anchors: surfaces::Anchors::TOP_RIGHT,
            logical_width: 360,
            logical_height: 100,
            output_scale: 1.0,
            top_margin: 12,
            right_margin: 12,
            exclusive_zone: 0,
            keyboard_interactivity: if focused {
                surfaces::KeyboardInteractivity::OnDemand
            } else {
                surfaces::KeyboardInteractivity::None
            },
            gap: 10,
            cards: vec![surfaces::CardSlot {
                notification,
                stack_index: 0,
                x: 0,
                y: 0,
                width: 360,
                height: 100,
            }],
            input_regions: vec![
                surfaces::InputRegion {
                    x: 14,
                    y: 0,
                    width: 332,
                    height: 100,
                },
                surfaces::InputRegion {
                    x: 0,
                    y: 14,
                    width: 360,
                    height: 72,
                },
            ],
        }
    }

    fn plan(outputs: &[&str], focused: bool) -> Result<surfaces::Plan, surfaces::PlanError> {
        Ok(surfaces::Plan {
            surfaces: outputs
                .iter()
                .map(|output| description(output, focused))
                .collect(),
        })
    }

    fn ready_session() -> Session {
        let mut session = Session::default();
        let ready = session.host_ready();
        assert!(ready.changed);
        assert!(ready.command.is_none());
        session
    }

    #[test]
    fn plans_queue_until_host_ready_then_drain_serially() {
        let mut session = Session::default();
        let queued = session.apply(&plan(&["B", "A"], false));
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
    fn focus_change_converges_after_an_inflight_create() {
        let mut session = ready_session();
        let create = session.apply(&plan(&["A"], false)).command.unwrap();
        assert!(session.apply(&plan(&["A"], true)).command.is_none());
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
    fn host_loss_retires_all_physical_state_until_replacement_readiness() {
        let mut session = ready_session();
        let first = session.apply(&plan(&["A", "B"], false)).command.unwrap();
        let old_surface = first.kind.surface();
        let pending = session
            .finish(first.id(), surfaces::CommandResult::Applied)
            .command
            .unwrap();
        let stopped = session.host_stopped();
        assert!(stopped.command.is_none());
        assert!(!stopped.snapshot.host_ready);
        assert!(stopped.snapshot.surfaces.applied.is_empty());
        assert!(stopped.snapshot.surfaces.pending.is_none());
        assert!(stopped.snapshot.surfaces.failures.is_empty());
        assert!(
            !session
                .finish(pending.id(), surfaces::CommandResult::Applied)
                .changed
        );
        let replacement = session.host_ready().command.unwrap();
        assert_eq!(replacement.kind.output().0, "A");
        assert_ne!(replacement.kind.surface(), old_surface);
    }

    #[test]
    fn compositor_close_recreates_a_still_desired_banner_surface() {
        let mut session = ready_session();
        let create = session.apply(&plan(&["A"], false)).command.unwrap();
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
        let accepted = plan(&["A"], false);
        let create = session.apply(&accepted).command.unwrap();
        session.finish(create.id(), surfaces::CommandResult::Applied);

        let rejected = session.apply(&Err(surfaces::PlanError::InvalidLayout));
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::InvalidPlan(
                surfaces::PlanError::InvalidLayout
            ))
        );
        assert_eq!(rejected.snapshot.surfaces.applied.len(), 1);

        let recovered = session.apply(&plan(&["A"], true));
        assert_eq!(recovered.snapshot.lifecycle_error, None);
        assert!(recovered.command.is_some());
    }

    #[test]
    fn diagnostics_never_expose_output_identity() {
        let mut session = ready_session();
        let transition = session.apply(&plan(&["private-output-8472"], false));
        let diagnostics = format!("{:?}", transition.snapshot);
        assert!(!diagnostics.contains("private-output-8472"));
    }
}
