//! Framework-neutral wallpaper source, output planning, and fit geometry.

mod layout;
mod plan;
mod source;

pub mod portal;
pub mod transition;

pub use layout::{layout, Layout, LayoutError, Rect};
pub use plan::{plan, Issue, Plan, Surface};
pub use source::{
    artwork_size, file_path, packaged_wallpaper_dir, parse_source, BuiltInId, BuiltInMetadata,
    Source, SourceErrorKind, ARTWORK_SIZES, DEFAULT_BUILT_IN, FALLBACK_BUILT_IN,
    PACKAGED_WALLPAPER_DIR, PACKAGED_WALLPAPER_DIR_ENV,
};

#[cfg(test)]
mod tests;
