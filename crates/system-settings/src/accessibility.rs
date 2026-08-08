//! Bounded sidebar, search, account, detail, subpage, and global-feedback semantics.

mod budget;
mod matching;
mod model;
mod projection;

pub use matching::{category_match_hint, category_matches};
pub use model::*;
pub use projection::project_settings_navigation;

#[cfg(test)]
mod tests;
