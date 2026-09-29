//! Continuous Wi-Fi scanning while the Wi-Fi pane is open (SET-14).
//!
//! The Mac's Wi-Fi settings pane keeps scanning for nearby networks for as
//! long as it's on screen, not just once when it first appears. Lulo
//! mirrors that: an immediate `RequestScan` the moment the pane becomes the
//! visible pane, then another roughly every 10-15 s for as long as it stays
//! visible. Unlike a "wake up, check whether anyone cares" timer that runs
//! for the whole window's lifetime, no timer exists at all while the pane
//! isn't visible: `wifi_pane_scan_task` holds the one live `Task` for the
//! periodic loop, and dropping it — the moment the pane is hidden, or the
//! whole `Settings` entity when the window closes — cancels it outright
//! (the same `Option<Task<()>>`-holds-the-timer idiom
//! `notification-center-app`'s `Host::wake` uses). Idle CPU stays at zero
//! whenever this pane isn't the one on screen.

use super::*;

/// How often the pane asks NetworkManager to scan again while it stays
/// open, in the 10-15 s band asked for (SET-14); the Mac's own interval
/// isn't published.
const WIFI_PANE_SCAN_INTERVAL: Duration = Duration::from_secs(12);
/// Give NetworkManager a moment after `RequestScan` before reading the
/// snapshot back, the same settle `start_snapshot_loads`'s startup scan
/// uses.
const WIFI_PANE_SCAN_SETTLE: Duration = Duration::from_millis(750);

/// The Wi-Fi pane is on screen: its category is selected and no subpage
/// (e.g. a "Details…" push) covers it (`render_detail` dispatches to
/// `render_wifi` under exactly this condition). A free function of the two
/// facts that decide it, so the rule is unit-testable without a live
/// `Settings` entity.
pub(in crate::controller) fn wifi_pane_is_visible(nav_is_empty: bool, category_name: &str) -> bool {
    nav_is_empty && category_name == "Wi-Fi"
}

/// Whether becoming visible should start the periodic scan loop and fire an
/// immediate scan: only on the transition into view, not on every
/// navigation event that leaves it visible (SET-14: "call RequestScan when
/// the pane opens", not on every render).
pub(in crate::controller) fn wifi_pane_scan_should_start_on_navigation(
    now_visible: bool,
    was_visible: bool,
) -> bool {
    now_visible && !was_visible
}

/// Whether becoming hidden should cancel the periodic scan loop: only on
/// the transition out of view, so a "still hidden" navigation event (moving
/// between two other panes) doesn't touch a loop that was never running.
pub(in crate::controller) fn wifi_pane_scan_should_stop_on_navigation(
    now_visible: bool,
    was_visible: bool,
) -> bool {
    !now_visible && was_visible
}

impl Settings {
    pub(in crate::controller) fn wifi_pane_visible(&self) -> bool {
        wifi_pane_is_visible(self.nav.is_empty(), self.current().name.as_ref())
    }

    /// Call after any navigation change (category selection, subpage
    /// push/pop, back/forward). Starts the periodic scan loop and fires an
    /// immediate scan the moment the Wi-Fi pane becomes the visible pane;
    /// cancels the loop the moment it stops being visible; does nothing on
    /// a navigation event that leaves visibility unchanged either way.
    pub(in crate::controller) fn sync_wifi_pane_scan_on_navigation(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let visible = self.wifi_pane_visible();
        if wifi_pane_scan_should_start_on_navigation(visible, self.wifi_pane_was_visible) {
            self.request_wifi_pane_scan(cx);
            self.wifi_pane_scan_task = Some(Self::spawn_wifi_pane_scan_loop(cx));
        } else if wifi_pane_scan_should_stop_on_navigation(visible, self.wifi_pane_was_visible) {
            // Dropping the Task cancels its timer outright, so no wake-up
            // exists at all while the pane is hidden (the owner's idle-CPU
            // budget, not just "skip the work on wake").
            self.wifi_pane_scan_task = None;
        }
        self.wifi_pane_was_visible = visible;
    }

    /// Ask NetworkManager to scan and re-read the Wi-Fi snapshot, unless a
    /// scan is already in flight or a real mutation (connecting, forgetting,
    /// toggling the radio) owns the state right now. A failed `RequestScan`
    /// (NetworkManager rate-limits repeated requests) doesn't stop the
    /// snapshot read below or surface as a pane-wide error — the same
    /// tolerance `start_snapshot_loads`'s startup scan already applies —
    /// since scanning is a background refresh here, not a user action.
    pub(in crate::controller) fn request_wifi_pane_scan(&mut self, cx: &mut Context<Self>) {
        if self.wifi_scanning
            || self.wifi_busy
            || self.wifi_loading
            || !self.wifi_available
            || !self.wifi_on
        {
            return;
        }
        self.wifi_scanning = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    let _ = rmac_network::request_scan();
                    std::thread::sleep(WIFI_PANE_SCAN_SETTLE);
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.wifi_scanning = false;
                // A real mutation may have started while this scan's
                // settle delay was sleeping; its own, more authoritative
                // snapshot wins, so leave this one on the floor.
                if !this.wifi_busy {
                    match result {
                        Ok(snapshot) => {
                            this.apply_wifi_snapshot(snapshot);
                            this.wifi_error = None;
                        }
                        Err(error) => {
                            this.wifi_error =
                                Some(format!("Could not update Wi-Fi: {error}").into());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One repeating timer, live only for as long as its `Task` handle is
    /// held (`wifi_pane_scan_task`) — deliberately **not** `.detach()`ed,
    /// so dropping that field (the pane hiding) or the whole entity (the
    /// window closing) cancels it immediately instead of leaving a
    /// "wake up and no-op" loop running for the rest of the process.
    /// `pub(in crate::controller)`: `initialization/construction.rs` also
    /// calls this directly, for a window that opens straight onto the
    /// Wi-Fi pane (`--pane wifi`), before `sync_wifi_pane_scan_on_navigation`
    /// ever runs.
    pub(in crate::controller) fn spawn_wifi_pane_scan_loop(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            async_io::Timer::after(WIFI_PANE_SCAN_INTERVAL).await;
            if this
                .update(cx, |this: &mut Settings, cx| {
                    this.request_wifi_pane_scan(cx);
                })
                .is_err()
            {
                break;
            }
        })
    }
}
