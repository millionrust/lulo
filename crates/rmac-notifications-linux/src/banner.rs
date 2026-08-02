//! Authority-bound E2 banner session for the eventual Linux layer surface.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use rmac_notifications::banner::{Config, PlacementPolicy, Schedule, Snapshot as LayoutSnapshot};
use rmac_notifications::{AppId, Notification, NotificationId, PostOutcome, Time};
use rmac_notifications_runtime::presentation::{
    Announcement, Card, ControlId, Effect as PresentationEffect, Error as PresentationError,
    Intent, Key, LiveRegion, Presenter,
};
use rmac_notifications_runtime::{
    Command as RuntimeCommand, Coordinator, Error as RuntimeError, Update as RuntimeUpdate,
};

use crate::media::{CustomSound, Icon, NotificationMedia, SoundFormat};
use crate::service::{ActionError, ActionSelection, RuntimeEvent, ServiceHandle};

pub const MAX_PENDING_POSTS: usize = 500;
pub const MAX_APPLICATION_NAMES: usize = 4_096;
pub const MAX_RETAINED_FEEDBACK: usize = 16;
const MAX_ACTIVATION_TOKEN_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Selection {
    Default,
    Button(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceRequest {
    Expire(NotificationId),
    Dismiss(NotificationId),
    Invoke {
        control: ControlId,
        notification: NotificationId,
        selection: Selection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostCommand {
    Redraw,
    CapturePreviousFocus,
    RestorePreviousFocus,
    Service(ServiceRequest),
}

#[derive(Clone, Eq, PartialEq)]
pub enum SoundCue {
    Default(NotificationId),
    Custom {
        notification: NotificationId,
        sound: CustomSound,
    },
}

impl fmt::Debug for SoundCue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default(id) => formatter.debug_tuple("Default").field(id).finish(),
            Self::Custom {
                notification,
                sound,
            } => formatter
                .debug_struct("Custom")
                .field("notification", notification)
                .field("sound", sound)
                .finish(),
        }
    }
}

impl SoundCue {
    pub fn notification(&self) -> NotificationId {
        match self {
            Self::Default(notification) | Self::Custom { notification, .. } => *notification,
        }
    }
}

#[derive(Clone)]
pub struct SoundPlayer {
    permits: async_channel::Receiver<()>,
    returns: async_channel::Sender<()>,
}

impl Default for SoundPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundPlayer {
    pub fn new() -> Self {
        let (returns, permits) = async_channel::bounded(1);
        let inserted = returns.try_send(()).is_ok();
        debug_assert!(inserted, "fresh sound player has one permit");
        Self { permits, returns }
    }

    /// Plays at most one notification sound at once. A concurrent cue fails
    /// explicitly instead of multiplying decoder/process memory during a
    /// notification flood. Dropping a cancelled future returns its permit.
    pub async fn play(&self, cue: &SoundCue) -> Result<(), SoundPlaybackError> {
        let _permit = self.acquire()?;
        let sound = match cue {
            SoundCue::Default(_) => rmac_audio::NotificationSound::Default,
            SoundCue::Custom { sound, .. } => rmac_audio::NotificationSound::Encoded {
                format: notification_sound_format(sound.format),
                bytes: sound.bytes(),
            },
        };
        rmac_audio::play_notification_sound(sound)
            .await
            .map_err(|error| SoundPlaybackError::Playback(error.kind()))
    }

    fn acquire(&self) -> Result<SoundPermit, SoundPlaybackError> {
        self.permits
            .try_recv()
            .map_err(|_| SoundPlaybackError::Busy)?;
        Ok(SoundPermit {
            returns: self.returns.clone(),
        })
    }
}

impl fmt::Debug for SoundPlayer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SoundPlayer")
            .field("available", &(!self.permits.is_empty()))
            .finish()
    }
}

struct SoundPermit {
    returns: async_channel::Sender<()>,
}

impl Drop for SoundPermit {
    fn drop(&mut self) {
        let returned = self.returns.try_send(()).is_ok();
        debug_assert!(returned, "one sound permit is returned exactly once");
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoundPlaybackError {
    Busy,
    Playback(rmac_audio::NotificationPlaybackErrorKind),
}

impl fmt::Display for SoundPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "notification sound failed ({self:?})")
    }
}

impl std::error::Error for SoundPlaybackError {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FeedbackId(u64);

impl FeedbackId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackOperation {
    Action,
    Dismiss,
    Expire,
    Sound,
}

impl FeedbackOperation {
    pub fn title(self) -> &'static str {
        match self {
            Self::Action => "Notification action failed",
            Self::Dismiss => "Could not dismiss notification",
            Self::Expire => "Notification could not close",
            Self::Sound => "Notification sound could not play",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackReason {
    NoLongerAvailable,
    InvalidRequest,
    ConnectionLost,
    ServiceUnavailable,
    Rejected,
    Busy,
    InvalidMedia,
    Unsupported,
    TimedOut,
    Failed,
}

impl FeedbackReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::NoLongerAvailable => "The notification is no longer available.",
            Self::InvalidRequest => "The request was invalid.",
            Self::ConnectionLost => "The notification service connection was lost.",
            Self::ServiceUnavailable => "The required notification service is unavailable.",
            Self::Rejected => "The request was rejected.",
            Self::Busy => "Another notification sound is already playing.",
            Self::InvalidMedia => "The notification sound was invalid.",
            Self::Unsupported => "Sound playback is unavailable on this system.",
            Self::TimedOut => "Sound playback took too long and was stopped.",
            Self::Failed => "The operation did not complete.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Feedback {
    pub id: FeedbackId,
    pub notification: NotificationId,
    pub operation: FeedbackOperation,
    pub reason: FeedbackReason,
}

impl Feedback {
    pub fn live_region(self) -> LiveRegion {
        LiveRegion::Polite
    }

    pub fn dismiss_label(self) -> &'static str {
        "Dismiss error"
    }

    /// Private-content-free fallback for an error toast or accessible status.
    /// Renderers may localize the operation and reason instead.
    pub fn accessible_message(self) -> String {
        format!("{}. {}", self.operation.title(), self.reason.message())
    }
}

fn notification_sound_format(format: SoundFormat) -> rmac_audio::NotificationSoundFormat {
    match format {
        SoundFormat::OggOpus => rmac_audio::NotificationSoundFormat::OggOpus,
        SoundFormat::OggVorbis => rmac_audio::NotificationSoundFormat::OggVorbis,
        SoundFormat::WavPcm => rmac_audio::NotificationSoundFormat::WavPcm,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub commands: Vec<HostCommand>,
    pub announcements: Vec<Announcement>,
    pub sounds: Vec<SoundCue>,
    pub focus_released: bool,
    pub schedule: Schedule,
}

impl Update {
    fn idle(schedule: Schedule) -> Self {
        Self {
            commands: Vec::new(),
            announcements: Vec::new(),
            sounds: Vec::new(),
            focus_released: false,
            schedule,
        }
    }

    fn merge(&mut self, mut other: Self) {
        for command in other.commands.drain(..) {
            self.push_command(command);
        }
        self.announcements.append(&mut other.announcements);
        self.sounds.append(&mut other.sounds);
        self.focus_released |= other.focus_released;
        self.schedule = other.schedule;
    }

    fn push_command(&mut self, command: HostCommand) {
        if command == HostCommand::Redraw {
            if !self.commands.contains(&HostCommand::Redraw) {
                self.commands.insert(0, command);
            }
        } else {
            self.commands.push(command);
        }
    }
}

pub struct Frame<'a> {
    pub layout: LayoutSnapshot,
    pub cards: &'a [Card],
    pub feedback: &'a [Feedback],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Runtime(RuntimeError),
    Presentation(PresentationError),
    MissingNotification,
    MismatchedNotification,
    TooManyPendingPosts,
    TooManyApplicationNames,
    InvalidApplicationIdentity,
    InvalidApplicationName,
    StaleCompletion,
    FeedbackIdExhausted,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "notification banner session failed ({self:?})")
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceErrorKind {
    InvalidActivationToken,
    Action(ActionError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceExecutionError {
    pub request: ServiceRequest,
    pub kind: ServiceErrorKind,
}

impl fmt::Display for ServiceExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "notification banner service operation failed ({:?})",
            self.kind
        )
    }
}

impl std::error::Error for ServiceExecutionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCompletion {
    Closed(ServiceRequest),
    Invoked {
        control: ControlId,
        notification: NotificationId,
        notification_remains: bool,
    },
}

struct PendingPost {
    outcome: PostOutcome,
    notification: Box<Notification>,
    media: NotificationMedia,
}

enum PresentPostError {
    NoOutput(PendingPost),
    Other(Error),
}

/// Complete framework-independent state for one future layer-surface host.
/// The host only renders [`Frame`], plays [`SoundCue`], and executes typed
/// commands; it never reconstructs notification policy or action identity.
pub struct BannerSession {
    coordinator: Coordinator,
    presenter: Presenter,
    presented: BTreeMap<NotificationId, Notification>,
    icons: BTreeMap<NotificationId, Icon>,
    application_names: BTreeMap<String, String>,
    pending: VecDeque<PendingPost>,
    feedback: Vec<Feedback>,
    next_feedback: u64,
}

impl BannerSession {
    pub fn new(
        config: Config,
        placement: PlacementPolicy,
        appearance: &rmac_appearance::Snapshot,
    ) -> Result<Self, Error> {
        Ok(Self {
            coordinator: Coordinator::new(config, placement, appearance).map_err(Error::Runtime)?,
            presenter: Presenter::default(),
            presented: BTreeMap::new(),
            icons: BTreeMap::new(),
            application_names: BTreeMap::new(),
            pending: VecDeque::new(),
            feedback: Vec::new(),
            next_feedback: 0,
        })
    }

    pub fn frame(&self) -> Frame<'_> {
        Frame {
            layout: self.coordinator.snapshot(),
            cards: self.presenter.cards(),
            feedback: &self.feedback,
        }
    }

    /// Produces the exact current per-output layer-surface transaction from
    /// renderer measurements. Focus policy always comes from this session's
    /// presenter rather than being reconstructed by the host.
    pub fn surface_plan(
        &self,
        compositor: &rmac_compositor::Snapshot,
        measurements: &[crate::surfaces::CardMeasurement],
    ) -> Result<crate::surfaces::Plan, crate::surfaces::PlanError> {
        crate::surfaces::plan(&self.frame(), compositor, measurements, self.focused())
    }

    pub fn icon(&self, id: NotificationId) -> Option<&Icon> {
        self.icons.get(&id)
    }

    /// Builds one exact worker input from the retained card identity and icon.
    /// A missing card is inert; callers never reconstruct application identity
    /// from visible text.
    pub fn icon_source(
        &self,
        id: NotificationId,
        request: crate::icon::Request,
    ) -> Result<Option<crate::icon_worker::Source>, crate::icon_worker::RequestError> {
        let Some(card) = self.presenter.cards().iter().find(|card| card.id == id) else {
            return Ok(None);
        };
        crate::icon_worker::Source::new(
            id,
            card.app_id.clone(),
            self.icons.get(&id).cloned(),
            request,
        )
        .map(Some)
    }

    pub fn focused(&self) -> Option<ControlId> {
        self.presenter.focused()
    }

    pub fn pending_control(&self) -> Option<ControlId> {
        self.presenter.pending()
    }

    pub fn pending_post_count(&self) -> usize {
        self.pending.len()
    }

    /// Exact current semantic tree for the future Linux accessibility host.
    /// `announcements` must be the announcement slice emitted with the update
    /// that caused this frame; replacements are never guessed from content.
    pub fn accessibility_snapshot(
        &self,
        announcements: &[Announcement],
    ) -> Result<
        crate::accessibility::BannerAccessibilitySnapshot,
        crate::accessibility::AccessibilityProjectionError,
    > {
        crate::accessibility::project_banner_accessibility(
            &self.frame(),
            self.focused(),
            self.pending_control(),
            announcements,
        )
    }

    pub fn apply_event(&mut self, event: RuntimeEvent, now: Time) -> Result<Update, Error> {
        match event {
            RuntimeEvent::Posted {
                outcome,
                notification,
                media,
            } => self.apply_post(outcome, notification, media, now),
            RuntimeEvent::Closed(closed) => {
                self.pending.retain(|post| post.outcome.id != closed.id);
                self.presented.remove(&closed.id);
                let runtime = self.coordinator.apply_closed(closed.id, now);
                self.finish(runtime, &[], Vec::new(), now)
            }
            RuntimeEvent::ActionInvoked(_) => Ok(self.idle(now)),
        }
    }

    pub fn apply_compositor(
        &mut self,
        compositor: &rmac_compositor::Snapshot,
        now: Time,
    ) -> Result<Update, Error> {
        let runtime = match self.coordinator.apply_compositor(compositor, now) {
            Ok(update) => update,
            Err(RuntimeError::NoOutput) => return Ok(self.idle(now)),
            Err(error) => return Err(Error::Runtime(error)),
        };
        let mut update = self.finish(runtime, &[], Vec::new(), now)?;
        let mut queued = std::mem::take(&mut self.pending);
        while let Some(post) = queued.pop_front() {
            match self.present_post(post, now) {
                Ok(next) => update.merge(next),
                Err(PresentPostError::NoOutput(post)) => {
                    self.pending.push_back(post);
                    self.pending.append(&mut queued);
                    break;
                }
                Err(PresentPostError::Other(error)) => {
                    self.pending.append(&mut queued);
                    return Err(error);
                }
            }
        }
        update.schedule = self.coordinator.schedule(now);
        Ok(update)
    }

    pub fn apply_appearance(
        &mut self,
        appearance: &rmac_appearance::Snapshot,
        now: Time,
    ) -> Result<Update, Error> {
        let runtime = self.coordinator.apply_appearance(appearance, now);
        self.finish(runtime, &[], Vec::new(), now)
    }

    pub fn set_application_names(
        &mut self,
        application_names: BTreeMap<String, String>,
        now: Time,
    ) -> Result<Update, Error> {
        validate_application_names(&application_names)?;
        if application_names == self.application_names {
            return Ok(self.idle(now));
        }
        let previous_cards = self.presenter.cards().to_vec();
        let outcome = self
            .presenter
            .sync(
                &self.coordinator.snapshot(),
                &self.presented_values(),
                &application_names,
                &[],
            )
            .map_err(Error::Presentation)?;
        self.application_names = application_names;
        self.prune_to_cards();
        let mut update = self.idle(now);
        update.announcements = outcome.announcements;
        update.focus_released = outcome.focus_released;
        if self.presenter.cards() != previous_cards {
            update.push_command(HostCommand::Redraw);
        }
        Ok(update)
    }

    pub fn set_hovered(
        &mut self,
        id: NotificationId,
        hovered: bool,
        now: Time,
    ) -> Result<Update, Error> {
        let runtime = self
            .coordinator
            .set_hovered(id, hovered, now)
            .map_err(Error::Runtime)?;
        self.finish(runtime, &[], Vec::new(), now)
    }

    pub fn select(&mut self, control: ControlId, now: Time) -> Result<Update, Error> {
        let effect = self.presenter.select(control);
        self.apply_presentation_effect(effect, now)
    }

    pub fn handle_key(&mut self, key: Key, now: Time) -> Result<Update, Error> {
        let effect = self.presenter.handle_key(key);
        self.apply_presentation_effect(effect, now)
    }

    pub fn activate_control(&mut self, control: ControlId, now: Time) -> Result<Update, Error> {
        let effect = self.presenter.activate_control(control);
        self.apply_presentation_effect(effect, now)
    }

    pub fn advance(&mut self, now: Time) -> Result<Update, Error> {
        let runtime = self.coordinator.advance(now);
        self.finish(runtime, &[], Vec::new(), now)
    }

    pub fn complete_service(
        &mut self,
        completion: ServiceCompletion,
        now: Time,
    ) -> Result<Update, Error> {
        match completion {
            ServiceCompletion::Closed(request) => {
                let (notification, operation) = service_feedback_target(request);
                let mut update = self.idle(now);
                if self.clear_feedback(notification, operation) {
                    update.push_command(HostCommand::Redraw);
                }
                Ok(update)
            }
            ServiceCompletion::Invoked {
                control,
                notification,
                notification_remains,
            } => {
                if !self.presenter.complete(control) {
                    let card_still_exists = self
                        .presenter
                        .cards()
                        .iter()
                        .any(|card| card.id == notification);
                    if self.presenter.pending().is_some() || card_still_exists {
                        return Err(Error::StaleCompletion);
                    }
                }
                // The service publishes ActionInvoked/Closed before its future
                // returns. If the event consumer already reconciled that
                // close, do not restart or relabel the terminal animation.
                if !notification_remains && !self.presented.contains_key(&notification) {
                    let mut update = self.idle(now);
                    self.clear_feedback(notification, FeedbackOperation::Action);
                    update.push_command(HostCommand::Redraw);
                    return Ok(update);
                }
                let runtime = self
                    .coordinator
                    .action_completed(notification, notification_remains, now)
                    .map_err(Error::Runtime)?;
                let mut update = self.finish(runtime, &[], Vec::new(), now)?;
                self.clear_feedback(notification, FeedbackOperation::Action);
                update.push_command(HostCommand::Redraw);
                Ok(update)
            }
        }
    }

    pub fn fail_service(
        &mut self,
        error: ServiceExecutionError,
        now: Time,
    ) -> Result<Update, Error> {
        if let ServiceRequest::Invoke {
            control,
            notification,
            ..
        } = error.request
        {
            if !self.presenter.complete(control) {
                let card_still_exists = self
                    .presenter
                    .cards()
                    .iter()
                    .any(|card| card.id == notification);
                if self.presenter.pending().is_some() || card_still_exists {
                    return Err(Error::StaleCompletion);
                }
            }
        }
        let (notification, operation) = service_feedback_target(error.request);
        self.push_feedback(notification, operation, service_feedback_reason(error.kind))?;
        let mut update = self.idle(now);
        update.push_command(HostCommand::Redraw);
        Ok(update)
    }

    pub fn fail_sound(
        &mut self,
        cue: &SoundCue,
        error: SoundPlaybackError,
        now: Time,
    ) -> Result<Update, Error> {
        self.push_feedback(
            cue.notification(),
            FeedbackOperation::Sound,
            sound_feedback_reason(error),
        )?;
        let mut update = self.idle(now);
        update.push_command(HostCommand::Redraw);
        Ok(update)
    }

    pub fn dismiss_feedback(&mut self, id: FeedbackId, now: Time) -> Update {
        let before = self.feedback.len();
        self.feedback.retain(|feedback| feedback.id != id);
        let mut update = self.idle(now);
        if self.feedback.len() != before {
            update.push_command(HostCommand::Redraw);
        }
        update
    }

    fn apply_post(
        &mut self,
        outcome: PostOutcome,
        notification: Option<Box<Notification>>,
        media: NotificationMedia,
        now: Time,
    ) -> Result<Update, Error> {
        if !outcome.delivery.banner {
            self.pending.retain(|post| post.outcome.id != outcome.id);
            let runtime = self
                .coordinator
                .apply_post(outcome, &[], now)
                .map_err(Error::Runtime)?;
            return self.finish(runtime, &[], Vec::new(), now);
        }
        let notification = notification.ok_or(Error::MissingNotification)?;
        if notification.id != outcome.id {
            return Err(Error::MismatchedNotification);
        }
        let post = PendingPost {
            outcome,
            notification,
            media,
        };
        match self.present_post(post, now) {
            Ok(update) => Ok(update),
            Err(PresentPostError::NoOutput(post)) => {
                self.queue(post)?;
                Ok(self.idle(now))
            }
            Err(PresentPostError::Other(error)) => Err(error),
        }
    }

    fn present_post(
        &mut self,
        mut post: PendingPost,
        now: Time,
    ) -> Result<Update, PresentPostError> {
        let runtime = match self.coordinator.apply_post(
            post.outcome,
            std::slice::from_ref(post.notification.as_ref()),
            now,
        ) {
            Ok(runtime) => runtime,
            Err(RuntimeError::NoOutput) => return Err(PresentPostError::NoOutput(post)),
            Err(error) => return Err(PresentPostError::Other(Error::Runtime(error))),
        };
        self.pending
            .retain(|pending| pending.outcome.id != post.outcome.id);
        self.clear_notification_feedback(post.outcome.id);
        self.presented
            .insert(post.outcome.id, (*post.notification).clone());
        if let Some(icon) = post.media.icon.take() {
            self.icons.insert(post.outcome.id, icon);
        } else {
            self.icons.remove(&post.outcome.id);
        }
        let sounds = if post.outcome.delivery.sound {
            vec![match post.media.sound.take() {
                Some(sound) => SoundCue::Custom {
                    notification: post.outcome.id,
                    sound,
                },
                None => SoundCue::Default(post.outcome.id),
            }]
        } else {
            Vec::new()
        };
        let reannounce = post
            .outcome
            .announce_as_new
            .then_some(post.outcome.id)
            .into_iter()
            .collect::<Vec<_>>();
        self.finish(runtime, &reannounce, sounds, now)
            .map_err(PresentPostError::Other)
    }

    fn queue(&mut self, post: PendingPost) -> Result<(), Error> {
        self.pending
            .retain(|pending| pending.outcome.id != post.outcome.id);
        if self.pending.len() >= MAX_PENDING_POSTS {
            return Err(Error::TooManyPendingPosts);
        }
        self.pending.push_back(post);
        Ok(())
    }

    fn apply_presentation_effect(
        &mut self,
        effect: PresentationEffect,
        now: Time,
    ) -> Result<Update, Error> {
        match effect {
            PresentationEffect::None => Ok(self.idle(now)),
            PresentationEffect::FocusChanged { notification, .. } => {
                let runtime = self
                    .coordinator
                    .set_focus(Some(notification), now)
                    .map_err(Error::Runtime)?;
                self.finish(runtime, &[], Vec::new(), now)
            }
            PresentationEffect::ReleaseFocus => {
                let runtime = self
                    .coordinator
                    .set_focus(None, now)
                    .map_err(Error::Runtime)?;
                self.finish(runtime, &[], Vec::new(), now)
            }
            PresentationEffect::Activate { control, intent } => match intent {
                Intent::InvokeDefault(notification) => {
                    self.clear_feedback(notification, FeedbackOperation::Action);
                    Ok(self.invoke_update(control, notification, Selection::Default, now))
                }
                Intent::InvokeButton {
                    notification,
                    index,
                } => {
                    self.clear_feedback(notification, FeedbackOperation::Action);
                    Ok(self.invoke_update(control, notification, Selection::Button(index), now))
                }
                Intent::Dismiss(notification) => {
                    self.clear_feedback(notification, FeedbackOperation::Dismiss);
                    let runtime = self
                        .coordinator
                        .request_dismiss(notification, now)
                        .map_err(Error::Runtime)?;
                    self.finish(runtime, &[], Vec::new(), now)
                }
            },
        }
    }

    fn invoke_update(
        &self,
        control: ControlId,
        notification: NotificationId,
        selection: Selection,
        now: Time,
    ) -> Update {
        let mut update = self.idle(now);
        update.push_command(HostCommand::Redraw);
        update.push_command(HostCommand::Service(ServiceRequest::Invoke {
            control,
            notification,
            selection,
        }));
        update
    }

    fn finish(
        &mut self,
        runtime: RuntimeUpdate,
        reannounce: &[NotificationId],
        sounds: Vec<SoundCue>,
        now: Time,
    ) -> Result<Update, Error> {
        let outcome = self
            .presenter
            .sync(
                &self.coordinator.snapshot(),
                &self.presented_values(),
                &self.application_names,
                reannounce,
            )
            .map_err(Error::Presentation)?;
        self.prune_to_cards();
        let mut update = Update {
            commands: runtime.commands.into_iter().map(host_command).collect(),
            announcements: outcome.announcements,
            sounds,
            focus_released: outcome.focus_released,
            schedule: runtime.schedule,
        };
        update.schedule = self.coordinator.schedule(now);
        Ok(update)
    }

    fn idle(&self, now: Time) -> Update {
        Update::idle(self.coordinator.schedule(now))
    }

    fn presented_values(&self) -> Vec<Notification> {
        self.presented.values().cloned().collect()
    }

    fn prune_to_cards(&mut self) {
        let visible = self
            .presenter
            .cards()
            .iter()
            .map(|card| card.id)
            .collect::<BTreeSet<_>>();
        self.presented.retain(|id, _| visible.contains(id));
        self.icons.retain(|id, _| visible.contains(id));
    }

    fn push_feedback(
        &mut self,
        notification: NotificationId,
        operation: FeedbackOperation,
        reason: FeedbackReason,
    ) -> Result<FeedbackId, Error> {
        let next = self
            .next_feedback
            .checked_add(1)
            .ok_or(Error::FeedbackIdExhausted)?;
        self.next_feedback = next;
        let id = FeedbackId(next);
        self.feedback.retain(|feedback| {
            feedback.notification != notification || feedback.operation != operation
        });
        if self.feedback.len() >= MAX_RETAINED_FEEDBACK {
            self.feedback.remove(0);
        }
        self.feedback.push(Feedback {
            id,
            notification,
            operation,
            reason,
        });
        Ok(id)
    }

    fn clear_feedback(
        &mut self,
        notification: NotificationId,
        operation: FeedbackOperation,
    ) -> bool {
        let before = self.feedback.len();
        self.feedback.retain(|feedback| {
            feedback.notification != notification || feedback.operation != operation
        });
        self.feedback.len() != before
    }

    fn clear_notification_feedback(&mut self, notification: NotificationId) -> bool {
        let before = self.feedback.len();
        self.feedback
            .retain(|feedback| feedback.notification != notification);
        self.feedback.len() != before
    }
}

impl fmt::Debug for BannerSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BannerSession")
            .field("coordinator", &self.coordinator)
            .field("presenter", &self.presenter)
            .field("presented", &self.presented.len())
            .field("icons", &self.icons.len())
            .field("application_names", &self.application_names.len())
            .field("pending", &self.pending.len())
            .field("feedback", &self.feedback.len())
            .finish()
    }
}

fn service_feedback_target(request: ServiceRequest) -> (NotificationId, FeedbackOperation) {
    match request {
        ServiceRequest::Expire(notification) => (notification, FeedbackOperation::Expire),
        ServiceRequest::Dismiss(notification) => (notification, FeedbackOperation::Dismiss),
        ServiceRequest::Invoke { notification, .. } => (notification, FeedbackOperation::Action),
    }
}

fn service_feedback_reason(kind: ServiceErrorKind) -> FeedbackReason {
    match kind {
        ServiceErrorKind::InvalidActivationToken => FeedbackReason::InvalidRequest,
        ServiceErrorKind::Action(ActionError::UnknownNotification) => {
            FeedbackReason::NoLongerAvailable
        }
        ServiceErrorKind::Action(
            ActionError::UnknownAction
            | ActionError::InvalidTarget
            | ActionError::InvalidApplication,
        ) => FeedbackReason::InvalidRequest,
        ServiceErrorKind::Action(ActionError::DocumentUnavailable) => FeedbackReason::Failed,
        ServiceErrorKind::Action(ActionError::PersistentNotification) => FeedbackReason::Rejected,
        ServiceErrorKind::Action(ActionError::Transport) => FeedbackReason::ConnectionLost,
        ServiceErrorKind::Action(ActionError::RuntimeUnavailable) => {
            FeedbackReason::ServiceUnavailable
        }
    }
}

fn sound_feedback_reason(error: SoundPlaybackError) -> FeedbackReason {
    match error {
        SoundPlaybackError::Busy => FeedbackReason::Busy,
        SoundPlaybackError::Playback(rmac_audio::NotificationPlaybackErrorKind::InvalidSound) => {
            FeedbackReason::InvalidMedia
        }
        SoundPlaybackError::Playback(
            rmac_audio::NotificationPlaybackErrorKind::UnsupportedPlatform,
        ) => FeedbackReason::Unsupported,
        SoundPlaybackError::Playback(rmac_audio::NotificationPlaybackErrorKind::Timeout) => {
            FeedbackReason::TimedOut
        }
        SoundPlaybackError::Playback(
            rmac_audio::NotificationPlaybackErrorKind::Prepare
            | rmac_audio::NotificationPlaybackErrorKind::Start
            | rmac_audio::NotificationPlaybackErrorKind::Wait
            | rmac_audio::NotificationPlaybackErrorKind::Rejected,
        ) => FeedbackReason::Failed,
    }
}

fn host_command(command: RuntimeCommand) -> HostCommand {
    match command {
        RuntimeCommand::Redraw => HostCommand::Redraw,
        RuntimeCommand::CapturePreviousFocus => HostCommand::CapturePreviousFocus,
        RuntimeCommand::RestorePreviousFocus => HostCommand::RestorePreviousFocus,
        RuntimeCommand::Expire(id) => HostCommand::Service(ServiceRequest::Expire(id)),
        RuntimeCommand::Dismiss(id) => HostCommand::Service(ServiceRequest::Dismiss(id)),
    }
}

fn validate_application_names(names: &BTreeMap<String, String>) -> Result<(), Error> {
    if names.len() > MAX_APPLICATION_NAMES {
        return Err(Error::TooManyApplicationNames);
    }
    if names
        .keys()
        .any(|app_id| AppId::parse(app_id.clone()).is_err())
    {
        return Err(Error::InvalidApplicationIdentity);
    }
    if names.values().any(|name| {
        name.trim().is_empty()
            || name.len() > rmac_notifications_runtime::presentation::MAX_APPLICATION_NAME_BYTES
            || name.chars().any(char::is_control)
    }) {
        return Err(Error::InvalidApplicationName);
    }
    Ok(())
}

fn validate_activation_token(token: Option<&str>) -> Result<Option<&str>, ServiceErrorKind> {
    let Some(token) = token else {
        return Ok(None);
    };
    if token.is_empty() {
        return Ok(None);
    }
    if token.len() > MAX_ACTIVATION_TOKEN_BYTES || token.chars().any(char::is_control) {
        return Err(ServiceErrorKind::InvalidActivationToken);
    }
    Ok(Some(token))
}

/// Executes one request against the same service authority used by both D-Bus
/// protocols. The caller must feed the resulting runtime events back through
/// [`BannerSession::apply_event`] in their original order. It must not await
/// this future from the sole runtime-event receiver task: publishing the
/// resulting event is bounded and requires that receiver to keep draining.
pub async fn execute_service(
    service: &ServiceHandle,
    request: ServiceRequest,
    activation_token: Option<&str>,
) -> Result<ServiceCompletion, ServiceExecutionError> {
    let result = match request {
        ServiceRequest::Expire(id) => service
            .expire_banner(id)
            .await
            .map(|_| ServiceCompletion::Closed(request)),
        ServiceRequest::Dismiss(id) => service
            .dismiss(id)
            .await
            .map(|_| ServiceCompletion::Closed(request)),
        ServiceRequest::Invoke {
            control,
            notification,
            selection,
        } => {
            let activation_token = validate_activation_token(activation_token)
                .map_err(|kind| ServiceExecutionError { request, kind })?;
            let selection = match selection {
                Selection::Default => ActionSelection::Default,
                Selection::Button(index) => ActionSelection::Button(index),
            };
            service
                .invoke(notification, selection, activation_token)
                .await
                .map(|_| ServiceCompletion::Invoked {
                    control,
                    notification,
                    notification_remains: service
                        .snapshot()
                        .iter()
                        .any(|candidate| candidate.id == notification),
                })
        }
    };
    result.map_err(|error| ServiceExecutionError {
        request,
        kind: ServiceErrorKind::Action(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{
        FocusState, LogicalOutput, LogicalPoint, LogicalSize, Output,
        OutputId as CompositorOutputId, OutputMode, PhysicalSize, Snapshot as CompositorSnapshot,
    };
    use rmac_notifications::protocol::{self, PortalInput, PortalSound};
    use rmac_notifications::{DeliveryPolicy, Server, TimeoutPolicy};

    fn appearance(motion: rmac_appearance::MotionPreference) -> rmac_appearance::Snapshot {
        rmac_appearance::Snapshot {
            motion,
            ..rmac_appearance::Snapshot::default()
        }
    }

    fn compositor() -> CompositorSnapshot {
        CompositorSnapshot {
            outputs: vec![Output {
                id: CompositorOutputId("private-output".into()),
                make: "private".into(),
                model: "private".into(),
                serial: Some("private".into()),
                physical_size_mm: None,
                modes: vec![OutputMode {
                    physical_size: PhysicalSize {
                        width: 1920,
                        height: 1080,
                    },
                    refresh_millihz: 60_000,
                    preferred: true,
                }],
                current_mode: Some(0),
                custom_mode: false,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(LogicalOutput {
                    position: LogicalPoint { x: 0.0, y: 0.0 },
                    size: LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 1.0,
                    transform: "normal".into(),
                }),
            }],
            focus: FocusState {
                output: Some(CompositorOutputId("private-output".into())),
                ..FocusState::default()
            },
            ..CompositorSnapshot::default()
        }
    }

    fn post(
        server: &mut Server,
        title: &str,
        policy: DeliveryPolicy,
    ) -> (PostOutcome, Box<Notification>) {
        let request = protocol::portal(PortalInput {
            app_id: "org.example.Chat".into(),
            id: "message-1".into(),
            title: Some(title.into()),
            body: Some("Private body".into()),
            default_action: Some("app.open".into()),
            sound: PortalSound::Default,
            buttons: vec![protocol::PortalButton {
                label: Some("Reply".into()),
                action: "app.reply".into(),
                ..protocol::PortalButton::default()
            }],
            ..PortalInput::default()
        })
        .unwrap();
        let outcome = server.post(request, Time(0), policy).unwrap();
        let notification = server
            .active()
            .find(|notification| notification.id == outcome.id)
            .unwrap()
            .clone();
        (outcome, Box::new(notification))
    }

    fn posted(
        outcome: PostOutcome,
        notification: Box<Notification>,
        media: NotificationMedia,
    ) -> RuntimeEvent {
        RuntimeEvent::Posted {
            outcome,
            notification: Some(notification),
            media,
        }
    }

    #[test]
    fn post_waits_for_a_real_output_then_publishes_content_icon_and_sound_once() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let (outcome, notification) = post(&mut server, "Private title", DeliveryPolicy::default());
        let id = outcome.id;
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        let update = session
            .apply_event(
                posted(
                    outcome,
                    notification,
                    NotificationMedia {
                        icon: Some(Icon::Themed(vec!["private-icon".into()])),
                        sound: None,
                    },
                ),
                Time(10),
            )
            .unwrap();
        assert!(update.commands.is_empty());
        assert!(update.sounds.is_empty());
        assert!(session.frame().cards.is_empty());
        assert_eq!(session.pending_post_count(), 1);

        let update = session.apply_compositor(&compositor(), Time(20)).unwrap();
        assert_eq!(session.pending_post_count(), 0);
        assert_eq!(session.frame().cards.len(), 1);
        assert_eq!(session.frame().cards[0].title, "Private title");
        assert!(matches!(session.icon(id), Some(Icon::Themed(_))));
        let source = session
            .icon_source(id, crate::icon::Request::new(40, 1.25).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(source.key().notification(), id);
        assert_eq!(source.key().logical_edge(), 40);
        assert_eq!(source.key().pixel_edge(), 50);
        let diagnostics = format!("{source:?}");
        assert!(!diagnostics.contains("org.example.Chat"));
        assert!(!diagnostics.contains("private-icon"));
        assert_eq!(update.sounds, vec![SoundCue::Default(id)]);
        assert_eq!(update.announcements.len(), 1);

        let update = session.apply_compositor(&compositor(), Time(30)).unwrap();
        assert!(update.sounds.is_empty());
        assert!(update.announcements.is_empty());
    }

    #[test]
    fn pointer_activation_keeps_exact_button_identity_and_completion() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let (outcome, notification) = post(&mut server, "Title", DeliveryPolicy::default());
        let id = outcome.id;
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        session.apply_compositor(&compositor(), Time(0)).unwrap();
        session
            .apply_event(
                posted(outcome, notification, NotificationMedia::default()),
                Time(1),
            )
            .unwrap();
        let control = ControlId::Button {
            notification: id,
            index: 0,
        };
        let request = ServiceRequest::Invoke {
            control,
            notification: id,
            selection: Selection::Button(0),
        };
        assert!(session
            .activate_control(control, Time(2))
            .unwrap()
            .commands
            .contains(&HostCommand::Service(request)));
        assert!(session
            .activate_control(ControlId::Card(id), Time(3))
            .unwrap()
            .commands
            .is_empty());
        assert_eq!(
            session.complete_service(
                ServiceCompletion::Invoked {
                    control: ControlId::Card(id),
                    notification: id,
                    notification_remains: true,
                },
                Time(4),
            ),
            Err(Error::StaleCompletion)
        );
        session
            .complete_service(
                ServiceCompletion::Invoked {
                    control,
                    notification: id,
                    notification_remains: true,
                },
                Time(5),
            )
            .unwrap();
        assert_eq!(session.pending_control(), None);
        assert_eq!(session.frame().cards.len(), 1);
    }

    #[test]
    fn failed_action_becomes_exact_accessible_feedback_and_success_clears_it() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let (outcome, notification) = post(
            &mut server,
            "Private failure title",
            DeliveryPolicy::default(),
        );
        let id = outcome.id;
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        session.apply_compositor(&compositor(), Time(0)).unwrap();
        session
            .apply_event(
                posted(outcome, notification, NotificationMedia::default()),
                Time(1),
            )
            .unwrap();
        let control = ControlId::Card(id);
        let request = ServiceRequest::Invoke {
            control,
            notification: id,
            selection: Selection::Default,
        };
        session.activate_control(control, Time(2)).unwrap();
        let update = session
            .fail_service(
                ServiceExecutionError {
                    request,
                    kind: ServiceErrorKind::Action(ActionError::Transport),
                },
                Time(3),
            )
            .unwrap();
        assert_eq!(session.pending_control(), None);
        assert!(update.commands.contains(&HostCommand::Redraw));
        assert_eq!(session.frame().feedback.len(), 1);
        let feedback = session.frame().feedback[0];
        assert_eq!(feedback.notification, id);
        assert_eq!(feedback.operation, FeedbackOperation::Action);
        assert_eq!(feedback.reason, FeedbackReason::ConnectionLost);
        assert_eq!(feedback.live_region(), LiveRegion::Polite);
        assert_eq!(feedback.dismiss_label(), "Dismiss error");
        assert!(feedback
            .accessible_message()
            .contains("connection was lost"));
        assert!(!feedback
            .accessible_message()
            .contains("Private failure title"));
        assert!(!format!("{session:?}").contains("Private failure title"));

        session.activate_control(control, Time(4)).unwrap();
        assert!(session.frame().feedback.is_empty());
        session
            .complete_service(
                ServiceCompletion::Invoked {
                    control,
                    notification: id,
                    notification_remains: true,
                },
                Time(5),
            )
            .unwrap();
        assert!(session.frame().feedback.is_empty());
    }

    #[test]
    fn playback_feedback_is_replaceable_dismissible_and_bounded() {
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        let cue = SoundCue::Default(NotificationId::from_protocol(1).unwrap());
        session
            .fail_sound(&cue, SoundPlaybackError::Busy, Time(1))
            .unwrap();
        let first_id = session.frame().feedback[0].id;
        session
            .fail_sound(
                &cue,
                SoundPlaybackError::Playback(rmac_audio::NotificationPlaybackErrorKind::Timeout),
                Time(2),
            )
            .unwrap();
        assert_eq!(session.frame().feedback.len(), 1);
        assert!(session.frame().feedback[0].id > first_id);
        assert_eq!(session.frame().feedback[0].reason, FeedbackReason::TimedOut);

        for value in 2..=u32::try_from(MAX_RETAINED_FEEDBACK + 2).unwrap() {
            session
                .fail_sound(
                    &SoundCue::Default(NotificationId::from_protocol(value).unwrap()),
                    SoundPlaybackError::Playback(rmac_audio::NotificationPlaybackErrorKind::Start),
                    Time(u64::from(value)),
                )
                .unwrap();
        }
        assert_eq!(session.frame().feedback.len(), MAX_RETAINED_FEEDBACK);
        assert!(session
            .frame()
            .feedback
            .windows(2)
            .all(|pair| pair[0].id < pair[1].id));
        let latest = *session.frame().feedback.last().unwrap();
        assert!(session
            .dismiss_feedback(FeedbackId(latest.id.get().saturating_add(1)), Time(30))
            .commands
            .is_empty());
        assert!(session
            .dismiss_feedback(latest.id, Time(31))
            .commands
            .contains(&HostCommand::Redraw));
        assert_eq!(session.frame().feedback.len(), MAX_RETAINED_FEEDBACK - 1);

        let before = session.frame().feedback.to_vec();
        session.next_feedback = u64::MAX;
        assert_eq!(
            session.fail_sound(
                &SoundCue::Default(NotificationId::from_protocol(99).unwrap()),
                SoundPlaybackError::Busy,
                Time(33),
            ),
            Err(Error::FeedbackIdExhausted)
        );
        assert_eq!(session.frame().feedback, before);
    }

    #[test]
    fn suppressed_replacement_exits_with_previous_content_and_never_adopts_new_media() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let (first, first_notification) = post(
            &mut server,
            "Visible private title",
            DeliveryPolicy::default(),
        );
        let id = first.id;
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Full),
        )
        .unwrap();
        session.apply_compositor(&compositor(), Time(0)).unwrap();
        session
            .apply_event(
                posted(
                    first,
                    first_notification,
                    NotificationMedia {
                        icon: Some(Icon::Themed(vec!["visible-icon".into()])),
                        sound: None,
                    },
                ),
                Time(0),
            )
            .unwrap();
        session.advance(Time(220)).unwrap();

        let (suppressed, suppressed_notification) = post(
            &mut server,
            "Suppressed replacement title",
            DeliveryPolicy {
                banner: rmac_notifications::BannerPolicy::Suppress,
                ..DeliveryPolicy::default()
            },
        );
        assert_eq!(suppressed.id, id);
        session
            .apply_event(
                posted(
                    suppressed,
                    suppressed_notification,
                    NotificationMedia {
                        icon: Some(Icon::Themed(vec!["suppressed-icon".into()])),
                        sound: None,
                    },
                ),
                Time(300),
            )
            .unwrap();
        assert_eq!(session.frame().cards[0].title, "Visible private title");
        assert!(matches!(
            session.icon(id),
            Some(Icon::Themed(names)) if names == &["visible-icon"]
        ));
        assert!(matches!(
            session.frame().cards[0].phase,
            rmac_notifications::banner::PhaseSnapshot::Exiting(_)
        ));
    }

    #[test]
    fn reduced_motion_dismiss_emits_one_authoritative_service_request() {
        let mut server = Server::new(10, TimeoutPolicy::default());
        let (outcome, notification) = post(&mut server, "Title", DeliveryPolicy::default());
        let id = outcome.id;
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        session.apply_compositor(&compositor(), Time(0)).unwrap();
        session
            .apply_event(
                posted(outcome, notification, NotificationMedia::default()),
                Time(1),
            )
            .unwrap();
        let update = session
            .activate_control(ControlId::Dismiss(id), Time(2))
            .unwrap();
        assert_eq!(
            update
                .commands
                .iter()
                .filter(|command| {
                    **command == HostCommand::Service(ServiceRequest::Dismiss(id))
                })
                .count(),
            1
        );
        assert!(session.frame().cards.is_empty());
        assert!(session.icon(id).is_none());
    }

    #[test]
    fn activation_tokens_are_bounded_and_never_printed() {
        assert_eq!(validate_activation_token(None), Ok(None));
        assert_eq!(validate_activation_token(Some("")), Ok(None));
        assert_eq!(
            validate_activation_token(Some("safe-token")),
            Ok(Some("safe-token"))
        );
        assert_eq!(
            validate_activation_token(Some("bad\ntoken")),
            Err(ServiceErrorKind::InvalidActivationToken)
        );
        assert_eq!(
            validate_activation_token(Some(&"x".repeat(MAX_ACTIVATION_TOKEN_BYTES + 1))),
            Err(ServiceErrorKind::InvalidActivationToken)
        );
        let request = ServiceRequest::Invoke {
            control: ControlId::Card(NotificationId::from_protocol(7).unwrap()),
            notification: NotificationId::from_protocol(7).unwrap(),
            selection: Selection::Default,
        };
        let error = ServiceExecutionError {
            request,
            kind: ServiceErrorKind::InvalidActivationToken,
        };
        assert!(!format!("{error:?}").contains("safe-token"));
    }

    #[test]
    fn sound_player_has_one_cancellation_safe_admission() {
        let player = SoundPlayer::new();
        let permit = player.acquire().unwrap();
        assert!(matches!(player.acquire(), Err(SoundPlaybackError::Busy)));
        drop(permit);
        assert!(player.acquire().is_ok());
        assert_eq!(
            notification_sound_format(SoundFormat::OggOpus),
            rmac_audio::NotificationSoundFormat::OggOpus
        );
        assert_eq!(
            notification_sound_format(SoundFormat::OggVorbis),
            rmac_audio::NotificationSoundFormat::OggVorbis
        );
        assert_eq!(
            notification_sound_format(SoundFormat::WavPcm),
            rmac_audio::NotificationSoundFormat::WavPcm
        );
    }

    #[test]
    fn application_catalog_is_bounded_even_for_currently_unused_entries() {
        let mut session = BannerSession::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance(rmac_appearance::MotionPreference::Reduced),
        )
        .unwrap();
        assert_eq!(
            session.set_application_names(
                BTreeMap::from([("bad\nidentity".into(), "Application".into())]),
                Time(0),
            ),
            Err(Error::InvalidApplicationIdentity)
        );
        assert_eq!(
            session.set_application_names(
                BTreeMap::from([("org.example.Valid".into(), "bad\nname".into())]),
                Time(0),
            ),
            Err(Error::InvalidApplicationName)
        );
    }

    #[test]
    fn authority_close_before_action_completion_never_restarts_terminal_motion() {
        for motion in [
            rmac_appearance::MotionPreference::Full,
            rmac_appearance::MotionPreference::Reduced,
        ] {
            let mut server = Server::new(10, TimeoutPolicy::default());
            let (outcome, notification) = post(&mut server, "Title", DeliveryPolicy::default());
            let id = outcome.id;
            let mut session = BannerSession::new(
                Config::default(),
                PlacementPolicy::ActiveOutput,
                &appearance(motion),
            )
            .unwrap();
            session.apply_compositor(&compositor(), Time(0)).unwrap();
            session
                .apply_event(
                    posted(outcome, notification, NotificationMedia::default()),
                    Time(1),
                )
                .unwrap();
            let control = ControlId::Card(id);
            session.activate_control(control, Time(2)).unwrap();
            let closed_update = session
                .apply_event(
                    RuntimeEvent::Closed(rmac_notifications::Closed {
                        id,
                        reason: rmac_notifications::CloseReason::ActionInvoked,
                    }),
                    Time(10),
                )
                .unwrap();
            let phase_before = session.frame().cards.first().map(|card| card.phase);
            let completion_update = session
                .complete_service(
                    ServiceCompletion::Invoked {
                        control,
                        notification: id,
                        notification_remains: false,
                    },
                    Time(100),
                )
                .unwrap();
            assert_eq!(
                session.frame().cards.first().map(|card| card.phase),
                phase_before
            );
            assert_eq!(completion_update.schedule, closed_update.schedule);
            assert_eq!(session.pending_control(), None);
        }
    }
}
