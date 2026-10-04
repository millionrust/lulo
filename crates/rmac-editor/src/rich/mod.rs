//! Rich text: an attributed-text model, RTF reading and writing, and the
//! [`RichTextEditor`] view that edits it.

mod editor;
mod layout;
pub mod model;
pub mod palette;
pub mod rtf;

pub use editor::{RichTextEditor, RichTextEvent};
pub use model::{
    Alignment, CharStyle, Document, ListKind, Paragraph, ParagraphStyle, Rgb, StyledRun,
    DEFAULT_RICH_SIZE,
};
