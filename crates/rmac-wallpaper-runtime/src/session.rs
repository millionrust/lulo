//! Process-level coordination between wallpaper authorities and a surface host.
//!
//! The real Wayland adapter executes one returned command and acknowledges it
//! through this session. Runtime updates may continue arriving while a command
//! is in flight; the registry converges onto the newest complete desired state
//! after that exact acknowledgement.

use crate::{surfaces, HealthSnapshot, Update};

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub health: HealthSnapshot,
    pub surfaces: surfaces::Snapshot,
    pub lifecycle_error: Option<surfaces::LifecycleError>,
    pub host_ready: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub snapshot: Snapshot,
    /// At most one command. A host must acknowledge it before another command
    /// can be issued.
    pub command: Option<surfaces::Command>,
    /// True when diagnostics, desired/applied state, failure state, or pending
    /// work changed.
    pub changed: bool,
}

#[derive(Default)]
pub struct Session {
    registry: surfaces::Registry,
    health: HealthSnapshot,
    lifecycle_error: Option<surfaces::LifecycleError>,
    host_ready: bool,
}

impl Session {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            health: self.health.clone(),
            surfaces: self.registry.snapshot(),
            lifecycle_error: self.lifecycle_error,
            host_ready: self.host_ready,
        }
    }

    /// Admit commands only after the real background-surface host reports that
    /// it can execute them.
    pub fn host_ready(&mut self) -> Transition {
        let changed = !self.host_ready;
        self.host_ready = true;
        self.issue_next(changed)
    }

    /// Accept one runtime publication. Invalid whole updates retain every
    /// previously accepted desired and applied surface.
    pub fn apply(&mut self, update: &Update) -> Transition {
        let is_render = matches!(update, Update::Render { .. });
        let next_health = match update {
            Update::Render { health, .. } | Update::Health(health) => health.clone(),
        };
        let mut changed = self.health != next_health;
        self.health = next_health;

        match self.registry.set_update(update) {
            Ok(surface_changed) => {
                changed |= surface_changed;
                if is_render {
                    changed |= self.lifecycle_error.take().is_some();
                }
            }
            Err(error) => {
                changed |= self.lifecycle_error != Some(error);
                self.lifecycle_error = Some(error);
            }
        }
        self.issue_next(changed)
    }

    /// Complete only the exact pending command. Stale acknowledgements are
    /// inert; a valid completion immediately yields the next converging command
    /// when another output or a newer desired frame needs work.
    pub fn finish(
        &mut self,
        id: surfaces::CommandId,
        result: surfaces::CommandResult,
    ) -> Transition {
        let transition = self.registry.finish(id, result);
        self.issue_next(transition.visible)
    }

    /// Wayland surfaces cannot survive their owning host. Retire every pending
    /// and applied physical identity while preserving desired raster state.
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

    pub fn surface_closed(&mut self, surface: surfaces::SurfaceId) -> Transition {
        let transition = self.registry.surface_closed(surface);
        self.issue_next(transition.visible)
    }

    /// Retry one exact output failure. Other outputs remain unaffected.
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
    use std::sync::Arc;

    use super::*;

    fn plan(outputs: &[&str]) -> rmac_wallpaper::Plan {
        rmac_wallpaper::Plan {
            surfaces: outputs
                .iter()
                .map(|output| rmac_wallpaper::Surface {
                    output: rmac_compositor::OutputId::from(*output),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 1.0,
                    fit: rmac_shell_settings::WallpaperFit::Fill,
                    source: rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN),
                })
                .collect(),
            issues: Vec::new(),
        }
    }

    fn raster(output: &str, revision: u8) -> rmac_wallpaper_image::RasterSurface {
        let logical_size = rmac_compositor::LogicalSize {
            width: 1920.0,
            height: 1080.0,
        };
        rmac_wallpaper_image::RasterSurface {
            output: output.into(),
            logical_size,
            scale: 1.0,
            fit: rmac_shell_settings::WallpaperFit::Fill,
            layout: rmac_wallpaper::layout(
                rmac_shell_settings::WallpaperFit::Fill,
                rmac_compositor::PhysicalSize {
                    width: 1,
                    height: 1,
                },
                logical_size,
                1.0,
            )
            .unwrap(),
            image: Arc::new(rmac_wallpaper_image::Decoded {
                width: 1,
                height: 1,
                rgba: Arc::from([revision, revision, revision, 255]),
            }),
        }
    }

    fn render(outputs: &[(&str, u8)]) -> Update {
        Update::Render {
            plan: plan(
                &outputs
                    .iter()
                    .map(|(output, _)| *output)
                    .collect::<Vec<_>>(),
            ),
            rasterized: rmac_wallpaper_image::Rasterized {
                surfaces: outputs
                    .iter()
                    .map(|(output, revision)| raster(output, *revision))
                    .collect(),
                issues: Vec::new(),
            },
            health: HealthSnapshot {
                compositor: crate::SourceHealth::Healthy,
                settings: crate::SourceHealth::Healthy,
                files: crate::SourceHealth::Healthy,
            },
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
    fn commands_are_serial_and_drain_in_stable_output_order() {
        let mut session = Session::default();
        let queued = session.apply(&render(&[("B", 1), ("A", 1)]));
        assert!(queued.command.is_none());
        assert!(!queued.snapshot.host_ready);
        let first = session.host_ready().command.unwrap();
        assert_eq!(first.kind.output().0, "A");
        assert!(matches!(first.kind, surfaces::CommandKind::Create { .. }));

        let second_transition = session.finish(first.id(), surfaces::CommandResult::Applied);
        assert_eq!(second_transition.snapshot.surfaces.applied.len(), 1);
        let second = second_transition.command.unwrap();
        assert_eq!(second.kind.output().0, "B");

        let finished = session.finish(second.id(), surfaces::CommandResult::Applied);
        assert!(finished.command.is_none());
        assert_eq!(finished.snapshot.surfaces.applied.len(), 2);
        assert!(finished.snapshot.surfaces.pending.is_none());
    }

    #[test]
    fn an_inflight_create_converges_to_the_newest_presented_frame() {
        let mut session = ready_session();
        let create = session.apply(&render(&[("A", 1)])).command.unwrap();
        let queued = session.apply(&render(&[("A", 2)]));
        assert!(queued.command.is_none());

        let present = session.finish(create.id(), surfaces::CommandResult::Applied);
        let present = present.command.unwrap();
        assert!(matches!(
            present.kind,
            surfaces::CommandKind::Present { .. }
        ));
        assert_eq!(present.kind.output().0, "A");
    }

    #[test]
    fn a_failed_output_does_not_block_other_outputs_and_can_retry() {
        let mut session = ready_session();
        let first = session
            .apply(&render(&[("A", 1), ("B", 1)]))
            .command
            .unwrap();
        let second_transition = session.finish(first.id(), surfaces::CommandResult::Failed);
        assert_eq!(second_transition.snapshot.surfaces.failures.len(), 1);
        let second_command = second_transition.command.unwrap();
        assert_eq!(second_command.kind.output().0, "B");

        let finished = session.finish(second_command.id(), surfaces::CommandResult::Applied);
        assert!(finished.command.is_none());
        let retried = session.retry(&rmac_compositor::OutputId::from("A"));
        assert_eq!(retried.command.unwrap().kind.output().0, "A");
    }

    #[test]
    fn host_loss_retires_physical_state_and_invalid_updates_preserve_state() {
        let mut session = ready_session();
        let first = session
            .apply(&render(&[("A", 1), ("B", 1)]))
            .command
            .unwrap();
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
        let resumed = session.host_ready();
        let replacement = resumed.command.as_ref().unwrap();
        assert_eq!(replacement.kind.output().0, "A");
        assert_ne!(replacement.kind.surface(), old_surface);

        let invalid = Update::Render {
            plan: plan(&["A", "A", "B"]),
            rasterized: rmac_wallpaper_image::Rasterized::default(),
            health: resumed.snapshot.health.clone(),
        };
        let rejected = session.apply(&invalid);
        assert_eq!(
            rejected.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::DuplicatePlanOutput)
        );
        assert_eq!(rejected.snapshot.surfaces.requested_outputs.len(), 2);
        assert!(rejected.snapshot.surfaces.failures.is_empty());

        let health_only = session.apply(&Update::Health(rejected.snapshot.health.clone()));
        assert_eq!(
            health_only.snapshot.lifecycle_error,
            Some(surfaces::LifecycleError::DuplicatePlanOutput)
        );
        let accepted = session.apply(&render(&[("A", 2), ("B", 1)]));
        assert_eq!(accepted.snapshot.lifecycle_error, None);
    }

    #[test]
    fn health_only_updates_never_create_surface_work() {
        let mut session = Session::default();
        let health = HealthSnapshot {
            compositor: crate::SourceHealth::Unavailable {
                detail: "private compositor socket".into(),
            },
            settings: crate::SourceHealth::Healthy,
            files: crate::SourceHealth::Healthy,
        };
        let transition = session.apply(&Update::Health(health));
        assert!(transition.command.is_none());
        assert!(transition.changed);
        assert!(transition.snapshot.surfaces.pending.is_none());
        assert!(!format!("{:?}", transition.snapshot.health).contains("private compositor"));
    }
}
