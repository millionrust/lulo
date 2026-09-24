//! Publishes the visible terminal grid to assistive technology.
//!
//! `rmac_terminal::accessibility::project_visible_terminal` (unit-tested in
//! that module) is framework-neutral: it turns the live `alacritty_terminal`
//! grid into plain text plus scalar-offset caret/selection ranges. This file
//! is the GPUI adapter that publishes that projection as a real AccessKit
//! node, following the exact synthetic-children pattern GPUI's own
//! `_accessibility` guide documents for a text surface that isn't backed by
//! individually-addressable child elements: a `Role::Terminal` node whose
//! single synthetic `Role::TextRun` child carries the value and character
//! lengths, with the caret/selection expressed as a `TextSelection` over
//! that run.
//!
//! `rmac_terminal` is compiled both as this binary's own library target
//! (`src/lib.rs`) and linked into it automatically, the same way
//! `crates/finder/src/view/accessibility.rs` reaches `rmac_finder::accessibility`
//! without `main.rs` declaring a duplicate `mod accessibility;`.

use super::*;
use rmac_terminal::accessibility::{project_visible_terminal, GridPoint, GridSelection};
use std::time::{Duration, Instant};

/// Minimum time between accessibility rebuilds. Idle windows never reach this
/// path at all (nothing calls `cx.notify()` without real output/input), so
/// this only guards against output storms, not idle polling.
const MIN_ACCESSIBILITY_INTERVAL: Duration = Duration::from_millis(120);

impl TerminalView {
    /// The active tab's bounded visible-grid snapshot, recomputed at most
    /// once per [`MIN_ACCESSIBILITY_INTERVAL`] per tab.
    fn terminal_accessibility_snapshot(&mut self) -> Option<TerminalAccessibilitySnapshot> {
        let tab_id = self.tabs[self.active].id;
        if let Some(cache) = &self.a11y_cache {
            if cache.tab_id == tab_id && cache.computed_at.elapsed() < MIN_ACCESSIBILITY_INTERVAL {
                return Some(cache.snapshot.clone());
            }
        }
        let selection = self.tabs[self.active]
            .ui
            .selection
            .map(|selection| GridSelection {
                anchor: GridPoint {
                    line: selection.anchor.0,
                    column: selection.anchor.1,
                },
                head: GridPoint {
                    line: selection.head.0,
                    column: selection.head.1,
                },
            });
        let input_enabled = self.tabs[self.active].accepts_input();
        let Ok(term) = self.tabs[self.active].term.lock() else {
            // A poisoned lock leaves the last-known snapshot in place rather
            // than dropping accessible content on a transient failure.
            return self.a11y_cache.as_ref().map(|cache| cache.snapshot.clone());
        };
        let snapshot = project_visible_terminal(&term, selection, input_enabled).ok()?;
        drop(term);
        self.a11y_cache = Some(TerminalAccessibilityCache {
            tab_id,
            computed_at: Instant::now(),
            snapshot: snapshot.clone(),
        });
        Some(snapshot)
    }

    /// A closure to hand to `.a11y_synthetic_children()` on the terminal
    /// grid's `Role::Terminal` node.
    pub(super) fn render_terminal_accessibility(
        &mut self,
    ) -> impl FnOnce(&mut A11ySubtreeBuilder) + 'static {
        let snapshot = self.terminal_accessibility_snapshot();
        move |builder| {
            let Some(snapshot) = snapshot else {
                return;
            };
            let mut run = accesskit::Node::new(Role::TextRun);
            run.set_value(snapshot.text.clone());
            run.set_character_lengths(
                snapshot
                    .text
                    .chars()
                    .map(|character| character.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            let run_id = builder.synthetic_node_id("terminal-grid-text");
            builder.push_child(run_id, run);

            let position = |character_index: usize| accesskit::TextPosition {
                node: run_id,
                character_index,
            };
            if let Some(selection) = snapshot.selection {
                builder
                    .parent_node()
                    .set_text_selection(accesskit::TextSelection {
                        anchor: position(selection.start),
                        focus: position(selection.end),
                    });
            } else if let Some(caret) = snapshot.caret {
                let caret = position(caret);
                builder
                    .parent_node()
                    .set_text_selection(accesskit::TextSelection {
                        anchor: caret,
                        focus: caret,
                    });
            }
        }
    }
}
