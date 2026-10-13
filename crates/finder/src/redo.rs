//! Edit ▸ Redo for file operations (FILES-32).
//!
//! Undo already runs through the crash-safe journal in `undo_journal`. Redo
//! deliberately does not grow a second journal: when Undo finishes, the
//! journal reports the plain steps it just reversed (move this item back,
//! remove that copy, ...), and Redo replays them through the *same*
//! journalled transfer, Trash and New Folder paths a user-initiated
//! operation takes. Every redone step therefore archives its own fresh Undo
//! receipt, is protected by the same conflict checks and recovery records,
//! and makes Undo available again, exactly like the first time. A crash
//! mid-Redo is recovered by the ordinary journal; the worst case after a
//! crash between Undo and Redo is that Redo is gone, never that data is.
//!
//! Redo history is session-only, as on the Mac, and is dropped the moment
//! the user does anything new, so it can only ever replay the operation the
//! Undo stack is currently one step behind.

use std::path::PathBuf;

/// One reversed operation, in the form needed to do it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RedoStep {
    /// Move `from` to `to` again (also covers a rename).
    Move { from: PathBuf, to: PathBuf },
    /// Copy `from` to `to` again.
    Copy { from: PathBuf, to: PathBuf },
    /// Create the empty folder at `path` again.
    NewFolder { path: PathBuf },
    /// Move `path` to the Bin again.
    Trash { path: PathBuf },
}

/// What kind of replay a [`RedoEntry`] needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RedoKind {
    /// Moves and copies run as one conflict-checked transfer.
    Transfer,
    /// Items go to the Bin one by one inside one undo batch.
    Trash,
    /// A single new folder.
    NewFolder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RedoEntry {
    pub(crate) steps: Vec<RedoStep>,
    /// The Undo label this operation had ("Undo Move “x”"), which the
    /// operation gets again when redone and which identifies it.
    undo_label: String,
    /// The latest Undo receipt right after the Undo that created this
    /// entry; the entry is only valid while that is still the latest.
    after: Option<String>,
}

impl RedoEntry {
    /// `None` when there is nothing to replay or the steps are a mix that
    /// no single user operation produces.
    pub(crate) fn new(
        steps: Vec<RedoStep>,
        undo_label: String,
        after: Option<String>,
    ) -> Option<Self> {
        let entry = Self {
            steps,
            undo_label,
            after,
        };
        entry.kind()?;
        Some(entry)
    }

    pub(crate) fn kind(&self) -> Option<RedoKind> {
        let first = self.steps.first()?;
        let kind = match first {
            RedoStep::Move { .. } | RedoStep::Copy { .. } => RedoKind::Transfer,
            RedoStep::Trash { .. } => RedoKind::Trash,
            RedoStep::NewFolder { .. } => RedoKind::NewFolder,
        };
        let uniform = self.steps.iter().all(|step| {
            matches!(
                (kind, step),
                (
                    RedoKind::Transfer,
                    RedoStep::Move { .. } | RedoStep::Copy { .. }
                ) | (RedoKind::Trash, RedoStep::Trash { .. })
                    | (RedoKind::NewFolder, RedoStep::NewFolder { .. })
            )
        });
        let single_folder = kind != RedoKind::NewFolder || self.steps.len() == 1;
        (uniform && single_folder).then_some(kind)
    }

    /// The menu label: the Undo label with its verb swapped.
    pub(crate) fn label(&self) -> String {
        self.undo_label.replacen("Undo", "Redo", 1)
    }
}

/// The session's undone-but-redoable operations, newest on top.
#[derive(Debug, Default)]
pub(crate) struct RedoStack {
    entries: Vec<RedoEntry>,
    /// Set when a Redo starts: the latest receipt before it ran.
    pending: Option<Option<String>>,
}

impl RedoStack {
    pub(crate) fn push(&mut self, entry: RedoEntry) {
        self.entries.push(entry);
    }

    /// The entry Redo would run now: the top one, and only while the Undo
    /// history is exactly where that entry's Undo left it.
    pub(crate) fn current(&self, latest: Option<&str>) -> Option<&RedoEntry> {
        self.entries
            .last()
            .filter(|entry| entry.after.as_deref() == latest)
    }

    /// Remember that a Redo of the top entry is starting while `latest` is
    /// the newest receipt.
    pub(crate) fn begin_redo(&mut self, latest: Option<&str>) {
        self.pending = Some(latest.map(str::to_owned));
    }

    /// Reconcile with the Undo history after any operation finished. A Redo
    /// that completed replaced the newest receipt with an equivalent one
    /// (same label), so the entry is consumed and the one beneath it now
    /// follows the new receipt. Anything else the user did makes the whole
    /// stack stale and drops it.
    pub(crate) fn sync(&mut self, latest: Option<&str>, latest_label: Option<&str>) {
        if let Some(before) = self.pending.take() {
            let redone = latest != before.as_deref()
                && self
                    .entries
                    .last()
                    .is_some_and(|entry| Some(entry.undo_label.as_str()) == latest_label);
            if redone {
                self.entries.pop();
                if let Some(next) = self.entries.last_mut() {
                    next.after = latest.map(str::to_owned);
                }
            }
        }
        if self
            .entries
            .last()
            .is_some_and(|entry| entry.after.as_deref() != latest)
        {
            self.entries.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(from: &str, to: &str) -> RedoStep {
        RedoStep::Move {
            from: from.into(),
            to: to.into(),
        }
    }

    fn entry(label: &str, after: &str) -> RedoEntry {
        RedoEntry::new(
            vec![mv("/a/x", "/b/x")],
            label.to_owned(),
            Some(after.to_owned()),
        )
        .unwrap()
    }

    #[test]
    fn redo_label_swaps_only_the_verb() {
        assert_eq!(
            entry("Undo Move “Undo me”", "1").label(),
            "Redo Move “Undo me”"
        );
    }

    #[test]
    fn mixed_or_empty_steps_are_not_redoable() {
        assert!(RedoEntry::new(Vec::new(), "Undo Move “x”".into(), None).is_none());
        assert!(RedoEntry::new(
            vec![mv("/a", "/b"), RedoStep::Trash { path: "/c".into() }],
            "Undo Move “x”".into(),
            None
        )
        .is_none());
        assert!(RedoEntry::new(
            vec![
                RedoStep::NewFolder { path: "/a".into() },
                RedoStep::NewFolder { path: "/b".into() }
            ],
            "Undo New Folder “a”".into(),
            None
        )
        .is_none());
        let copies_and_moves = RedoEntry::new(
            vec![
                mv("/a", "/b"),
                RedoStep::Copy {
                    from: "/c".into(),
                    to: "/d".into(),
                },
            ],
            "Undo Copy “d”".into(),
            None,
        )
        .unwrap();
        assert_eq!(copies_and_moves.kind(), Some(RedoKind::Transfer));
    }

    #[test]
    fn redo_is_offered_only_while_undo_is_where_it_left_it() {
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “x”", "r1"));
        assert!(stack.current(Some("r1")).is_some());
        assert!(stack.current(Some("r2")).is_none());
        assert!(stack.current(None).is_none());
    }

    #[test]
    fn a_new_operation_drops_the_redo_history() {
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “x”", "r1"));
        stack.sync(Some("r9"), Some("Undo Copy “y”"));
        assert_eq!(stack.len(), 0);
    }

    #[test]
    fn an_unrelated_event_leaves_a_valid_stack_alone() {
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “x”", "r1"));
        stack.sync(Some("r1"), Some("Undo Copy “y”"));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn redoing_rebases_the_entry_beneath_onto_the_new_receipt() {
        // Undo B (history now ends at A), then Undo A (history ends at Z):
        // stack is [B after A, A after Z].
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “b”", "rA"));
        stack.push(entry("Undo Move “a”", "rZ"));
        // Redo A: it runs, archives receipt rA2 with the same label.
        stack.begin_redo(Some("rZ"));
        stack.sync(Some("rA2"), Some("Undo Move “a”"));
        assert_eq!(stack.len(), 1);
        assert_eq!(
            stack.current(Some("rA2")).map(RedoEntry::label),
            Some("Redo Move “b”".to_owned())
        );
        // Redo B: the stack empties.
        stack.begin_redo(Some("rA2"));
        stack.sync(Some("rB2"), Some("Undo Move “b”"));
        assert_eq!(stack.len(), 0);
    }

    #[test]
    fn a_redo_that_changed_nothing_keeps_the_entry() {
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “x”", "r1"));
        stack.begin_redo(Some("r1"));
        stack.sync(Some("r1"), Some("Undo Copy “y”"));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn a_different_operation_during_redo_is_not_mistaken_for_it() {
        let mut stack = RedoStack::default();
        stack.push(entry("Undo Move “x”", "r1"));
        stack.begin_redo(Some("r1"));
        stack.sync(Some("r2"), Some("Undo Copy “z”"));
        assert_eq!(stack.len(), 0);
    }
}
