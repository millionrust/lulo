//! Accessibility support, provided by [AccessKit][accesskit].
//!
//! There are user-facing guide-level docs [here](crate::_accessibility).
//!
//! ## Architecture
//!
//! ```text
//!                              ┌────────────────────────────────┐   ┌─────────────────────┐
//!                           ┌─▶│ AccessKit Adapter (MacOS)      │◀─▶│ MacOS System APIs   │
//!                           │  └────────────────────────────────┘   └─────────────────────┘
//!                           │
//! ┌──────┐   ┌───────────┐  │  ┌────────────────────────────────┐   ┌─────────────────────┐
//! │ GPUI │◀─▶│ AccessKit │◀─┼─▶│ AccessKit Adapter (Windows)    │◀─▶│ Windows System APIs │
//! └──────┘   └───────────┘  │  └────────────────────────────────┘   └─────────────────────┘
//!                           │
//!                           │  ┌────────────────────────────────┐   ┌─────────────────────┐
//!                           └─▶│ AccessKit Adapter (Linux)      │◀─▶│ dbus                │
//!                              └────────────────────────────────┘   └─────────────────────┘
//! ```
//!
//! In order for GPUI apps to be usable for people using assistive technology,
//! we must do a few things:
//! - Inform the system when the UI changes meaningfully. This includes:
//!   - Reporting new/removed/changed UI elements
//!   - *Not* reporting irrelevant UI changes, e.g. an invisible `div()` being
//!     added.
//!   - Reporting the appearance and capabilities of each UI element. For example:
//!     - What does this piece of text say?
//!     - How far along is this progress bar?
//!     - Can this node be focused?
//!     - Can this node have a value directly assigned? (e.g. a slider)
//! - Allowing the system to interact with the UI by dispatching actions to
//!   nodes. Note that AccessKit has its own [`Action`] type, which is not the
//!   [`crate::Action`] trait.
//! - Activate and deactivate accessibility features when requested by the
//!   system.
//!
//! Activating and deactivating at the right time is trivial, so I won't go into
//! detail here. The other two are almost orthogonal in implementation.
//!
//! The state for both lives in the [`A11y`] struct in this module.
//!
//! ### Reporting UI changes
//!
//! Every frame, we build a [`TreeUpdate`] and send it to the platform-specific
//! adapter. A [`TreeUpdate`] is a representation of a subset of the UI tree.
//! When the adapter receives the update, it diffs it against the previous
//! update, and calls platform-specific APIs to inform screen readers about the
//! changes. Nodes may have been created, destroyed, or updated.
//!
//! Each node has an ID, and this ID *should* be stable across frames. If a
//! node's ID changes, then, from AccessKit's point of view, it is a different
//! node.
//!
//! We derive the node ID from the [`GlobalElementId`] in
//! [`GlobalElementId::accesskit_node_id`]. Nodes without [`GlobalElementId`]s
//! cannot produce an AccessKit [`NodeId`], and so are not included in the
//! accessibility tree. We try to warn when using accessibility APIs on
//! [`div()`] without setting an ID.
//!
//! This all happens in [`Drawable::prepaint`]. The [`A11y`] struct maintains a
//! stack of nodes during prepainting, which we can use to calculate the
//! [`NodeId`]s, and record parent-child relationships. Once all [`Element`]s in
//! a frame have been prepainted, we send the resulting [`TreeUpdate`] object to
//! the adapter and the screen reader can announce the changes.
//!
//! #### Synthetic children
//!
//! Additionally, some nodes can register "synthetic children" using
//! [`Element::a11y_synthetic_children`]. Normally, one accesskit node is pushed
//! for every [`Element`] with a role and id. However, sometimes a single
//! element may want to produce many accesskit nodes. These extra nodes are
//! referred to as "synthetic children" of the element providing a non-default
//! [`Element::a11y_synthetic_children`] implementation.
//!
//! The user is provided a builder-style API using [`A11ySubtreeBuilder`], which
//! allows them to create push nodes that are children of the current node, as
//! well as modify the current node itself.
//!
//! GPUI calls this callback *after* prepainting (and just before popping the
//! corresponding element), since this step may need prepaint information to be
//! available. In the future, we may want to add prepaint information more
//! generally to [`Element::write_a11y_info`], but for now that's not necessary.
//!
//! ### Responding to actions
//!
//! On adapter creation, we provide a callback to the adapter, which can be used
//! to dispatch actions. This callback forwards to [`A11y::action_listeners`], a
//! mapping from [`NodeId`]s to action handlers (basically just `Box<dyn
//! Fn()>`).
//!
//! This is populated in:
//! - [`Window::on_a11y_action`], which is called by:
//! - [`Interactivity::paint`], which is called by:
//! - [`StatefulInteractiveElement::on_a11y_action`], which is a public-facing API
//!
//! These are cleared at the start of a frame, and re-populated during painting.
//!
//! [`NodeId`]: accesskit::NodeId

use crate::*;

use crate::{App, Bounds, FocusId, Pixels, SharedString, Window};
use accesskit::{Action, NodeId, TreeUpdate};
use collections::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The fixed AccessKit node ID used for the root of every window's a11y tree.
pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

/// A listener for an accessibility action on a specific node.
pub(crate) type A11yActionListener =
    Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static>;

/// Per-window accessibility state.
///
/// Manages the AccessKit tree that is built each frame and the mappings
/// needed to dispatch incoming action requests back to the right elements.
pub(crate) struct A11y {
    /// Whether accessibility has been [forcibly disabled] for this window.
    ///
    /// [forcibly disabled]: crate::Application::new_inaccessible
    force_disabled: bool,
    /// Whether a11y features have been requested by the system.
    ///
    /// Updated by AccessKit using callbacks provided to the adapter. Can change
    /// halfway through a frame.
    active_flag: Arc<AtomicBool>,
    /// Whether a11y features are active for *this specific frame*.
    ///
    /// At the start of each frame, we load [`Self::active_flag`] (using
    /// [`Self::sync_active_flag`]) and use this to determine whether we
    /// should construct a [`TreeUpdate`] for this frame. It's important that
    /// this value is stable within a frame, because the builder API exposed by
    /// this type maintains a stack of nodes and each must be pushed and popped
    /// exactly once.
    ///
    /// At the end of the frame, we re-call [`Self::sync_active_flag`] to
    /// determine whether we should actually send the finished [`TreeUpdate`].
    active_this_frame: bool,
    pub(crate) nodes: A11yNodeBuilder,
    pub(crate) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(crate) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    /// The window's title, used to label the root node so assistive
    /// technology can tell windows apart.
    window_title: Option<SharedString>,
}

impl A11y {
    pub(crate) fn new(
        active_flag: Arc<AtomicBool>,
        force_disabled: bool,
        window_title: Option<SharedString>,
    ) -> Self {
        Self {
            force_disabled,
            active_flag,
            active_this_frame: false,
            nodes: A11yNodeBuilder::new(),
            focus_ids: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            action_listeners: FxHashMap::default(),
            window_title,
        }
    }

    pub(crate) fn set_window_title(&mut self, title: impl Into<SharedString>) {
        self.window_title = Some(title.into());
    }

    /// Ensures that [`Self::is_active`] returns up to date information.
    ///
    /// See the docs for [`Self::active_flag`] and [`Self::active_this_frame`]
    /// for more commentary.
    pub(crate) fn sync_active_flag(&mut self) {
        self.active_this_frame = !self.force_disabled && self.active_flag.load(Ordering::SeqCst);
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_this_frame
    }

    pub(crate) fn set_focusable(&mut self, node_id: NodeId, focus_id: FocusId) {
        self.focus_ids.insert(node_id, focus_id);
    }

    /// Report `node_id` as the currently-focused node, if it is present in the
    /// tree.
    ///
    /// Must only be called once per frame.
    pub(crate) fn set_focus(&mut self, node_id: NodeId) {
        // A focused node must have been registered as focusable this frame.
        if !self.focus_ids.contains_key(&node_id) {
            if cfg!(debug_assertions) {
                panic!("set_focus called for a node that was not registered with set_focusable");
            } else {
                log::warn!(
                    "a11y: set_focus called for a node that was not registered with \
                     set_focusable ({node_id:?})"
                );
            }
        }
        if self.nodes.has_node(node_id) {
            self.nodes.set_focus(node_id);
        }
    }

    pub(crate) fn set_active_descendant(&mut self, node_id: NodeId) {
        // The active descendant must be a descendant of the focused container,
        // not the focused node itself.
        if self.nodes.node_is_focused(node_id) {
            if cfg!(debug_assertions) {
                panic!("set_active_descendant called on the focused node");
            } else {
                log::warn!("a11y: set_active_descendant called on the focused node ({node_id:?})");
            }
            return;
        }
        if self.nodes.has_node(node_id) && self.nodes.focus_is_ancestor_of_current() {
            self.nodes.set_active_descendant(node_id);
        }
    }

    /// Clear per-frame state and push the root node to start a new frame.
    pub(crate) fn begin_frame(&mut self) {
        self.focus_ids.clear();
        self.node_bounds.clear();
        self.action_listeners.clear();
        self.nodes.begin_frame(self.window_title.as_ref());
    }

    /// rmac: give the root (window) node the window's size, in physical
    /// pixels. Wayland has no window position to report, so without node
    /// bounds a layer-shell surface (Control Centre, Spotlight) had no
    /// AT-SPI extents at all and a screen reader could not place it.
    pub(crate) fn set_root_bounds(&mut self, bounds: accesskit::Rect) {
        if let Some(root) = self.nodes.nodes_stack.first_mut() {
            root.set_bounds(bounds);
        }
    }

    /// Finalize the tree and produce a [`TreeUpdate`] for the platform adapter.
    pub(crate) fn end_frame(&mut self) -> TreeUpdate {
        self.nodes.finalize()
    }
}

/// Builder API for synthetic children. See the docs for
/// [`Element::a11y_synthetic_children`].
pub struct A11ySubtreeBuilder<'a> {
    parent_id: NodeId,
    nodes: &'a mut A11yNodeBuilder,
}

impl<'a> A11ySubtreeBuilder<'a> {
    pub(crate) fn new(parent_id: NodeId, nodes: &'a mut A11yNodeBuilder) -> Self {
        Self { parent_id, nodes }
    }

    /// Derive a [`NodeId`] for a synthetic child.
    ///
    /// The generated ID is based on the hash of `key`, as well as the parent's
    /// ID. This means that `key`s must be unique within the same
    /// [`Element::a11y_synthetic_children`] call, but may be duplicated across
    /// different calls.
    pub fn synthetic_node_id(&self, key: impl Hash) -> NodeId {
        let mut hasher = std::hash::DefaultHasher::default();
        self.parent_id.0.hash(&mut hasher);
        key.hash(&mut hasher);
        NodeId(hasher.finish())
    }

    /// Append a synthetic leaf node as a child of this element's node.
    ///
    /// Returns `false` if a node with this id is already present in the tree,
    /// in which case the node is discarded.
    pub fn push_child(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        self.nodes.push_leaf(id, node)
    }

    /// A mutable reference to the parent node.
    pub fn parent_node(&mut self) -> &mut accesskit::Node {
        self.nodes
            .current_node_mut()
            .expect("A11ySubtreeBuilder exists only while its element's node is on the stack")
    }
}

pub(crate) struct A11yNodeBuilder {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    /// This is the exact type required by accesskit, so we can't just make it a
    /// `HashMap<NodeId, Node>` to remove the need for `seen_ids`
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    seen_ids: FxHashSet<NodeId>,
    /// The node that GPUI considers focused. Note that this may be different to
    /// what is reported to accesskit - see [`Self::active_descendant`]
    focus: Option<NodeId>,
    /// If a node calls `.aria_active_descendant()`, AND an ancestor is focused,
    /// override it as the focused node. This supports the "active descendant"
    /// pattern, which allows a focused container to act as if a descendant is
    /// focused.
    active_descendant: Option<NodeId>,
    /// rmac: how many times each (parent, text) pair has been published this
    /// frame, so repeated identical text under one parent gets distinct,
    /// stable node ids. See [`Self::push_text_leaf`].
    text_occurrences: FxHashMap<u64, u32>,
}

impl A11yNodeBuilder {
    fn new() -> Self {
        Self {
            ids_stack: SmallVec::new(),
            nodes_stack: SmallVec::new(),
            all_nodes: Vec::new(),
            seen_ids: FxHashSet::default(),
            focus: None,
            active_descendant: None,
            text_occurrences: FxHashMap::default(),
        }
    }

    /// rmac: publish a plain text element (a `SharedString`, `&str` or
    /// `StyledText` child) as a static-text node, the way AppKit exposes every
    /// drawn `NSTextField` label as `AXStaticText`. Without this a settings
    /// row's title, a toolbar caption or a column header was invisible to
    /// assistive technology unless its author built a labelled node by hand.
    ///
    /// The node becomes a child of the nearest enclosing accessible element.
    /// Its id hashes that parent, the text and the occurrence count, so it is
    /// stable across frames while the text stays. Text inside a node that
    /// already speaks for it (a label, a text field, a named button or menu
    /// item) is not repeated; an unnamed button takes the text as its name.
    pub(crate) fn push_text_leaf(&mut self, text: &str, bounds: accesskit::Rect) {
        let text = text.trim();
        if text.is_empty() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        let Some(&parent_id) = self.ids_stack.last() else {
            return;
        };
        let Some(parent) = self.nodes_stack.last_mut() else {
            return;
        };
        match text_placement(parent, text) {
            TextPlacement::Skip => return,
            TextPlacement::NameParent => {
                let name = match parent.label() {
                    Some(existing) => format!("{existing} {text}"),
                    None => text.to_owned(),
                };
                parent.set_label(name);
                return;
            }
            TextPlacement::Child => {}
        }
        let mut hasher = std::hash::DefaultHasher::default();
        parent_id.0.hash(&mut hasher);
        text.hash(&mut hasher);
        let key = hasher.finish();
        let occurrence = self.text_occurrences.entry(key).or_insert(0);
        let index = *occurrence;
        *occurrence += 1;
        let mut hasher = std::hash::DefaultHasher::default();
        TEXT_NODE_SALT.hash(&mut hasher);
        key.hash(&mut hasher);
        index.hash(&mut hasher);
        let id = NodeId(hasher.finish());
        if id == ROOT_NODE_ID || self.seen_ids.contains(&id) {
            return;
        }
        let mut node = accesskit::Node::new(accesskit::Role::Label);
        node.set_value(text.to_owned());
        node.set_bounds(bounds);
        self.push_leaf(id, node);
    }

    #[must_use]
    fn can_push(&mut self, id: NodeId) -> bool {
        debug_assert!(!self.ids_stack.is_empty(), "node pushed before push_root");

        if !self.seen_ids.insert(id) {
            debug_assert!(
                false,
                "Duplicate a11y node id: {id:?}. In a release build, this node would be silently discarded from the a11y tree."
            );
            return false;
        }

        true
    }

    /// Push a new node onto the stack. It becomes a child of the current
    /// top-of-stack node.
    ///
    /// Returns `true` if the node was successfully pushed.
    pub(crate) fn push(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.ids_stack.push(id);
        self.nodes_stack.push(node);
        true
    }

    /// Add a leaf node as a child of the current top-of-stack node, without
    /// pushing it onto the stack. Semantically equivalent to a [`Self::push`]
    /// followed by a [`Self::pop`].
    ///
    /// Returns `true` if the node was successfully pushed.
    pub(crate) fn push_leaf(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.all_nodes.push((id, node));
        true
    }

    pub(crate) fn current_node_mut(&mut self) -> Option<&mut accesskit::Node> {
        self.nodes_stack.last_mut()
    }

    /// Pop the current node off the stack and finalize it into the all_nodes
    /// list.
    pub(crate) fn pop(&mut self) {
        debug_assert!(self.ids_stack.len() > 1, "pop would remove the root node");

        if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
            self.all_nodes.push((id, node));
        }
    }

    /// Push the root node to start a new frame.
    fn begin_frame(&mut self, window_title: Option<&SharedString>) {
        self.all_nodes.clear();
        self.ids_stack.clear();
        self.nodes_stack.clear();
        self.seen_ids.clear();
        let mut root_node = accesskit::Node::new(accesskit::Role::Window);
        if let Some(title) = window_title {
            root_node.set_label(title.to_string());
        }

        self.ids_stack.push(ROOT_NODE_ID);
        self.nodes_stack.push(root_node);
        self.focus = None;
        self.active_descendant = None;
        self.text_occurrences.clear();
    }

    /// Returns whether a node with the given ID has been pushed in this frame.
    pub(crate) fn has_node(&self, id: NodeId) -> bool {
        id == ROOT_NODE_ID || self.seen_ids.contains(&id)
    }

    /// Returns whether `id` is the node currently reported as focused.
    pub(crate) fn node_is_focused(&self, id: NodeId) -> bool {
        self.focus == Some(id)
    }

    pub(crate) fn focus_is_ancestor_of_current(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };

        // The current node is on top of the stack; everything below it is an
        // ancestor.
        let ancestor_count = self.ids_stack.len().saturating_sub(1);
        self.ids_stack[..ancestor_count].contains(&focus)
    }

    pub(crate) fn set_active_descendant(&mut self, id: NodeId) {
        if self
            .active_descendant
            .is_some_and(|existing| existing != id)
        {
            if cfg!(debug_assertions) {
                panic!("active descendant claimed by multiple nodes in one frame");
            } else {
                log::warn!(
                    "a11y: multiple nodes claimed the active descendant this frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.active_descendant = Some(id);
    }

    pub(crate) fn set_focus(&mut self, id: NodeId) {
        if self.focus.is_some() {
            if cfg!(debug_assertions) {
                panic!("set_focus called more than once in a single frame");
            } else {
                log::warn!(
                    "a11y: set_focus called more than once in a single frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.focus = Some(id);
    }

    fn finalize(&mut self) -> TreeUpdate {
        // Stack should contain only the root node
        debug_assert_eq!(self.ids_stack.len(), 1);
        debug_assert_eq!(self.ids_stack[0], ROOT_NODE_ID);

        if self.ids_stack.len() != 1 {
            log::error!(
                "a11y: Stack imbalance at end of frame: expected 1 (root), got {}. \
                 Some elements may have pushed without popping.",
                self.ids_stack.len()
            );
        }

        // Pop remaining nodes (should just be the root).
        while !self.ids_stack.is_empty() {
            if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
                self.all_nodes.push((id, node));
            }
        }

        let focus = match self.active_descendant {
            Some(id) if self.has_node(id) => id,
            Some(id) => {
                if cfg!(debug_assertions) {
                    panic!("active_descendant set to {id:?}, which is not in the tree");
                } else {
                    log::warn!("active_descendant set to {id:?}, which is not in the tree");
                    self.focus.unwrap_or(ROOT_NODE_ID)
                }
            }

            _ => self.focus.unwrap_or(ROOT_NODE_ID),
        };

        let mut nodes = std::mem::take(&mut self.all_nodes);
        label_controls_from_row_text(&mut nodes);
        let update = TreeUpdate {
            nodes,
            tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
            tree_id: accesskit::TreeId::ROOT,
            focus,
        };

        Self::repair_tree_update(update)
    }

    /// Accesskit panics on invalid [`TreeUpdate`]s. This function defensively
    /// checks invariants that accesskit panics on, and tries to fix them.
    fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
        let node_ids: FxHashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();

        // Focus must point to a node in the tree.
        if !node_ids.contains(&update.focus) {
            log::error!(
                "a11y: Focused node {:?} is not in the tree ({} nodes). \
                 Falling back to root. This is a bug in the a11y tree builder.",
                update.focus,
                update.nodes.len()
            );
            update.focus = ROOT_NODE_ID;
        }

        // Every child reference must point to a node in the update.
        for (id, node) in &mut update.nodes {
            let has_invalid_child = node
                .children()
                .iter()
                .any(|child_id| !node_ids.contains(child_id));
            if has_invalid_child {
                let children = node.children();
                let invalid_count = children
                    .iter()
                    .filter(|child_id| !node_ids.contains(child_id))
                    .count();
                log::error!(
                    "a11y: Node {:?} references {} children not present in the tree. \
                     Stripping invalid child references.",
                    id,
                    invalid_count
                );
                let valid: Vec<NodeId> = children
                    .iter()
                    .copied()
                    .filter(|child_id| node_ids.contains(child_id))
                    .collect();
                node.set_children(valid);
            }
        }

        update
    }
}

/// Salt for static-text node ids, keeping them apart from element ids.
const TEXT_NODE_SALT: u64 = 0x726d_6163_7465_7874;

/// Where a plain text element's string goes in the tree, given the
/// accessible node it is drawn inside.
#[derive(Debug, PartialEq, Eq)]
enum TextPlacement {
    /// The parent already presents this text (or presents text itself).
    Skip,
    /// The parent is an unnamed control: the text is its name, the way an
    /// `NSButton`'s title is its `AXTitle`.
    NameParent,
    /// A static-text child of the parent.
    Child,
}

fn text_placement(parent: &accesskit::Node, text: &str) -> TextPlacement {
    use accesskit::Role;
    let role = parent.role();
    if matches!(
        role,
        Role::Label
            | Role::TextRun
            | Role::Terminal
            | Role::Document
            | Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::PasswordInput
            | Role::NumberInput
            | Role::EmailInput
            | Role::UrlInput
            | Role::PhoneNumberInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::TimeInput
            | Role::Image
    ) {
        return TextPlacement::Skip;
    }
    // A container named after its title (a slider group, a dialog) still
    // shows that title as text; a row or control named after its text
    // does not repeat it.
    let container = matches!(
        role,
        Role::Group
            | Role::RadioGroup
            | Role::Toolbar
            | Role::Window
            | Role::Dialog
            | Role::AlertDialog
            | Role::Pane
            | Role::Region
            | Role::Section
            | Role::Form
            | Role::TabPanel
    );
    if !container
        && (parent.label().is_some_and(|label| label.trim() == text)
            || parent.value().is_some_and(|value| value.trim() == text))
    {
        return TextPlacement::Skip;
    }
    let names_itself = matches!(
        role,
        Role::Button
            | Role::DefaultButton
            | Role::Link
            | Role::Tab
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::CheckBox
            | Role::Switch
            | Role::RadioButton
            | Role::ComboBox
            | Role::ListBoxOption
    );
    if !names_itself {
        TextPlacement::Child
    } else if parent.label().is_none()
        && parent.value().is_none()
        && text.chars().any(char::is_alphanumeric)
    {
        // A glyph (✓, ×, ⌄) is not a name; the row's title names it.
        TextPlacement::NameParent
    } else {
        // A named control's other text (a shortcut, a count, a caption
        // inside it) is part of the control, not a separate label.
        TextPlacement::Skip
    }
}

/// Controls a settings row's title names, as `AXTitleUIElement` does on
/// macOS.
fn takes_row_label(role: accesskit::Role) -> bool {
    use accesskit::Role;
    matches!(
        role,
        Role::Switch
            | Role::CheckBox
            | Role::Slider
            | Role::SpinButton
            | Role::ComboBox
            | Role::ProgressIndicator
            | Role::Meter
            | Role::ColorWell
    )
}

/// Whether `node` still needs its row's title as its name. A pop-up named
/// only after its current value (rmac-ui's `PopUpButton`) counts as unnamed:
/// on macOS the value is `AXValue` and the row title names it.
fn needs_row_label(node: &accesskit::Node) -> bool {
    takes_row_label(node.role())
        && match node.label() {
            None => true,
            Some(label) => node.role() == accesskit::Role::ComboBox && node.value() == Some(label),
        }
}

/// Controls that end a row: the next control's title comes after them.
fn ends_row(node: &accesskit::Node) -> bool {
    use accesskit::Role;
    takes_row_label(node.role())
        || matches!(
            node.role(),
            Role::Button
                | Role::DefaultButton
                | Role::RadioButton
                | Role::Link
                | Role::TextInput
                | Role::SearchInput
        )
}

/// rmac: name each unnamed toggle, slider or pop-up after the text of the
/// row it sits in, so a screen reader says "Play sound on startup, switch,
/// on" rather than "switch, on". A settings row draws its title as plain
/// text to the left of the control (a checkbox's caption to its right);
/// both are siblings of the control under its nearest accessible ancestor
/// (see [`A11yNodeBuilder::push_text_leaf`]). The first static text after
/// the previous control and on the control's row wins; failing that, the
/// first one on its right; failing that, an enclosing named group's name.
/// Runs once per built frame over the nodes already built.
fn label_controls_from_row_text(nodes: &mut [(NodeId, accesskit::Node)]) {
    use accesskit::Role;
    let has_candidate = nodes.iter().any(|(_, node)| needs_row_label(node));
    if !has_candidate {
        return;
    }
    let index: FxHashMap<NodeId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(position, (id, _))| (*id, position))
        .collect();
    let static_text = |position: usize| -> Option<(&str, accesskit::Rect)> {
        let node = &nodes[position].1;
        if node.role() != Role::Label {
            return None;
        }
        Some((node.value()?, node.bounds()?))
    };
    let mut names: Vec<(usize, String)> = Vec::new();
    for (_, parent) in nodes.iter() {
        if parent.children().is_empty() {
            continue;
        }
        let children: Vec<usize> = parent
            .children()
            .iter()
            .filter_map(|child| index.get(child).copied())
            .collect();
        let group_name = if matches!(parent.role(), Role::Group | Role::RadioGroup) {
            parent.label()
        } else {
            None
        };
        let mut since = 0;
        for (slot, &position) in children.iter().enumerate() {
            let node = &nodes[position].1;
            if !ends_row(node) {
                continue;
            }
            let row_start = since;
            since = slot + 1;
            if !needs_row_label(node) {
                continue;
            }
            let Some(bounds) = node.bounds() else {
                continue;
            };
            let slack = bounds.height().max(16.0);
            // A slider's title may sit just above it, left-aligned with it,
            // with the "Slow"/"Fast" ends beside the track; that title wins
            // over a text beside it. Otherwise the first text beside it on
            // its line does.
            let candidates = || {
                children[row_start..slot]
                    .iter()
                    .filter_map(|&p| static_text(p))
            };
            let above = (node.role() == Role::Slider)
                .then(|| {
                    candidates()
                        .filter(|(_, text)| {
                            text.y1 <= bounds.y0 + 1.0
                                && bounds.y0 - text.y1 <= 0.5 * slack
                                && text.x0 <= bounds.x0 + 1.0
                        })
                        .last()
                })
                .flatten();
            let before = above.or_else(|| {
                candidates().find(|(_, text)| {
                    let center = (text.y0 + text.y1) / 2.0;
                    text.x1 <= bounds.x0 + 1.0
                        && center >= bounds.y0 - slack
                        && center <= bounds.y1 + slack
                })
            });
            let after = || {
                children[slot + 1..]
                    .iter()
                    .take_while(|&&p| !ends_row(&nodes[p].1))
                    .filter_map(|&p| static_text(p))
                    .find(|(_, text)| {
                        let center = (text.y0 + text.y1) / 2.0;
                        text.x0 >= bounds.x1 - 1.0 && center >= bounds.y0 && center <= bounds.y1
                    })
            };
            if let Some(name) = before.or_else(after).map(|(text, _)| text).or(group_name) {
                names.push((position, name.to_owned()));
            }
        }
    }
    for (position, name) in names {
        nodes[position].1.set_label(name);
    }
}

#[cfg(test)]
mod row_text_tests {
    use super::{TextPlacement, label_controls_from_row_text, text_placement};
    use accesskit::{Node, NodeId, Rect, Role};

    fn text(value: &str, rect: Rect) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(value);
        node.set_bounds(rect);
        node
    }

    fn control(role: Role, rect: Rect) -> Node {
        let mut node = Node::new(role);
        node.set_bounds(rect);
        node
    }

    fn label_of(nodes: &[(NodeId, Node)], id: u64) -> Option<String> {
        nodes
            .iter()
            .find(|(node_id, _)| *node_id == NodeId(id))
            .and_then(|(_, node)| node.label().map(str::to_owned))
    }

    #[test]
    fn a_switch_takes_its_row_title_not_the_next_rows() {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4), NodeId(5)]);
        let mut nodes = vec![
            (NodeId(0), root),
            // Title and description on the left, switch centred on the right.
            (
                NodeId(1),
                text(
                    "Play sound on startup",
                    Rect::new(20.0, 100.0, 200.0, 116.0),
                ),
            ),
            (
                NodeId(2),
                text("Plays a chime", Rect::new(20.0, 118.0, 140.0, 132.0)),
            ),
            (
                NodeId(3),
                control(Role::Switch, Rect::new(400.0, 108.0, 436.0, 124.0)),
            ),
            (
                NodeId(4),
                text("Alert volume", Rect::new(20.0, 160.0, 120.0, 176.0)),
            ),
            (
                NodeId(5),
                control(Role::Slider, Rect::new(200.0, 156.0, 392.0, 180.0)),
            ),
        ];
        label_controls_from_row_text(&mut nodes);
        assert_eq!(
            label_of(&nodes, 3).as_deref(),
            Some("Play sound on startup")
        );
        assert_eq!(label_of(&nodes, 5).as_deref(), Some("Alert volume"));
    }

    #[test]
    fn a_checkbox_takes_its_caption_on_the_right_and_named_controls_keep_theirs() {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4)]);
        let mut named = control(Role::ComboBox, Rect::new(300.0, 10.0, 380.0, 34.0));
        named.set_label("Low Power");
        let mut nodes = vec![
            (NodeId(0), root),
            (
                NodeId(1),
                text("Energy Mode", Rect::new(20.0, 14.0, 120.0, 30.0)),
            ),
            (NodeId(2), named),
            (
                NodeId(3),
                control(Role::CheckBox, Rect::new(20.0, 60.0, 36.0, 76.0)),
            ),
            (
                NodeId(4),
                text("Show in menu bar", Rect::new(42.0, 60.0, 160.0, 76.0)),
            ),
        ];
        label_controls_from_row_text(&mut nodes);
        assert_eq!(label_of(&nodes, 2).as_deref(), Some("Low Power"));
        assert_eq!(label_of(&nodes, 3).as_deref(), Some("Show in menu bar"));
    }

    #[test]
    fn a_value_named_pop_up_and_a_slider_take_their_titles() {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1), NodeId(2), NodeId(3), NodeId(4), NodeId(5)]);
        let mut pop_up = control(Role::ComboBox, Rect::new(300.0, 10.0, 380.0, 34.0));
        pop_up.set_label("Bottom");
        pop_up.set_value("Bottom");
        let mut nodes = vec![
            (NodeId(0), root),
            (
                NodeId(1),
                text("Position on screen", Rect::new(20.0, 14.0, 160.0, 30.0)),
            ),
            (NodeId(2), pop_up),
            (
                NodeId(3),
                text("Key repeat rate", Rect::new(20.0, 50.0, 140.0, 66.0)),
            ),
            (NodeId(4), text("Slow", Rect::new(20.0, 74.0, 50.0, 90.0))),
            (
                NodeId(5),
                control(Role::Slider, Rect::new(20.0, 70.0, 300.0, 94.0)),
            ),
        ];
        label_controls_from_row_text(&mut nodes);
        assert_eq!(label_of(&nodes, 2).as_deref(), Some("Position on screen"));
        assert_eq!(label_of(&nodes, 5).as_deref(), Some("Key repeat rate"));
    }

    #[test]
    fn text_placement_names_unnamed_buttons_and_skips_repeats() {
        let button = Node::new(Role::Button);
        assert_eq!(text_placement(&button, "Share"), TextPlacement::NameParent);
        let mut named = Node::new(Role::Button);
        named.set_label("Refresh");
        assert_eq!(text_placement(&named, "Refresh"), TextPlacement::Skip);
        assert_eq!(text_placement(&named, "⌘R"), TextPlacement::Skip);
        let group = Node::new(Role::Group);
        assert_eq!(
            text_placement(&group, "Battery Health"),
            TextPlacement::Child
        );
        let field = Node::new(Role::TextInput);
        assert_eq!(text_placement(&field, "typed"), TextPlacement::Skip);
        let check = Node::new(Role::CheckBox);
        assert_eq!(text_placement(&check, "✓"), TextPlacement::Skip);
        // A slider group named after its title still shows the title; a
        // row named after its text does not repeat it.
        let mut titled = Node::new(Role::Group);
        titled.set_label("Tracking speed");
        assert_eq!(
            text_placement(&titled, "Tracking speed"),
            TextPlacement::Child
        );
        let mut row = Node::new(Role::ListItem);
        row.set_label("Wi-Fi");
        assert_eq!(text_placement(&row, "Wi-Fi"), TextPlacement::Skip);
    }
}

#[cfg(test)]
mod tests {
    // Import specific items rather than glob-importing `super`, which would pull
    // in gpui's own `test` attribute macro and shadow the standard one.
    use super::{A11y, A11yNodeBuilder, ROOT_NODE_ID};
    use crate::FocusId;
    use accesskit::{NodeId, Role};
    use std::sync::{Arc, atomic::AtomicBool};

    fn test_node() -> accesskit::Node {
        accesskit::Node::new(Role::GenericContainer)
    }

    fn new_builder() -> A11yNodeBuilder {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame(None);
        builder
    }

    fn new_a11y() -> A11y {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, None);
        a11y.begin_frame();
        a11y
    }

    #[test]
    fn active_descendant_honored_when_container_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(item, test_node()));

        // The item is on top of the stack; the focused container is its
        // ancestor, so the claim is honored.
        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop(); // item
        builder.pop(); // container
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_honored_for_deep_descendant() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let group = NodeId(2);
        let item = NodeId(3);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(group, test_node()));
        assert!(builder.push(item, test_node()));

        // The item is a grandchild of the focused container; depth doesn't
        // matter, the focused ancestor is still on the stack.
        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop(); // item
        builder.pop(); // group
        builder.pop(); // container
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_ignored_when_focus_in_other_subtree() {
        let mut builder = new_builder();
        let focused_container = NodeId(1);
        let focused_leaf = NodeId(2);
        let other_container = NodeId(3);
        let other_item = NodeId(4);

        // First subtree holds real focus.
        assert!(builder.push(focused_container, test_node()));
        assert!(builder.push(focused_leaf, test_node()));
        builder.set_focus(focused_leaf);
        builder.pop(); // focused_leaf
        builder.pop(); // focused_container

        // Second subtree: its item would claim the active descendant, but the
        // focus is not on any of its ancestors, so the gate rejects it.
        assert!(builder.push(other_container, test_node()));
        assert!(builder.push(other_item, test_node()));
        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop(); // other_item
        builder.pop(); // other_container

        let update = builder.finalize();
        assert_eq!(update.focus, focused_leaf);
    }

    #[test]
    fn active_descendant_ignored_when_nothing_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        assert!(builder.push(item, test_node()));

        // Nothing is focused (focus defaults to the root window node), so the
        // gate rejects the claim.
        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, ROOT_NODE_ID);
    }

    #[test]
    fn regular_focus_used_when_no_active_descendant() {
        let mut builder = new_builder();
        let focused = NodeId(1);

        assert!(builder.push(focused, test_node()));
        builder.set_focus(focused);
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, focused);
    }

    #[test]
    fn focus_is_ancestor_excludes_self_and_non_ancestors() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);

        // With the focused container itself on top, it is not its own (strict)
        // ancestor, so the gate is false.
        assert!(!builder.focus_is_ancestor_of_current());

        assert!(builder.push(item, test_node()));
        // Now the focused container is a strict ancestor of the item on top.
        assert!(builder.focus_is_ancestor_of_current());

        builder.pop();
        builder.pop();
    }

    // The double-claim guard panics only in debug builds; in release it falls
    // back to last-wins with a warning.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn multiple_active_descendant_claims_panic_in_debug() {
        let mut builder = new_builder();
        builder.set_active_descendant(NodeId(1));
        builder.set_active_descendant(NodeId(2));
    }

    // Setting focus twice in one frame means two elements both claimed window
    // focus; that panics in debug and falls back to last-wins in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "set_focus called more than once")
    )]
    fn setting_focus_twice_panics_in_debug() {
        let mut builder = new_builder();
        builder.set_focus(NodeId(1));
        builder.set_focus(NodeId(2));
    }

    // Focusing a node that was never registered as focusable is a bug: panic in
    // debug, warn in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was not registered with set_focusable")
    )]
    fn set_focus_without_set_focusable() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        // set_focusable was never called for `node`.
        a11y.set_focus(node);
    }

    // The focused node cannot also be its own active descendant: panic in
    // debug, warn in release.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "on the focused node"))]
    fn set_active_descendant_on_focused_node() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        a11y.set_focusable(node, FocusId::default());
        a11y.set_focus(node);
        a11y.set_active_descendant(node);
    }

    // Two sibling children of a focused container both claim the active
    // descendant (both pass the focus gate). The second claim is a bug: panic
    // in debug, last-wins + warn in release.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn two_siblings_claiming_active_descendant() {
        let mut a11y = new_a11y();
        let container = NodeId(1);
        let first = NodeId(2);
        let second = NodeId(3);

        assert!(a11y.nodes.push(container, test_node()));
        a11y.set_focusable(container, FocusId::default());
        a11y.set_focus(container);

        assert!(a11y.nodes.push(first, test_node()));
        a11y.set_active_descendant(first);
        a11y.nodes.pop(); // first

        assert!(a11y.nodes.push(second, test_node()));
        a11y.set_active_descendant(second);
        a11y.nodes.pop(); // second

        a11y.nodes.pop(); // container
    }

    // Node A is focused; node C (a child of the unfocused node B) claims the
    // active descendant. The final tree must still report A as focused.
    #[test]
    fn active_descendant_in_unfocused_subtree_keeps_real_focus() {
        let mut a11y = new_a11y();
        let a = NodeId(1);
        let b = NodeId(2);
        let c = NodeId(3);

        assert!(a11y.nodes.push(a, test_node()));
        a11y.set_focusable(a, FocusId::default());
        a11y.set_focus(a);
        a11y.nodes.pop(); // a

        assert!(a11y.nodes.push(b, test_node()));
        assert!(a11y.nodes.push(c, test_node()));
        a11y.set_active_descendant(c);
        a11y.nodes.pop(); // c
        a11y.nodes.pop(); // b

        let update = a11y.end_frame();
        assert_eq!(update.focus, a);
    }
}
