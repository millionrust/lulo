//! Bounded asynchronous request and mandatory-preview orchestration.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use async_channel::{Receiver, Sender};

use crate::{Consent, Importer, PortalResponse, Prepared, RequestId};

pub const MAX_PENDING_REQUESTS: usize = 8;
pub const MAX_PENDING_PREVIEW_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_PARENT_WINDOW_BYTES: usize = 1_024;
pub const MAX_URI_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_EVENTS: usize = MAX_PENDING_REQUESTS * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CancellationState {
    Active,
    Completing,
    Cancelled,
}

struct CancellationInner {
    state: Mutex<CancellationState>,
    wake: Sender<()>,
    receiver: Receiver<()>,
}

/// One idempotent cancellation capability shared by the D-Bus request object
/// and the broker future. The mutex gives Close and preview completion one
/// explicit linearization point, so an accepted request cannot be cancelled
/// halfway through its durable settings commit.
#[derive(Clone)]
pub struct Cancellation {
    inner: Arc<CancellationInner>,
}

impl Cancellation {
    pub fn new() -> Self {
        let (wake, receiver) = async_channel::bounded(1);
        Self {
            inner: Arc::new(CancellationInner {
                state: Mutex::new(CancellationState::Active),
                wake,
                receiver,
            }),
        }
    }

    /// Returns true only for the call which actually cancelled the request.
    pub fn cancel(&self) -> bool {
        let mut state = lock(&self.inner.state);
        if *state != CancellationState::Active {
            return false;
        }
        *state = CancellationState::Cancelled;
        let _ = self.inner.wake.try_send(());
        true
    }

    pub fn is_cancelled(&self) -> bool {
        *lock(&self.inner.state) == CancellationState::Cancelled
    }

    fn begin_completion(&self) -> bool {
        let mut state = lock(&self.inner.state);
        if *state != CancellationState::Active {
            return false;
        }
        *state = CancellationState::Completing;
        true
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let _ = self.inner.receiver.recv().await;
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Cancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Cancellation(<private>)")
    }
}

/// Renderer input for one mandatory consent window. Source URI and staging
/// path never cross this boundary.
#[derive(Clone)]
pub struct PreviewRequest {
    id: RequestId,
    app_id: String,
    parent_window: String,
    image: Arc<rmac_wallpaper_image::Decoded>,
    source_bytes: u64,
    format: rmac_wallpaper_system::ImageFormat,
}

impl PreviewRequest {
    pub(crate) fn new(
        id: RequestId,
        app_id: String,
        parent_window: String,
        image: Arc<rmac_wallpaper_image::Decoded>,
        source_bytes: u64,
        format: rmac_wallpaper_system::ImageFormat,
    ) -> Self {
        Self {
            id,
            app_id,
            parent_window,
            image,
            source_bytes,
            format,
        }
    }

    pub fn id(&self) -> RequestId {
        self.id
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn parent_window(&self) -> &str {
        &self.parent_window
    }

    pub fn image(&self) -> &Arc<rmac_wallpaper_image::Decoded> {
        &self.image
    }

    pub fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    pub fn format(&self) -> rmac_wallpaper_system::ImageFormat {
        self.format
    }
}

impl fmt::Debug for PreviewRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreviewRequest")
            .field("id", &self.id)
            .field("app_id", &self.app_id)
            .field("parent_window", &"<private>")
            .field("source", &"<private>")
            .field("source_bytes", &self.source_bytes)
            .field("format", &self.format)
            .field("width", &self.image.width)
            .field("height", &self.image.height)
            .finish()
    }
}

/// Public payload of one ordered presentation lifecycle event.
#[derive(Clone, Debug)]
pub enum PreviewEventKind {
    Open(PreviewRequest),
    Close { id: RequestId },
}

/// An event retains the request's admission lease through terminal Close
/// delivery. A stalled consumer therefore stops new admission instead of
/// allowing old dismissal events to be overwritten.
#[derive(Clone)]
pub struct PreviewEvent {
    kind: PreviewEventKind,
    _close_lease: Option<Arc<CloseLease>>,
}

impl PreviewEvent {
    pub fn kind(&self) -> &PreviewEventKind {
        &self.kind
    }

    pub fn into_kind(self) -> PreviewEventKind {
        self.kind
    }

    pub(crate) fn open(request: PreviewRequest) -> Self {
        Self {
            kind: PreviewEventKind::Open(request),
            _close_lease: None,
        }
    }

    fn close(id: RequestId, permit: Permit) -> Self {
        Self {
            kind: PreviewEventKind::Close { id },
            _close_lease: Some(Arc::new(CloseLease { _permit: permit })),
        }
    }
}

impl fmt::Debug for PreviewEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

struct Pending {
    prepared: Prepared,
    decision: Option<Sender<Consent>>,
    preview_bytes: usize,
}

#[derive(Default)]
struct PendingState {
    requests: BTreeMap<RequestId, Pending>,
    preview_bytes: usize,
}

struct Inner {
    importer: Importer,
    previews: Sender<PreviewEvent>,
    permits: Sender<()>,
    available_permits: Receiver<()>,
    // Decoding one source at a time prevents several maximum-size images from
    // allocating concurrently before the retained-preview budget can apply.
    prepare: Mutex<()>,
    pending: Mutex<PendingState>,
}

/// Cloneable authority used by both the D-Bus method and the preview process.
#[derive(Clone)]
pub struct Broker {
    inner: Arc<Inner>,
}

impl Broker {
    pub fn new(importer: Importer) -> (Self, Receiver<PreviewEvent>) {
        // One Open and one terminal Close can be retained for every admitted
        // request without making cancellation wait for a stalled UI process.
        let (previews, receiver) = async_channel::bounded(MAX_PREVIEW_EVENTS);
        let (permits, available_permits) = async_channel::bounded(MAX_PENDING_REQUESTS);
        for () in std::iter::repeat_n((), MAX_PENDING_REQUESTS) {
            permits
                .try_send(())
                .expect("fresh request permit queue has exact capacity");
        }
        (
            Self {
                inner: Arc::new(Inner {
                    importer,
                    previews,
                    permits,
                    available_permits,
                    prepare: Mutex::new(()),
                    pending: Mutex::new(PendingState::default()),
                }),
            },
            receiver,
        )
    }

    /// Runs one request through frozen decode, mandatory preview, and durable
    /// commit. Admission is nonblocking: overload returns response 2 instead of
    /// allowing unbounded queued file reads or dialogs.
    pub async fn request(
        &self,
        request: rmac_wallpaper::portal::Request,
        parent_window: String,
        cancellation: Cancellation,
    ) -> PortalResponse {
        if !valid_parent_window(&parent_window) || request.uri.len() > MAX_URI_BYTES {
            return PortalResponse::Other;
        }
        let Ok(()) = self.inner.available_permits.try_recv() else {
            return PortalResponse::Other;
        };
        let permit = Permit {
            sender: self.inner.permits.clone(),
        };
        if cancellation.is_cancelled() {
            return PortalResponse::Cancelled;
        }

        let importer = self.inner.importer.clone();
        let prepare = self.inner.clone();
        let prepare_cancellation = cancellation.clone();
        let prepared = blocking::unblock(move || {
            let _serial = lock(&prepare.prepare);
            if prepare_cancellation.is_cancelled() {
                return None;
            }
            Some(importer.prepare(request))
        })
        .await;
        let Some(prepared) = prepared else {
            return PortalResponse::Cancelled;
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return error.response(),
        };
        if cancellation.is_cancelled() {
            return cancel_prepared(self.inner.importer.clone(), prepared).await;
        }

        let id = prepared.id();
        let preview_bytes = prepared.image().rgba.len();
        let (decision, decisions) = async_channel::bounded(1);
        {
            let mut pending = lock(&self.inner.pending);
            let Some(total) = pending.preview_bytes.checked_add(preview_bytes) else {
                return PortalResponse::Other;
            };
            if total > MAX_PENDING_PREVIEW_BYTES || pending.requests.contains_key(&id) {
                return PortalResponse::Other;
            }
            let preview = PreviewRequest::new(
                id,
                prepared.app_id().to_owned(),
                parent_window,
                prepared.image().clone(),
                prepared.byte_len(),
                prepared.format(),
            );
            pending.preview_bytes = total;
            pending.requests.insert(
                id,
                Pending {
                    prepared,
                    decision: Some(decision),
                    preview_bytes,
                },
            );
            // The event queue reserves both an Open and terminal Close slot for
            // each retained admission lease. Only receiver shutdown can make
            // this initial publication fail while the invariant holds.
            if self
                .inner
                .previews
                .try_send(PreviewEvent::open(preview))
                .is_err()
            {
                remove_pending(&mut pending, id);
                return PortalResponse::Other;
            }
        }

        let mut guard = PendingGuard {
            inner: self.inner.clone(),
            id: Some(id),
            permit: Some(permit),
        };
        enum Wake {
            Decision(Result<Consent, async_channel::RecvError>),
            Cancelled,
        }
        let wake =
            futures_lite::future::race(async { Wake::Decision(decisions.recv().await) }, async {
                cancellation.cancelled().await;
                Wake::Cancelled
            })
            .await;
        let consent = match wake {
            Wake::Decision(Ok(consent)) if cancellation.begin_completion() => consent,
            Wake::Decision(Ok(_)) | Wake::Cancelled => Consent::Cancel,
            Wake::Decision(Err(_)) => Consent::Cancel,
        };
        let Some(prepared) = guard.take() else {
            return PortalResponse::Other;
        };
        let importer = self.inner.importer.clone();
        blocking::unblock(move || importer.finish(prepared, consent))
            .await
            .map(|outcome| outcome.response())
            .unwrap_or_else(|error| error.response())
    }

    /// Resolve a live preview exactly once. Stale, duplicated, and replayed
    /// decisions are inert.
    pub fn decide(&self, id: RequestId, consent: Consent) -> bool {
        let mut pending = lock(&self.inner.pending);
        let Some(request) = pending.requests.get_mut(&id) else {
            return false;
        };
        let Some(decision) = request.decision.take() else {
            return false;
        };
        decision.try_send(consent).is_ok()
    }

    pub fn pending_count(&self) -> usize {
        lock(&self.inner.pending).requests.len()
    }
}

impl fmt::Debug for Broker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Broker")
            .field("pending", &self.pending_count())
            .finish_non_exhaustive()
    }
}

struct Permit {
    sender: Sender<()>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        // Service shutdown can drop the receiving authority before a queued UI
        // event releases its lease. Losing that final token is harmless because
        // the broker is already gone; Drop must never panic during teardown.
        let _ = self.sender.try_send(());
    }
}

struct CloseLease {
    _permit: Permit,
}

struct PendingGuard {
    inner: Arc<Inner>,
    id: Option<RequestId>,
    permit: Option<Permit>,
}

impl PendingGuard {
    fn take(&mut self) -> Option<Prepared> {
        let id = self.id.take()?;
        let mut pending = lock(&self.inner.pending);
        let request = remove_pending(&mut pending, id)?;
        drop(pending);
        let permit = self
            .permit
            .take()
            .expect("a live pending request owns one admission permit");
        let _ = self
            .inner
            .previews
            .try_send(PreviewEvent::close(id, permit));
        Some(request.prepared)
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let removed = remove_pending(&mut lock(&self.inner.pending), id).is_some();
        if removed {
            let permit = self
                .permit
                .take()
                .expect("a live pending request owns one admission permit");
            let _ = self
                .inner
                .previews
                .try_send(PreviewEvent::close(id, permit));
        }
    }
}

fn remove_pending(pending: &mut PendingState, id: RequestId) -> Option<Pending> {
    let request = pending.requests.remove(&id)?;
    pending.preview_bytes = pending
        .preview_bytes
        .checked_sub(request.preview_bytes)
        .expect("retained preview byte accounting must remain exact");
    Some(request)
}

async fn cancel_prepared(importer: Importer, prepared: Prepared) -> PortalResponse {
    blocking::unblock(move || importer.finish(prepared, Consent::Cancel))
        .await
        .map(|outcome| outcome.response())
        .unwrap_or_else(|error| error.response())
}

fn valid_parent_window(parent: &str) -> bool {
    parent.len() <= MAX_PARENT_WINDOW_BYTES && !parent.chars().any(char::is_control)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
