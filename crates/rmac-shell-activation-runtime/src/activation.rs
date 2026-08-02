use std::fmt;

use crate::Context;

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
    pub(crate) fn new(
        event: rmac_shortcuts::Event,
        context: Result<Context, ActivationError>,
    ) -> Self {
        Self { event, context }
    }

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
