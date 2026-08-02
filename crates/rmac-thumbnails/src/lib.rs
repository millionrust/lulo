//! Cross-platform, invalidation-safe image and bounded media preview generation.

mod api;
mod media;
mod renderer;

pub use api::*;
pub use media::{generate_media_preview, media_kind};
pub use renderer::is_current;

#[cfg(test)]
mod tests;
