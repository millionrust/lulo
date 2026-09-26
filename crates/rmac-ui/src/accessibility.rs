//! Text, caret and selection for assistive technology.
//!
//! AccessKit only gives a node an AT-SPI `Text` interface when the node has
//! `Role::TextRun` children: a `value` on a text input is not readable over
//! AT-SPI on its own (`accesskit_consumer::Node::supports_text_ranges`). This
//! module publishes text the way AccessKit expects it: one run per line (a
//! run is AccessKit's unit of line navigation), each with character and word
//! lengths, and the caret or selection as a `TextSelection` on the parent.
//!
//! [`AccessibleTextInput::accessible_text_input`] does that for a field backed
//! by an [`InputState`]. The field's own input element also reports a text
//! role and holds keyboard focus, so the node built here is marked as a proxy
//! and the Linux platform layer (`gpui_linux::linux::a11y`) folds that inner
//! node into it before the tree reaches AT-SPI. A screen reader then sees one
//! focused, named field with its text and caret.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    accesskit, A11ySubtreeBuilder, AccessibleAction, App, Div, Entity, Focusable as _,
    SharedString, Stateful, StatefulInteractiveElement as _,
};

use crate::InputState;

/// Marks a node whose single text-input descendant the platform layer folds
/// into it. Must match `gpui_linux::linux::a11y::TEXT_PROXY_CLASS`.
pub const TEXT_PROXY_CLASS: &str = "rmac-text-proxy";

/// Longest text published to assistive technology, in bytes. Longer text is
/// cut at a character boundary; a caret beyond the cut is not reported.
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 512 * 1024;

/// One line of published text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRunSpec {
    /// The line, including its trailing `\n` when it has one.
    pub text: String,
    /// UTF-8 length of each character. Characters are Unicode scalar values,
    /// so AT-SPI offsets (which count scalar values) equal character indices.
    pub character_lengths: Vec<u8>,
    /// Character index where each word starts; a word keeps the whitespace
    /// after it. AccessKit stores these as `u8`, so words starting past
    /// index 255 of a long line are not marked.
    pub word_starts: Vec<u8>,
    /// Offset of the line's first character in the whole text.
    pub start: usize,
}

impl TextRunSpec {
    pub fn len(&self) -> usize {
        self.character_lengths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.character_lengths.is_empty()
    }
}

/// Split `text` into AccessKit text runs, one per line. Empty text still gets
/// one empty run so the node keeps a `Text` interface and a caret position.
pub fn text_runs(text: &str) -> Vec<TextRunSpec> {
    let text = bounded(text);
    let mut runs = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let character_lengths: Vec<u8> = line.chars().map(|c| c.len_utf8() as u8).collect();
        let word_starts = word_starts(line);
        let len = character_lengths.len();
        runs.push(TextRunSpec {
            text: line.to_owned(),
            character_lengths,
            word_starts,
            start,
        });
        start += len;
    }
    // A trailing newline (or no text at all) leaves the caret on an empty
    // last line, which needs a run of its own.
    if text.is_empty() || text.ends_with('\n') {
        runs.push(TextRunSpec {
            text: String::new(),
            character_lengths: Vec::new(),
            word_starts: Vec::new(),
            start,
        });
    }
    runs
}

fn bounded(text: &str) -> &str {
    if text.len() <= MAX_ACCESSIBLE_TEXT_BYTES {
        return text;
    }
    let mut end = MAX_ACCESSIBLE_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Where each word starts: the first character, and every non-whitespace
/// character after whitespace, up to the last index a `u8` can hold.
fn word_starts(line: &str) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut previous_space = true;
    for (index, character) in line.chars().enumerate().take(256) {
        let space = character.is_whitespace();
        if index == 0 || (previous_space && !space) {
            starts.push(index as u8);
        }
        previous_space = space;
    }
    starts
}

/// The run and character index of a whole-text character `offset`. An
/// offset at a line break lands at the start of the next line, and one past
/// the end lands at the end of the last run.
pub fn run_position(runs: &[TextRunSpec], offset: usize) -> Option<(usize, usize)> {
    let last = runs.len().checked_sub(1)?;
    for (index, run) in runs.iter().enumerate() {
        if offset < run.start + run.len() || index == last {
            let character = offset.checked_sub(run.start)?;
            return (character <= run.len()).then_some((index, character));
        }
    }
    None
}

/// Push `text` as text runs under the builder's node and set its caret or
/// selection (`anchor`, `focus`, character offsets). Returns each run's node
/// id with its start offset, for mapping a `SetTextSelection` request back.
pub fn push_text_runs(
    builder: &mut A11ySubtreeBuilder,
    text: &str,
    selection: Option<(usize, usize)>,
) -> Vec<(accesskit::NodeId, usize)> {
    let runs = text_runs(text);
    let mut ids = Vec::with_capacity(runs.len());
    for (index, run) in runs.iter().enumerate() {
        let id = builder.synthetic_node_id(("text-run", index));
        let mut node = accesskit::Node::new(accesskit::Role::TextRun);
        node.set_value(run.text.clone());
        node.set_character_lengths(run.character_lengths.clone());
        node.set_word_starts(run.word_starts.clone());
        builder.push_child(id, node);
        ids.push((id, run.start));
    }
    let position = |offset: usize| {
        run_position(&runs, offset).map(|(index, character_index)| accesskit::TextPosition {
            node: ids[index].0,
            character_index,
        })
    };
    if let Some((anchor, focus)) = selection {
        if let (Some(anchor), Some(focus)) = (position(anchor), position(focus)) {
            builder
                .parent_node()
                .set_text_selection(accesskit::TextSelection { anchor, focus });
        }
    }
    ids
}

/// Character offset of a `TextPosition` inside runs published by
/// [`push_text_runs`].
pub fn offset_of(
    runs: &[(accesskit::NodeId, usize)],
    position: &accesskit::TextPosition,
) -> Option<usize> {
    runs.iter()
        .find(|(id, _)| *id == position.node)
        .map(|(_, start)| start + position.character_index)
}

fn char_offset(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    text[..byte].chars().count()
}

fn byte_offset(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(byte, _)| byte)
}

/// An element that publishes an [`InputState`]'s text to assistive
/// technology.
pub trait AccessibleTextInput: Sized {
    /// Publish `state`'s text, caret and selection on this element's node,
    /// and let assistive technology focus the field and move its caret or
    /// selection (`Focus`, `SetTextSelection`). The element must have an id
    /// and a text role (`Role::TextInput`, `MultilineTextInput` or
    /// `SearchInput`), and contain the field.
    fn accessible_text_input(self, state: &Entity<InputState>, cx: &App) -> Self;
}

impl AccessibleTextInput for Stateful<Div> {
    fn accessible_text_input(self, state: &Entity<InputState>, cx: &App) -> Self {
        let input = state.read(cx);
        // Cheap: the rope is shared, not copied, until a tree is built.
        let rope = input.text().clone();
        let selected = input.selected_range();
        let cursor = input.cursor();
        let runs: Rc<RefCell<Vec<(accesskit::NodeId, usize)>>> = Rc::default();
        let published = runs.clone();
        let focus_state = state.clone();
        let selection_state = state.clone();
        self.a11y_synthetic_children(move |builder| {
            builder.parent_node().set_class_name(TEXT_PROXY_CLASS);
            let text = rope.to_string();
            let (anchor, focus) = if cursor == selected.start {
                (selected.end, selected.start)
            } else {
                (selected.start, selected.end)
            };
            let selection = Some((char_offset(&text, anchor), char_offset(&text, focus)));
            *published.borrow_mut() = push_text_runs(builder, &text, selection);
        })
        .on_a11y_action(AccessibleAction::Focus, move |_, window, cx| {
            let handle = focus_state.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        })
        .on_a11y_action(
            AccessibleAction::SetTextSelection,
            move |data, window, cx| {
                let Some(accesskit::ActionData::SetTextSelection(selection)) = data else {
                    return;
                };
                let runs = runs.borrow();
                let (Some(anchor), Some(focus)) = (
                    offset_of(&runs, &selection.anchor),
                    offset_of(&runs, &selection.focus),
                ) else {
                    return;
                };
                let handle = selection_state.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
                selection_state.update(cx, |state, cx| {
                    let text = state.value();
                    let range: Range<usize> = byte_offset(&text, anchor.min(focus))
                        ..byte_offset(&text, anchor.max(focus));
                    state.set_selected_range(range, cx);
                });
            },
        )
    }
}

/// Set a node's accessible description (what a screen reader reads after
/// the name, such as a file's kind). GPUI has no `aria_description`, so this
/// writes it through the element's synthetic-children hook; don't combine it
/// with another `a11y_synthetic_children` on the same element.
pub fn with_description(
    element: Stateful<Div>,
    description: impl Into<gpui::SharedString>,
) -> Stateful<Div> {
    let description = description.into();
    element.a11y_synthetic_children(move |builder| {
        builder
            .parent_node()
            .set_description(description.to_string());
    })
}

/// Longest `Table` exposed to assistive technology, in rows. Bounds the
/// per-frame cost of publishing off-screen rows for any `rmac_ui::Table`
/// user, however many rows its model actually holds — an honest, bounded
/// degradation (rows past the cap are simply not reachable) rather than an
/// unbounded per-frame cost.
pub const MAX_ACCESSIBLE_TABLE_ROWS: usize = 4096;

/// One model row that isn't currently painted, published to assistive
/// technology because a `Table`'s virtualization skips it. See
/// [`offscreen_table_row_indices`] and [`push_offscreen_table_rows`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleTableRow {
    /// The row's index into the table's full model (0-based).
    pub index: usize,
    /// The row's name, read from its first column.
    pub name: SharedString,
    /// Whether this row is the table's selected row.
    pub selected: bool,
}

/// Row indices that need a synthetic AccessKit node: every model row
/// index below `row_count` and outside `painted` (which already has a
/// real, accessible element from the delegate's own row rendering), up to
/// [`MAX_ACCESSIBLE_TABLE_ROWS`].
pub fn offscreen_table_row_indices(row_count: usize, painted: Range<usize>) -> Vec<usize> {
    (0..row_count.min(MAX_ACCESSIBLE_TABLE_ROWS))
        .filter(|index| !painted.contains(index))
        .collect()
}

/// A deterministic AccessKit node id for a synthetic table row, stable
/// across frames (so a screen reader keeps tracking the same row as it
/// scrolls into and out of the painted range) and unique per table
/// instance. `salt` should be the hosting `Entity`'s id
/// (`Entity::entity_id().as_u64()`), so two `Table`s on screen at once
/// never collide.
pub fn table_row_node_id(salt: u64, row_index: usize) -> accesskit::NodeId {
    let mut hasher = DefaultHasher::new();
    salt.hash(&mut hasher);
    "rmac_ui::accessibility::table_row".hash(&mut hasher);
    row_index.hash(&mut hasher);
    accesskit::NodeId(hasher.finish())
}

/// Push one synthetic `Role::Row` node per `row`: its 1-based row index
/// and the table's row count (so a screen reader can announce "row 145 of
/// 300"), its name, its selected state, and — since none of these rows
/// are painted — `bounds`, which callers should fill in with the table's
/// own bounds (the best available position for an item that isn't on
/// screen). Each node also advertises the `Click` and `Focus` actions;
/// [`table_row_node_id`] gives the id to register their handlers under
/// with `Window::on_a11y_action`, since this builder has no access to
/// `Window` to register them itself.
pub fn push_offscreen_table_rows(
    builder: &mut A11ySubtreeBuilder,
    salt: u64,
    rows: &[AccessibleTableRow],
    row_count: usize,
    bounds: Option<accesskit::Rect>,
) {
    for row in rows {
        let id = table_row_node_id(salt, row.index);
        let mut node = accesskit::Node::new(accesskit::Role::Row);
        node.set_label(row.name.to_string());
        node.set_selected(row.selected);
        node.set_row_index(row.index + 1);
        node.set_row_count(row_count);
        node.add_action(accesskit::Action::Click);
        node.add_action(accesskit::Action::Focus);
        if let Some(bounds) = bounds {
            node.set_bounds(bounds);
        }
        builder.push_child(id, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(runs: &[TextRunSpec]) -> Vec<&str> {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn one_run_per_line_with_an_empty_run_after_a_final_break() {
        let runs = text_runs("ab\ncd");
        assert_eq!(texts(&runs), ["ab\n", "cd"]);
        assert_eq!((runs[0].start, runs[1].start), (0, 3));

        let runs = text_runs("ab\n");
        assert_eq!(texts(&runs), ["ab\n", ""]);
        assert_eq!(runs[1].start, 3);

        assert_eq!(texts(&text_runs("")), [""]);
    }

    #[test]
    fn lengths_count_scalar_values_and_words_keep_trailing_space() {
        let runs = text_runs("héllo wörld  x\n");
        let run = &runs[0];
        assert_eq!(run.len(), 15);
        assert_eq!(run.character_lengths[1], 2);
        assert_eq!(run.word_starts, vec![0, 6, 13]);
        assert_eq!(text_runs("  a b")[0].word_starts, vec![0, 2, 4]);
    }

    #[test]
    fn word_starts_stop_where_accesskit_indices_end() {
        let line = format!("{} y", "x".repeat(300));
        assert_eq!(text_runs(&line)[0].word_starts, vec![0]);
    }

    #[test]
    fn positions_prefer_the_start_of_the_next_line() {
        let runs = text_runs("ab\ncd");
        assert_eq!(run_position(&runs, 0), Some((0, 0)));
        assert_eq!(run_position(&runs, 2), Some((0, 2)));
        assert_eq!(run_position(&runs, 3), Some((1, 0)));
        assert_eq!(run_position(&runs, 5), Some((1, 2)));
        assert_eq!(run_position(&runs, 6), None);

        let runs = text_runs("ab\n");
        assert_eq!(run_position(&runs, 3), Some((1, 0)));
    }

    #[test]
    fn byte_and_character_offsets_round_trip() {
        let text = "aé界b";
        assert_eq!(char_offset(text, 3), 2);
        assert_eq!(char_offset(text, 2), 1, "inside a character rounds down");
        assert_eq!(byte_offset(text, 3), 6);
        assert_eq!(byte_offset(text, 99), text.len());
    }

    #[test]
    fn oversized_text_is_cut_at_a_character_boundary() {
        let text = "é".repeat(MAX_ACCESSIBLE_TEXT_BYTES);
        let runs = text_runs(&text);
        let total: usize = runs.iter().map(|run| run.text.len()).sum();
        assert!(total <= MAX_ACCESSIBLE_TEXT_BYTES);
        assert_eq!(total % 2, 0);
    }

    #[test]
    fn offscreen_rows_are_every_index_outside_the_painted_range() {
        assert_eq!(
            offscreen_table_row_indices(10, 3..6),
            vec![0, 1, 2, 6, 7, 8, 9]
        );
        // A painted range covering the whole model needs no synthetic rows.
        assert_eq!(offscreen_table_row_indices(5, 0..5), Vec::<usize>::new());
        // Nothing painted at all (e.g. before the first layout): every row.
        assert_eq!(offscreen_table_row_indices(3, 0..0), vec![0, 1, 2]);
    }

    #[test]
    fn offscreen_rows_are_capped_so_a_huge_model_stays_bounded() {
        let indices = offscreen_table_row_indices(MAX_ACCESSIBLE_TABLE_ROWS + 500, 0..0);
        assert_eq!(indices.len(), MAX_ACCESSIBLE_TABLE_ROWS);
        assert_eq!(indices.last(), Some(&(MAX_ACCESSIBLE_TABLE_ROWS - 1)));
    }

    #[test]
    fn row_node_ids_are_stable_and_distinct() {
        // Stable: the same salt and row index always hash the same way, so a
        // screen reader keeps tracking a row across frames.
        assert_eq!(table_row_node_id(1, 5), table_row_node_id(1, 5));
        // Distinct per row...
        assert_ne!(table_row_node_id(1, 5), table_row_node_id(1, 6));
        // ...and distinct per table, so two `Table`s never collide.
        assert_ne!(table_row_node_id(1, 5), table_row_node_id(2, 5));
    }
}
