//! Bounded Unicode document rendering for rmac print/export workflows.

mod model;
mod portal;
mod render;
mod rich;

pub use model::*;
pub use portal::{
    OutputFormat, PortalPrintError, PortalPrintPhase, PortalPrintTransaction, PrintIdentity,
    PrintSubmission,
};
pub use render::render_pdf;
pub use rich::{render_rich_pdf, RichAlign, RichLine, RichSpan};

#[cfg(test)]
mod tests;
