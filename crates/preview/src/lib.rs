//! rmac Preview: measured metrics, zoom arithmetic, page layout, document
//! identity and navigation, poppler output parsing, and the blocking
//! decode/render work — shared by the binary, its tests and Quick Look.

pub mod bounded;
pub mod document;
pub mod layout;
pub mod metrics;
pub mod pdfwriter;
pub mod poppler;
pub mod render;
pub mod zoom;
