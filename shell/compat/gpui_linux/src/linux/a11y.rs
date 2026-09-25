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

/// Class name that `rmac_ui::accessibility` puts on a node publishing the
/// text, caret and selection of the text field it contains.
pub(crate) const TEXT_PROXY_CLASS: &str = "rmac-text-proxy";

/// Everything rmac changes on a frame's tree before `accesskit_unix` sees it.
pub(crate) fn prepare_tree_update(tree_update: &mut accesskit::TreeUpdate) {
    label_toolkit(tree_update);
    collapse_text_proxies(tree_update);
}

fn is_text_input(role: accesskit::Role) -> bool {
    use accesskit::Role;
    matches!(
        role,
        Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::PasswordInput
            | Role::EmailInput
            | Role::UrlInput
            | Role::PhoneNumberInput
            | Role::NumberInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::TimeInput
            | Role::WeekInput
            | Role::MonthInput
    )
}

/// Fold each text field's own input node into the proxy node around it.
///
/// A gpui-component text field reports a text role and holds keyboard focus
/// but publishes no text, and there is no API to give its node text runs.
/// rmac wraps the field in a node that does publish them (see
/// `rmac_ui::accessibility`), which left AT-SPI with a named, readable entry
/// containing an unnamed, empty, focused one. Here the inner node's children
/// move up into the proxy, the inner node goes away, and focus moves to the
/// proxy, so a screen reader lands on one field with its name, text and
/// caret. Frames without a proxy are left untouched.
pub(crate) fn collapse_text_proxies(tree_update: &mut accesskit::TreeUpdate) {
    use collections::{FxHashMap, FxHashSet};

    let proxies: Vec<usize> = tree_update
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, (_, node))| node.class_name() == Some(TEXT_PROXY_CLASS))
        .map(|(index, _)| index)
        .collect();
    if proxies.is_empty() {
        return;
    }
    let index: FxHashMap<accesskit::NodeId, usize> = tree_update
        .nodes
        .iter()
        .enumerate()
        .map(|(position, (id, _))| (*id, position))
        .collect();
    let mut removed = FxHashSet::default();
    for proxy in proxies {
        let proxy_id = tree_update.nodes[proxy].0;
        tree_update.nodes[proxy].1.clear_class_name();
        let children = tree_update.nodes[proxy].1.children().to_vec();
        let Some((slot, inner)) = children.iter().enumerate().find_map(|(slot, child)| {
            let position = *index.get(child)?;
            is_text_input(tree_update.nodes[position].1.role()).then_some((slot, position))
        }) else {
            continue;
        };
        let inner_id = tree_update.nodes[inner].0;
        let mut merged = children[..slot].to_vec();
        merged.extend_from_slice(tree_update.nodes[inner].1.children());
        merged.extend_from_slice(&children[slot + 1..]);
        tree_update.nodes[proxy].1.set_children(merged);
        removed.insert(inner_id);
        if tree_update.focus == inner_id {
            tree_update.focus = proxy_id;
        }
    }
    if !removed.is_empty() {
        tree_update.nodes.retain(|(id, _)| !removed.contains(id));
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
    fn node(role: accesskit::Role, children: &[u64]) -> accesskit::Node {
        let mut node = accesskit::Node::new(role);
        node.set_children(children.iter().copied().map(NodeId).collect::<Vec<_>>());
        node
    }

    #[test]
    fn text_proxy_absorbs_the_focused_inner_field() {
        use accesskit::Role;
        let mut proxy = node(Role::TextInput, &[2, 4]);
        proxy.set_class_name(TEXT_PROXY_CLASS);
        let mut update = TreeUpdate {
            nodes: vec![
                (NodeId(0), node(Role::Window, &[1])),
                (NodeId(1), proxy),
                (NodeId(2), node(Role::TextInput, &[3])),
                (NodeId(3), node(Role::Button, &[])),
                (NodeId(4), node(Role::TextRun, &[])),
            ],
            tree: Some(Tree::new(NodeId(0))),
            tree_id: TreeId::ROOT,
            focus: NodeId(2),
        };
        prepare_tree_update(&mut update);
        assert_eq!(update.focus, NodeId(1));
        assert!(update.nodes.iter().all(|(id, _)| *id != NodeId(2)));
        let proxy = &update
            .nodes
            .iter()
            .find(|(id, _)| *id == NodeId(1))
            .unwrap()
            .1;
        assert_eq!(proxy.children(), &[NodeId(3), NodeId(4)]);
        assert_eq!(proxy.class_name(), None);
    }

    #[test]
    fn frames_without_a_proxy_are_unchanged() {
        use accesskit::Role;
        let mut update = TreeUpdate {
            nodes: vec![
                (NodeId(0), node(Role::Window, &[1])),
                (NodeId(1), node(Role::TextInput, &[2])),
                (NodeId(2), node(Role::TextInput, &[])),
            ],
            tree: None,
            tree_id: TreeId::ROOT,
            focus: NodeId(2),
        };
        collapse_text_proxies(&mut update);
        assert_eq!(update.nodes.len(), 3);
        assert_eq!(update.focus, NodeId(2));
    }
}
