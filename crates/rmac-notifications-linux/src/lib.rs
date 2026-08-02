//! Linux D-Bus wire decoding for the rmac notification authority.

pub mod banner;
pub mod center;
pub mod center_surface;
mod decode;
pub mod icon;
pub mod icon_worker;
pub mod media;
mod model;
pub mod service;
pub mod surface_session;
pub mod surfaces;

pub use decode::{freedesktop, portal, portal_with_media, PortalDecoded};
pub use model::*;

#[cfg(test)]
mod tests;
