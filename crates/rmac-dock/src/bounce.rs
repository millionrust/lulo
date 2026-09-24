//! Launch and attention bounce of Dock tiles.
//!
//! Measured on macOS 26.2 at the default tile size of 64 pt from 60 fps
//! screen recordings (docs/dock-behaviour-2026-09-23.md): one bounce lifts the
//! tile 18 pt and lands again after 680 ms. The lift follows a parabola: at
//! 17 % of the period the tile was 56 % of the way up, at 81 % it was 62 %,
//! matching `4p(1 - p)` within a point. A launch keeps bouncing until the
//! application shows a window and then finishes the bounce in progress, so a
//! warm launch bounces exactly once.

use std::collections::{BTreeMap, BTreeSet};

/// Duration of one bounce, lift-off to landing.
pub const BOUNCE_PERIOD_MS: u64 = 680;
/// Peak lift as a fraction of the tile size (18 of 64 pt).
pub const BOUNCE_HEIGHT_RATIO: f32 = 18.0 / 64.0;
/// rmac policy for a launch that never shows a window: give up after about
/// ten seconds instead of bouncing forever.
pub const MAX_LAUNCH_BOUNCES: u64 = 15;

/// Lift of a tile `elapsed_ms` after its bounce started, for a tile of
/// `tile` logical pixels. Repeats every period.
pub fn bounce_lift(elapsed_ms: u64, tile: f32) -> f32 {
    let phase = (elapsed_ms % BOUNCE_PERIOD_MS) as f32 / BOUNCE_PERIOD_MS as f32;
    4.0 * phase * (1.0 - phase) * BOUNCE_HEIGHT_RATIO * tile
}

/// The end of the bounce in progress at `now_ms` for a bounce that started at
/// `started_ms`; a bounce that has not left the ground yet still runs once.
fn end_of_current_bounce(started_ms: u64, now_ms: u64) -> u64 {
    let elapsed = now_ms.saturating_sub(started_ms);
    let bounces = elapsed.div_ceil(BOUNCE_PERIOD_MS).max(1);
    started_ms.saturating_add(bounces.saturating_mul(BOUNCE_PERIOD_MS))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Bounce {
    started_ms: u64,
    /// Landing time once the reason to bounce has ended.
    ends_ms: Option<u64>,
}

impl Bounce {
    fn lift(&self, now_ms: u64, tile: f32) -> f32 {
        if now_ms < self.started_ms || self.ends_ms.is_some_and(|end| now_ms >= end) {
            return 0.0;
        }
        bounce_lift(now_ms - self.started_ms, tile)
    }

    fn finished(&self, now_ms: u64) -> bool {
        self.ends_ms.is_some_and(|end| now_ms >= end)
    }
}

/// Bounce state for every application tile, keyed by canonical app ID.
///
/// It only reacts to authoritative facts: a launch the Dock itself issued, a
/// window appearing for an application, and compositor urgency.
#[derive(Clone, Debug, Default)]
pub struct BounceTracker {
    launches: BTreeMap<String, Bounce>,
    attention: BTreeMap<String, Bounce>,
    running: BTreeSet<String>,
    initialized: bool,
}

impl BounceTracker {
    /// The Dock launched `app_id`: bounce until its first window appears.
    /// Opening another window of an application that is already running does
    /// not bounce on macOS, so it is ignored here.
    pub fn launch(&mut self, app_id: &str, now_ms: u64) {
        let key = canonical(app_id);
        if self.running.contains(&key)
            || self
                .launches
                .get(&key)
                .is_some_and(|bounce| !bounce.finished(now_ms))
        {
            return;
        }
        self.launches.insert(
            key,
            Bounce {
                started_ms: now_ms,
                ends_ms: Some(
                    now_ms.saturating_add(MAX_LAUNCH_BOUNCES.saturating_mul(BOUNCE_PERIOD_MS)),
                ),
            },
        );
    }

    /// The launch of `app_id` failed: the tile lands at the end of the
    /// current bounce instead of bouncing on toward the cap.
    pub fn launch_failed(&mut self, app_id: &str, now_ms: u64) {
        if let Some(bounce) = self.launches.get_mut(&canonical(app_id)) {
            if !bounce.finished(now_ms) {
                let end = end_of_current_bounce(bounce.started_ms, now_ms);
                bounce.ends_ms = Some(bounce.ends_ms.map_or(end, |cap| cap.min(end)));
            }
        }
    }

    /// Reconcile with the newest Dock model. `running` holds applications
    /// with at least one window; `attention` holds applications with an
    /// urgent window that are not frontmost.
    pub fn reconcile(
        &mut self,
        running: &BTreeSet<String>,
        attention: &BTreeSet<String>,
        now_ms: u64,
    ) {
        let running: BTreeSet<String> = running.iter().map(|id| canonical(id)).collect();
        let attention: BTreeSet<String> = attention.iter().map(|id| canonical(id)).collect();
        if self.initialized {
            for app in running.difference(&self.running) {
                match self.launches.get_mut(app) {
                    // The launch finished: land at the end of this bounce.
                    Some(bounce) if !bounce.finished(now_ms) => {
                        let end = end_of_current_bounce(bounce.started_ms, now_ms);
                        bounce.ends_ms = Some(bounce.ends_ms.map_or(end, |cap| cap.min(end)));
                    }
                    // Launched from elsewhere (Spotlight, Files, a terminal):
                    // the Dock still bounces the new app once.
                    _ => {
                        self.launches.insert(
                            app.clone(),
                            Bounce {
                                started_ms: now_ms,
                                ends_ms: Some(now_ms.saturating_add(BOUNCE_PERIOD_MS)),
                            },
                        );
                    }
                }
            }
        }
        for app in &attention {
            // An open end means "still asking". A request that returns before
            // the tile landed keeps the same rhythm; a later one starts anew.
            match self.attention.get_mut(app) {
                Some(bounce) if !bounce.finished(now_ms) => bounce.ends_ms = None,
                _ => {
                    self.attention.insert(
                        app.clone(),
                        Bounce {
                            started_ms: now_ms,
                            ends_ms: None,
                        },
                    );
                }
            }
        }
        for (app, bounce) in self.attention.iter_mut() {
            if !attention.contains(app) && bounce.ends_ms.is_none() {
                bounce.ends_ms = Some(end_of_current_bounce(bounce.started_ms, now_ms));
            }
        }
        self.launches.retain(|_, bounce| !bounce.finished(now_ms));
        self.attention.retain(|_, bounce| !bounce.finished(now_ms));
        self.running = running;
        self.initialized = true;
    }

    /// Current lift of `app_id`'s tile in logical pixels.
    pub fn lift(&self, app_id: &str, now_ms: u64, tile: f32) -> f32 {
        let key = canonical(app_id);
        let launch = self
            .launches
            .get(&key)
            .map_or(0.0, |bounce| bounce.lift(now_ms, tile));
        let attention = self
            .attention
            .get(&key)
            .map_or(0.0, |bounce| bounce.lift(now_ms, tile));
        launch.max(attention)
    }

    /// True while any tile still needs animation frames.
    pub fn is_animating(&self, now_ms: u64) -> bool {
        self.launches
            .values()
            .chain(self.attention.values())
            .any(|bounce| !bounce.finished(now_ms))
    }
}

fn canonical(app_id: &str) -> String {
    crate::canonical_app_id(app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn one_bounce_is_a_measured_parabola() {
        assert_eq!(bounce_lift(0, 64.0), 0.0);
        assert!((bounce_lift(BOUNCE_PERIOD_MS / 2, 64.0) - 18.0).abs() < 0.01);
        // 17 % of the way through the measured bounce the tile was 10 of 18 pt up.
        let early = bounce_lift(116, 64.0);
        assert!((early - 10.2).abs() < 0.6, "{early}");
        // Repeats every period.
        assert_eq!(
            bounce_lift(BOUNCE_PERIOD_MS + 100, 64.0),
            bounce_lift(100, 64.0)
        );
    }

    #[test]
    fn warm_launch_bounces_once_and_lands() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&[]), &set(&[]), 0);
        tracker.launch("calculator.desktop", 1_000);
        assert!(tracker.lift("calculator", 1_340, 64.0) > 17.0);
        // The window appears mid-bounce: the tile still lands first.
        tracker.reconcile(&set(&["calculator.desktop"]), &set(&[]), 1_300);
        assert!(tracker.lift("calculator.desktop", 1_600, 64.0) > 0.0);
        assert!(tracker.is_animating(1_679));
        assert!(!tracker.is_animating(1_680));
        assert_eq!(tracker.lift("calculator.desktop", 1_700, 64.0), 0.0);
    }

    #[test]
    fn slow_launch_keeps_bouncing_until_its_window_then_finishes_the_bounce() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&[]), &set(&[]), 0);
        tracker.launch("notes", 0);
        tracker.reconcile(&set(&[]), &set(&[]), 2_000);
        assert!(tracker.is_animating(2_000));
        // Window at 2.1 s: third bounce (2040..2720) is completed.
        tracker.reconcile(&set(&["notes"]), &set(&[]), 2_100);
        assert!(tracker.is_animating(2_719));
        assert!(!tracker.is_animating(2_720));
    }

    #[test]
    fn a_launch_that_never_shows_a_window_stops_at_the_cap() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&[]), &set(&[]), 0);
        tracker.launch("ghost", 0);
        let cap = MAX_LAUNCH_BOUNCES * BOUNCE_PERIOD_MS;
        assert!(tracker.is_animating(cap - 1));
        assert!(!tracker.is_animating(cap));
    }

    #[test]
    fn a_failed_launch_lands_after_the_current_bounce() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&[]), &set(&[]), 0);
        tracker.launch("ghost.desktop", 0);
        tracker.launch_failed("ghost", 100);
        assert!(tracker.is_animating(BOUNCE_PERIOD_MS - 1));
        assert!(!tracker.is_animating(BOUNCE_PERIOD_MS));
        // Failing an app that is not bouncing changes nothing.
        tracker.launch_failed("other", 100);
        assert!(!tracker.is_animating(BOUNCE_PERIOD_MS));
    }

    #[test]
    fn apps_already_running_at_startup_do_not_bounce() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&["files", "terminal"]), &set(&[]), 0);
        assert!(!tracker.is_animating(0));
        // A new window of a running app (New Window, Open) does not bounce.
        tracker.launch("terminal.desktop", 10);
        assert!(!tracker.is_animating(10));
        // An app launched from elsewhere later bounces exactly once.
        tracker.reconcile(&set(&["files", "terminal", "preview"]), &set(&[]), 500);
        assert!(tracker.lift("preview", 500 + BOUNCE_PERIOD_MS / 2, 64.0) > 17.0);
        assert!(!tracker.is_animating(500 + BOUNCE_PERIOD_MS));
    }

    #[test]
    fn attention_bounces_until_the_request_ends_then_lands() {
        let mut tracker = BounceTracker::default();
        tracker.reconcile(&set(&["mail"]), &set(&[]), 0);
        tracker.reconcile(&set(&["mail"]), &set(&["mail"]), 100);
        assert!(tracker.is_animating(100 + 10 * BOUNCE_PERIOD_MS));
        tracker.reconcile(&set(&["mail"]), &set(&[]), 100 + 3 * BOUNCE_PERIOD_MS + 50);
        assert!(tracker.is_animating(100 + 4 * BOUNCE_PERIOD_MS - 1));
        assert!(!tracker.is_animating(100 + 4 * BOUNCE_PERIOD_MS));
    }
}
