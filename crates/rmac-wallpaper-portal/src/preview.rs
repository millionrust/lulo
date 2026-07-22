//! Framework-neutral state for the mandatory Wallpaper consent window.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use crate::broker::{
    PreviewEvent, PreviewEventKind, PreviewRequest, MAX_PARENT_WINDOW_BYTES,
    MAX_PENDING_PREVIEW_BYTES, MAX_PENDING_REQUESTS,
};
use crate::{Consent, RequestId};

pub const DIALOG_WIDTH: f64 = 620.0;
pub const DIALOG_HEIGHT: f64 = 500.0;
pub const PREVIEW_WIDTH: f64 = 560.0;
pub const PREVIEW_HEIGHT: f64 = 315.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    Cancel,
    Accept,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    AwaitingDecision,
    Resolving,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Escape,
    Enter,
    Space,
    Tab,
    BackTab,
    ArrowLeft,
    ArrowRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectReason {
    Duplicate,
    Capacity,
    InvalidIdentity,
    InvalidParent,
    InvalidImage,
    InvalidSourceSize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Update {
    Opened {
        id: RequestId,
    },
    Queued {
        id: RequestId,
        position: usize,
    },
    Closed {
        id: RequestId,
        next: Option<RequestId>,
    },
    RemovedQueued {
        id: RequestId,
    },
    Rejected {
        id: RequestId,
        reason: RejectReason,
    },
    Ignored {
        id: RequestId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOutcome {
    Ignored,
    FocusChanged(Control),
    Decision(Decision),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    pub id: RequestId,
    pub consent: Consent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Semantics {
    pub title: String,
    pub description: String,
    pub preview_label: String,
    pub cancel_label: String,
    pub accept_label: String,
    pub default_control: Control,
}

/// One visible modal model. It contains decoded pixels but never the source URI
/// or managed staging path.
pub struct Dialog {
    request: PreviewRequest,
    layout: rmac_wallpaper::Layout,
    phase: Phase,
    focused: Control,
}

impl Dialog {
    fn new(request: PreviewRequest) -> Result<Self, RejectReason> {
        validate(&request)?;
        let layout = rmac_wallpaper::layout(
            rmac_shell_settings::WallpaperFit::Fill,
            request.image().physical_size(),
            rmac_compositor::LogicalSize {
                width: PREVIEW_WIDTH,
                height: PREVIEW_HEIGHT,
            },
            1.0,
        )
        .map_err(|_| RejectReason::InvalidImage)?;
        Ok(Self {
            request,
            layout,
            phase: Phase::AwaitingDecision,
            focused: Control::Accept,
        })
    }

    pub fn id(&self) -> RequestId {
        self.request.id()
    }

    pub fn app_id(&self) -> &str {
        self.request.app_id()
    }

    pub fn parent_window(&self) -> &str {
        self.request.parent_window()
    }

    pub fn image(&self) -> &Arc<rmac_wallpaper_image::Decoded> {
        self.request.image()
    }

    pub fn source_bytes(&self) -> u64 {
        self.request.source_bytes()
    }

    pub fn format(&self) -> rmac_wallpaper_system::ImageFormat {
        self.request.format()
    }

    pub fn layout(&self) -> rmac_wallpaper::Layout {
        self.layout
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn focused(&self) -> Control {
        self.focused
    }

    pub fn semantics(&self) -> Semantics {
        Semantics {
            title: "Change Wallpaper?".into(),
            description: format!(
                "{} wants to change the wallpaper on every display. This replaces your per-display wallpaper choices.",
                self.app_id()
            ),
            preview_label: "Preview of the proposed wallpaper".into(),
            cancel_label: "Cancel".into(),
            accept_label: "Set Wallpaper".into(),
            default_control: Control::Accept,
        }
    }
}

impl fmt::Debug for Dialog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Dialog")
            .field("id", &self.id())
            .field("app_id", &self.app_id())
            .field("parent_window", &"<private>")
            .field("source", &"<private>")
            .field("source_bytes", &self.source_bytes())
            .field("format", &self.format())
            .field("width", &self.image().width)
            .field("height", &self.image().height)
            .field("phase", &self.phase)
            .field("focused", &self.focused)
            .finish()
    }
}

/// Serial modal presenter. Only one consent window is visible; later requests
/// retain FIFO order and external Close removes either the visible or queued
/// request without synthesizing a user decision.
#[derive(Default)]
pub struct Presenter {
    active: Option<Dialog>,
    queued: VecDeque<PreviewRequest>,
}

impl Presenter {
    pub fn apply(&mut self, event: PreviewEvent) -> Update {
        match event.into_kind() {
            PreviewEventKind::Open(request) => self.open(request),
            PreviewEventKind::Close { id } => self.close(id),
        }
    }

    pub fn active(&self) -> Option<&Dialog> {
        self.active.as_ref()
    }

    pub fn queued_count(&self) -> usize {
        self.queued.len()
    }

    pub fn input(&mut self, key: Key) -> InputOutcome {
        let Some(active) = self.active.as_mut() else {
            return InputOutcome::Ignored;
        };
        if active.phase != Phase::AwaitingDecision {
            return InputOutcome::Ignored;
        }
        match key {
            Key::Escape => self.begin(Consent::Cancel),
            Key::Enter | Key::Space => match active.focused {
                Control::Cancel => self.begin(Consent::Decline),
                Control::Accept => self.begin(Consent::Accept),
            },
            Key::Tab | Key::BackTab => {
                active.focused = opposite(active.focused);
                InputOutcome::FocusChanged(active.focused)
            }
            Key::ArrowLeft => {
                active.focused = Control::Cancel;
                InputOutcome::FocusChanged(active.focused)
            }
            Key::ArrowRight => {
                active.focused = Control::Accept;
                InputOutcome::FocusChanged(active.focused)
            }
        }
    }

    pub fn activate(&mut self, control: Control) -> InputOutcome {
        let Some(active) = self.active.as_mut() else {
            return InputOutcome::Ignored;
        };
        if active.phase != Phase::AwaitingDecision {
            return InputOutcome::Ignored;
        }
        active.focused = control;
        match control {
            Control::Cancel => self.begin(Consent::Decline),
            Control::Accept => self.begin(Consent::Accept),
        }
    }

    pub fn window_closed(&mut self) -> InputOutcome {
        self.begin(Consent::Cancel)
    }

    /// If broker delivery reports a stale request, remove the resolving modal
    /// locally; a successful delivery remains visible until its terminal Close
    /// event so the UI cannot expose the next request during an active commit.
    pub fn decision_delivery(&mut self, id: RequestId, delivered: bool) -> Option<Update> {
        if delivered {
            return None;
        }
        let matches = self
            .active
            .as_ref()
            .is_some_and(|active| active.id() == id && active.phase == Phase::Resolving);
        matches.then(|| self.close(id))
    }

    fn open(&mut self, request: PreviewRequest) -> Update {
        let id = request.id();
        if self.active.as_ref().is_some_and(|active| active.id() == id)
            || self.queued.iter().any(|queued| queued.id() == id)
        {
            return Update::Rejected {
                id,
                reason: RejectReason::Duplicate,
            };
        }
        if self
            .active
            .as_ref()
            .is_some_and(|_| self.queued.len() + 1 >= MAX_PENDING_REQUESTS)
        {
            return Update::Rejected {
                id,
                reason: RejectReason::Capacity,
            };
        }
        if let Err(reason) = validate(&request) {
            return Update::Rejected { id, reason };
        }
        if self.active.is_none() {
            self.active = Some(Dialog::new(request).expect("validated preview remains valid"));
            Update::Opened { id }
        } else {
            self.queued.push_back(request);
            Update::Queued {
                id,
                position: self.queued.len(),
            }
        }
    }

    fn close(&mut self, id: RequestId) -> Update {
        if self.active.as_ref().is_some_and(|active| active.id() == id) {
            self.active = self
                .queued
                .pop_front()
                .map(|request| Dialog::new(request).expect("queued preview was validated"));
            return Update::Closed {
                id,
                next: self.active.as_ref().map(Dialog::id),
            };
        }
        if let Some(index) = self.queued.iter().position(|request| request.id() == id) {
            self.queued.remove(index);
            return Update::RemovedQueued { id };
        }
        Update::Ignored { id }
    }

    fn begin(&mut self, consent: Consent) -> InputOutcome {
        let Some(active) = self.active.as_mut() else {
            return InputOutcome::Ignored;
        };
        if active.phase != Phase::AwaitingDecision {
            return InputOutcome::Ignored;
        }
        active.phase = Phase::Resolving;
        InputOutcome::Decision(Decision {
            id: active.id(),
            consent,
        })
    }
}

impl fmt::Debug for Presenter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Presenter")
            .field("active", &self.active)
            .field("queued", &self.queued.len())
            .finish()
    }
}

fn validate(request: &PreviewRequest) -> Result<(), RejectReason> {
    if request.app_id().is_empty()
        || request.app_id().len() > 256
        || request.app_id().chars().any(char::is_control)
    {
        return Err(RejectReason::InvalidIdentity);
    }
    if request.parent_window().len() > MAX_PARENT_WINDOW_BYTES
        || request.parent_window().chars().any(char::is_control)
    {
        return Err(RejectReason::InvalidParent);
    }
    if request.source_bytes() == 0
        || request.source_bytes() > rmac_wallpaper_system::MAX_WALLPAPER_BYTES
    {
        return Err(RejectReason::InvalidSourceSize);
    }
    let expected = usize::try_from(request.image().width)
        .ok()
        .and_then(|width| {
            usize::try_from(request.image().height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4));
    if expected != Some(request.image().rgba.len())
        || expected.is_none_or(|bytes| bytes > MAX_PENDING_PREVIEW_BYTES)
    {
        return Err(RejectReason::InvalidImage);
    }
    Ok(())
}

fn opposite(control: Control) -> Control {
    match control {
        Control::Cancel => Control::Accept,
        Control::Accept => Control::Cancel,
    }
}
