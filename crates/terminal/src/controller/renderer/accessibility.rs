//! Publishes the visible terminal grid to assistive technology.
//!
//! `rmac_terminal::accessibility::project_visible_terminal` (unit-tested in
//! that module) is framework-neutral: it turns the live `alacritty_terminal`
//! grid into plain text plus scalar-offset caret/selection ranges. This file
//! is the GPUI adapter that publishes that projection on the grid's
//! `Role::Terminal` node through `rmac_ui::accessibility::push_text_runs`:
//! one `Role::TextRun` per logical line (soft-wrapped rows joined), so a
//! screen reader's line navigation reads a line, not the whole screen, with
//! the caret or selection as the node's `TextSelection`. AccessKit derives
//! AT-SPI text-inserted/removed and caret-moved events from the change
//! between two published trees.
//!
//! Only the visible screen is published. Scrollback is reached by scrolling
//! the view, which republishes the screen at that offset (the caret is then
//! withheld, as the insertion point is off screen).
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
    /// once per [`MIN_ACCESSIBILITY_INTERVAL`] per tab. A redraw inside the
    /// interval reuses the last snapshot and schedules one redraw at the end
    /// of the interval, so output that stops mid-interval is still published.
    fn terminal_accessibility_snapshot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<TerminalAccessibilitySnapshot> {
        let tab_id = self.tabs[self.active].id;
        if let Some(cache) = &mut self.a11y_cache {
            let elapsed = cache.computed_at.elapsed();
            if cache.tab_id == tab_id && elapsed < MIN_ACCESSIBILITY_INTERVAL {
                if !cache.refresh_scheduled {
                    cache.refresh_scheduled = true;
                    let wait = MIN_ACCESSIBILITY_INTERVAL - elapsed + Duration::from_millis(1);
                    cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                        cx.background_executor().timer(wait).await;
                        let _ = this.update(cx, |this, cx| {
                            if let Some(cache) = &mut this.a11y_cache {
                                cache.refresh_scheduled = false;
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
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
            refresh_scheduled: false,
        });
        Some(snapshot)
    }

    /// A closure to hand to `.a11y_synthetic_children()` on the terminal
    /// grid's `Role::Terminal` node. Nothing is projected while no assistive
    /// technology is listening.
    pub(super) fn render_terminal_accessibility(
        &mut self,
        a11y_active: bool,
        cx: &mut Context<Self>,
    ) -> impl FnOnce(&mut A11ySubtreeBuilder) + 'static {
        let snapshot = a11y_active
            .then(|| self.terminal_accessibility_snapshot(cx))
            .flatten();
        move |builder| {
            let Some(snapshot) = snapshot else {
                return;
            };
            let selection = snapshot
                .selection
                .map(|selection| (selection.start, selection.end))
                .or(snapshot.caret.map(|caret| (caret, caret)));
            rmac_ui::accessibility::push_text_runs(builder, &snapshot.text, selection);
        }
    }
}
