//! Shared AT-SPI accessibility-tree metadata, patched onto GPUI's per-frame
//! [`accesskit::TreeUpdate`] before it reaches `accesskit_unix`.
//!
//! `accesskit_unix` 0.22.1 (still true as of the latest published 0.23.0)
//! derives the AT-SPI *application* object's `Name` property — what a screen
//! reader like Orca announces as the app's name — from
//! `std::env::current_exe()`'s file name, through a private,
//! `OnceLock`-backed `AppContext` with no public constructor argument or
//! setter to override it (`accesskit_unix::context::app_name`/
//! `get_or_init_app_context`, both crate-private). There is therefore no hook
//! *anywhere* in application code, including here, that can turn
//! `rmac-calculator` into `Calculator` for that property without forking
//! `accesskit_unix` itself; see the "AT-SPI application name" amendment in
//! `docs/decisions/0013-gpui-linux-patch.md` for the full trace.
//!
//! What *is* reachable from here is the per-window [`accesskit::Tree`]'s
//! `toolkit_name`/`toolkit_version`, a distinct pair of AT-SPI
//! `Application.ToolkitName`/`Application.Version` properties. Upstream GPUI
//! (`crates/gpui/src/window/a11y.rs`) never fills these in, so AT clients
//! currently see `accesskit_consumer`'s generic fallback (`"AccessKit"`, no
//! version) for every rmac window. This module fixes that.

/// The value AT-SPI's `Application.ToolkitName` property reports for every
/// rmac window, in place of `accesskit_consumer`'s `"AccessKit"` fallback.
pub(crate) const TOOLKIT_NAME: &str = "gpui_linux";

/// The value AT-SPI's `Application.Version` property reports alongside
/// [`TOOLKIT_NAME`]: this crate's own vendored version.
pub(crate) fn toolkit_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Label a frame's [`accesskit::Tree`] (when present) with rmac's toolkit
/// identity, so AT-SPI's `Application.ToolkitName`/`Application.Version`
/// describe the actual backend instead of falling back to `"AccessKit"` with
/// no version. A no-op when the update carries no `Tree` (most per-frame
/// updates omit it once the initial tree has been sent).
pub(crate) fn label_toolkit(tree_update: &mut accesskit::TreeUpdate) {
    if let Some(tree) = tree_update.tree.as_mut() {
        tree.toolkit_name = Some(TOOLKIT_NAME.to_owned());
        tree.toolkit_version = Some(toolkit_version().to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{NodeId, Tree, TreeId, TreeUpdate};

    fn update_with_tree() -> TreeUpdate {
        TreeUpdate {
            nodes: Vec::new(),
            tree: Some(Tree::new(NodeId(0))),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    #[test]
    fn label_toolkit_fills_in_name_and_version() {
        let mut update = update_with_tree();
        label_toolkit(&mut update);
        let tree = update.tree.as_ref().expect("tree survives labeling");
        assert_eq!(tree.toolkit_name.as_deref(), Some(TOOLKIT_NAME));
        assert_eq!(tree.toolkit_version.as_deref(), Some(toolkit_version()));
    }

    #[test]
    fn label_toolkit_is_a_no_op_without_a_tree() {
        let mut update = update_with_tree();
        update.tree = None;
        label_toolkit(&mut update);
        assert!(update.tree.is_none());
    }
}
