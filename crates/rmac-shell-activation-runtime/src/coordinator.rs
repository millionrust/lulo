use crate::{Activation, ActivationError, Context};

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
                .map(|invocation| {
                    Context::new(
                        invocation,
                        self.runtime
                            .compositor()
                            .expect("successful resolution requires a compositor snapshot")
                            .clone(),
                    )
                })
                .map_err(ActivationError::Resolve)
        } else {
            Err(ActivationError::NotActivated)
        };
        Activation::new(event, context)
    }

    fn take_readiness(&mut self) -> bool {
        if self.readiness_announced || !self.endpoint_ready || !self.runtime.ready() {
            return false;
        }
        self.readiness_announced = true;
        true
    }
}
