//! rmac Preview: measured metrics, zoom arithmetic, page layout, document
//! identity and navigation, poppler output parsing, and the blocking
//! decode/render work — shared by the binary, its tests and Quick Look.

pub mod bounded;
pub mod document;
pub mod layout;
pub mod markup;
pub mod metrics;
pub mod pdfwriter;
pub mod poppler;
pub mod redact;
pub mod render;
pub mod selection;
pub mod settings_store;
pub mod versions;
#[cfg(windows)]
pub mod winpdf;
pub mod zoom;
