//! Continuous Wi-Fi scanning while the Wi-Fi pane is open (SET-14).
//!
//! The Mac's Wi-Fi settings pane keeps scanning for nearby networks for as
//! long as it's on screen, not just once when it first appears. Lulo
//! mirrors that: an immediate `RequestScan` the moment the pane becomes the
//! visible pane, then another roughly every 10-15 s for as long as it stays
//! visible, and none at all once it's hidden or closed — the periodic timer
//! below runs for the settings window's whole lifetime (the same low-cost
//! "wake up, check whether anyone cares" shape as the Date & Time clock
//! refresh in `initialization/watchers/system.rs`), but only ever calls
//! into NetworkManager when this pane is the one on screen.

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

/// Whether becoming (or staying) visible should kick off an immediate scan:
/// only on the transition into view, not on every navigation event that
/// leaves it visible or leaves it hidden (SET-14: "call RequestScan when
/// the pane opens", not on every render).
pub(in crate::controller) fn wifi_pane_scan_should_start_on_navigation(
    now_visible: bool,
    was_visible: bool,
) -> bool {
    now_visible && !was_visible
}

impl Settings {
    pub(in crate::controller) fn wifi_pane_visible(&self) -> bool {
        wifi_pane_is_visible(self.nav.is_empty(), self.current().name.as_ref())
    }

    /// Call after any navigation change (category selection, subpage
    /// push/pop, back/forward). Kicks off an immediate scan the moment the
    /// Wi-Fi pane becomes the visible pane; does nothing when it already
    /// was, or still isn't.
    pub(in crate::controller) fn sync_wifi_pane_scan_on_navigation(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let visible = self.wifi_pane_visible();
        if wifi_pane_scan_should_start_on_navigation(visible, self.wifi_pane_was_visible) {
            self.request_wifi_pane_scan(cx);
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

    /// Started once for the settings window's lifetime
    /// (`initialization::watchers::start_watchers`); see the module doc for
    /// why this is safe to leave running rather than start and stop it with
    /// the pane.
    pub(in crate::controller) fn start_wifi_pane_scan_loop(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            async_io::Timer::after(WIFI_PANE_SCAN_INTERVAL).await;
            if this
                .update(cx, |this: &mut Settings, cx| {
                    if this.wifi_pane_visible() {
                        this.request_wifi_pane_scan(cx);
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }
}
