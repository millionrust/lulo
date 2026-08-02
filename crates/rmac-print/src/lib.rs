//! Bounded Unicode document rendering for rmac print/export workflows.

mod model;
mod portal;
mod render;

pub use model::*;
pub use portal::{
    OutputFormat, PortalPrintError, PortalPrintPhase, PortalPrintTransaction, PrintIdentity,
    PrintSubmission,
};
pub use render::render_pdf;

#[cfg(test)]
mod tests;
