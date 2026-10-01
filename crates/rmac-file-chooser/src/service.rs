//! Bus-independent hand-off between the portal adapter and the panel UI.
//!
//! Each portal call becomes one [`PanelRequest`] on a channel that the GPUI
//! process drains. The adapter then waits for the panel's [`Outcome`] or for
//! the frontend's `Request.Close()`, whichever comes first.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use async_channel::{Receiver, Sender};

use crate::outcome::Outcome;
use crate::recent;
use crate::request::Mode;
use crate::request::Request;

/// More simultaneous panels than this are refused rather than queued.
pub const MAX_LIVE_PANELS: usize = 8;

/// `Request.Close()` from the portal frontend, delivered to both sides.
#[derive(Clone, Debug)]
pub struct Close {
    to_panel: Sender<()>,
    to_adapter: Sender<()>,
}

impl Close {
    pub fn close(&self) {
        let _ = self.to_panel.try_send(());
        let _ = self.to_adapter.try_send(());
    }
}

/// What the UI receives for one portal call.
pub struct PanelRequest {
    pub id: u64,
    pub request: Request,
    /// Loaded by the portal thread before the GPUI window is opened.
    pub recent_places: Vec<std::path::PathBuf>,
    /// Send exactly one outcome; dropping it without sending means Cancel.
    pub reply: Sender<Outcome>,
    /// Resolves when the frontend closes the request; the panel must close.
    pub closed: Receiver<()>,
}

#[derive(Clone, Debug)]
pub struct Broker {
    panels: Sender<PanelRequest>,
    live: Arc<AtomicUsize>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerError {
    Busy,
    Unavailable,
}

impl Broker {
    pub fn channel() -> (Self, Receiver<PanelRequest>) {
        let (panels, receiver) = async_channel::bounded(MAX_LIVE_PANELS);
        (
            Self {
                panels,
                live: Arc::new(AtomicUsize::new(0)),
                next_id: Arc::new(AtomicU64::new(1)),
            },
            receiver,
        )
    }

    /// Create the close pair for one request before it is exported, so a
    /// Close() that races the panel opening is never lost.
    pub fn close_pair() -> (Close, Receiver<()>, Receiver<()>) {
        let (to_panel, panel_closed) = async_channel::bounded(1);
        let (to_adapter, adapter_closed) = async_channel::bounded(1);
        (
            Close {
                to_panel,
                to_adapter,
            },
            panel_closed,
            adapter_closed,
        )
    }

    /// Present one panel and wait for the user or for Close().
    pub async fn present(
        &self,
        mut request: Request,
        panel_closed: Receiver<()>,
        adapter_closed: Receiver<()>,
    ) -> Result<Outcome, BrokerError> {
        let admitted = self
            .live
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < MAX_LIVE_PANELS).then_some(live + 1)
            })
            .is_ok();
        if !admitted {
            return Err(BrokerError::Busy);
        }
        let _guard = LiveGuard(self.live.clone());
        let app_id = request.app_id.clone();
        let mode = request.mode;
        let recent_places = recent::load(&app_id);
        if request.initial_folder().is_none() {
            request.current_folder = recent_places.first().cloned();
        }
        let (reply, outcome) = async_channel::bounded(1);
        let panel = PanelRequest {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            request,
            recent_places,
            reply,
            closed: panel_closed,
        };
        if self.panels.send(panel).await.is_err() {
            return Err(BrokerError::Unavailable);
        }
        let chosen = async { outcome.recv().await.unwrap_or(Outcome::Cancelled) };
        let closed = async {
            let _ = adapter_closed.recv().await;
            Outcome::Cancelled
        };
        let outcome = futures_lite::future::or(chosen, closed).await;
        if let Outcome::Chosen(selection) = &outcome {
            if let Some(path) = selection.paths.first() {
                let folder = if mode == Mode::Open && path.is_dir() {
                    Some(path.as_path())
                } else {
                    path.parent()
                };
                if let Some(folder) = folder {
                    recent::remember(&app_id, folder);
                }
            }
        }
        Ok(outcome)
    }
}

struct LiveGuard(Arc<AtomicUsize>);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
