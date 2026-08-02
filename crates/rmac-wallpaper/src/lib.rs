//! Framework-neutral wallpaper source, output planning, and fit geometry.

mod layout;
mod plan;
mod source;

pub mod portal;
pub mod transition;

pub use layout::{layout, Layout, LayoutError, Rect};
pub use plan::{plan, Issue, Plan, Surface};
pub use source::{
    file_path, parse_source, BuiltInId, BuiltInMetadata, Source, SourceErrorKind, DEFAULT_BUILT_IN,
};

#[cfg(test)]
mod tests;
