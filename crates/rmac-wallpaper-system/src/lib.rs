//! Safe local-file authority for wallpaper decoding adapters.

mod file;
mod model;
mod resolution;

pub use file::open_file;
pub use model::{
    Error, ErrorKind, FileAsset, ImageFormat, Resolution, ResolutionIssue, ResolvedSource,
    ResolvedSurface, MAX_WALLPAPER_BYTES,
};
pub use resolution::{resolve, resolve_plan};

#[cfg(test)]
mod tests;
