//! Rich text: an attributed-text model, RTF reading and writing, and the
//! [`RichTextEditor`] view that edits it.

pub mod clipboard;
mod editor;
pub mod html;
mod layout;
pub mod model;
pub mod palette;
pub mod rtf;

pub use editor::{RichTextEditor, RichTextEvent};
pub use model::{
    Alignment, CharStyle, Document, DocumentAttributes, DocumentProperties, Ligatures, ListKind,
    Paragraph, ParagraphStyle, Rgb, StyledRun, BASELINE_STEP, DEFAULT_RICH_SIZE, KERN_STEP,
    MAX_LIST_LEVEL,
};
