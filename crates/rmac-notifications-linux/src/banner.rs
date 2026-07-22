//! Authority-bound E2 banner session for the eventual Linux layer surface.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use rmac_notifications::banner::{Config, PlacementPolicy, Schedule, Snapshot as LayoutSnapshot};
use rmac_notifications::{AppId, Notification, NotificationId, PostOutcome, Time};
use rmac_notifications_runtime::presentation::{
    Announcement, Card, ControlId, Effect as PresentationEffect, Error as PresentationError,
    Intent, Key, Presenter,
};
use rmac_notifications_runtime::{
    Command as RuntimeCommand, Coordinator, Error as RuntimeError, Update as RuntimeUpdate,
};

use crate::media::{CustomSound, Icon, NotificationMedia, SoundFormat};
use crate::service::{ActionError, ActionSelection, RuntimeEvent, ServiceHandle};

pub const MAX_PENDING_POSTS: usize = 500;
pub const MAX_APPLICATION_NAMES: usize = 4_096;
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
        })
    }

    pub fn frame(&self) -> Frame<'_> {
        Frame {
            layout: self.coordinator.snapshot(),
            cards: self.presenter.cards(),
        }
    }

    pub fn icon(&self, id: NotificationId) -> Option<&Icon> {
        self.icons.get(&id)
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
            ServiceCompletion::Closed(_) => Ok(self.idle(now)),
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
                    update.push_command(HostCommand::Redraw);
                    return Ok(update);
                }
                let runtime = self
                    .coordinator
                    .action_completed(notification, notification_remains, now)
                    .map_err(Error::Runtime)?;
                let mut update = self.finish(runtime, &[], Vec::new(), now)?;
                update.push_command(HostCommand::Redraw);
                Ok(update)
            }
        }
    }

    pub fn fail_service(&mut self, request: ServiceRequest, now: Time) -> Result<Update, Error> {
        let ServiceRequest::Invoke { control, .. } = request else {
            return Ok(self.idle(now));
        };
        if !self.presenter.complete(control) {
            return Err(Error::StaleCompletion);
        }
        let mut update = self.idle(now);
        update.push_command(HostCommand::Redraw);
        Ok(update)
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
                    Ok(self.invoke_update(control, notification, Selection::Default, now))
                }
                Intent::InvokeButton {
                    notification,
                    index,
                } => Ok(self.invoke_update(control, notification, Selection::Button(index), now)),
                Intent::Dismiss(notification) => {
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
            .finish()
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
