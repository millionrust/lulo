//! Deterministic coordinator joining lock wire events, PAM prompts, and input.
//!
//! This module performs no Wayland, PAM, systemd, or logind I/O. A Linux pump
//! executes the returned actions; keeping policy here makes cancellation,
//! stale workers, readiness, and unlock authority executable on any host.

use std::collections::VecDeque;
use std::fmt;

use rmac_lock_provider::{
    AttemptId, Error as ProviderError, Event as ProviderEvent, OutputId, Phase, Provider,
    Transition, UnlockAuthorization,
};

use crate::keyboard::{DecodedKey, EditError, EditOutcome, PromptEditor};
use crate::paint::{LockVisualState, PromptVisual};
use crate::pam_broker::PendingPrompt;
use crate::pam_conversation::RequestKind;

const MAX_QUEUED_INPUTS: usize = 32;
#[cfg(any(target_os = "linux", test))]
const MAX_USERNAME_BYTES: usize = 256;

#[cfg(any(target_os = "linux", test))]
pub(crate) fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= MAX_USERNAME_BYTES
        && !username.as_bytes().contains(&0)
}

pub struct Coordinator {
    provider: Provider,
    attempt: Option<AttemptId>,
    draining_cancelled_worker: Option<AttemptId>,
    editor: Option<PromptEditor>,
    queued_inputs: VecDeque<DecodedKey>,
    authentication_failed_visible: bool,
}

impl Default for Coordinator {
    fn default() -> Self {
        Self {
            provider: Provider::new(),
            attempt: None,
            draining_cancelled_worker: None,
            editor: None,
            queued_inputs: VecDeque::new(),
            authentication_failed_visible: false,
        }
    }
}

impl Coordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> Phase {
        self.provider.phase()
    }

    pub fn failed_attempts(&self) -> u32 {
        self.provider.failed_attempts()
    }

    pub fn prompt_character_count(&self) -> Option<usize> {
        self.editor.as_ref().and_then(PromptEditor::character_count)
    }

    pub fn visual_state(&self) -> LockVisualState {
        let prompt =
            self.editor
                .as_ref()
                .map_or(PromptVisual::Hidden, |editor| match editor.kind() {
                    RequestKind::EchoOff => {
                        PromptVisual::secret(editor.character_count().unwrap_or_default())
                    }
                    RequestKind::EchoOn => {
                        PromptVisual::text(editor.character_count().unwrap_or_default())
                    }
                    RequestKind::Info | RequestKind::Error => PromptVisual::Notice,
                    RequestKind::Radio => PromptVisual::Radio {
                        selected: editor.radio_selection().unwrap_or(false),
                    },
                    RequestKind::Binary => PromptVisual::Binary,
                });
        LockVisualState::new(prompt, self.authentication_failed_visible)
    }

    pub fn apply(&mut self, event: RuntimeEvent) -> Result<RuntimeActions, Error> {
        let mut actions = RuntimeActions::default();
        match event {
            RuntimeEvent::OutputAdded(output) => {
                self.apply_provider(ProviderEvent::OutputAdded(output), &mut actions)?;
            }
            RuntimeEvent::OutputRemoved(output) => {
                self.apply_provider(ProviderEvent::OutputRemoved(output), &mut actions)?;
            }
            RuntimeEvent::FrameCommitted(output) => {
                self.apply_provider(ProviderEvent::FrameCommitted(output), &mut actions)?;
            }
            RuntimeEvent::LockAcquired => {
                self.apply_provider(ProviderEvent::CompositorLocked, &mut actions)?;
                self.start_authentication(&mut actions)?;
            }
            RuntimeEvent::LockFinished => {
                self.editor = None;
                self.queued_inputs.clear();
                self.attempt = None;
                self.draining_cancelled_worker = None;
                self.authentication_failed_visible = false;
                self.apply_provider(ProviderEvent::CompositorFinished, &mut actions)?;
            }
            RuntimeEvent::UnlockFlushed => {
                self.apply_provider(ProviderEvent::UnlockCommitted, &mut actions)?;
            }
            RuntimeEvent::Prompt(prompt) => {
                if self.provider.phase() != Phase::Authenticating || self.attempt.is_none() {
                    return Err(Error::PromptWithoutAttempt);
                }
                if self.editor.is_some() {
                    return Err(Error::DuplicatePrompt);
                }
                self.editor = Some(PromptEditor::new(prompt));
                self.authentication_failed_visible = false;
                actions.prompt_changed = true;
                self.replay_queued_inputs(&mut actions)?;
            }
            RuntimeEvent::Input(input) => {
                self.handle_input(input, &mut actions)?;
            }
            RuntimeEvent::AuthenticationFinished { attempt, outcome } => {
                self.authentication_finished(attempt, outcome, &mut actions)?;
            }
        }
        Ok(actions)
    }

    fn apply_provider(
        &mut self,
        event: ProviderEvent,
        actions: &mut RuntimeActions,
    ) -> Result<(), Error> {
        let transition = self.provider.apply(event).map_err(Error::Provider)?;
        actions.absorb(transition);
        Ok(())
    }

    fn start_authentication(&mut self, actions: &mut RuntimeActions) -> Result<(), Error> {
        if self.attempt.is_some() || self.draining_cancelled_worker.is_some() {
            return Err(Error::WorkerAlreadyActive);
        }
        let transition = self
            .provider
            .apply(ProviderEvent::BeginAuthentication)
            .map_err(Error::Provider)?;
        let attempt = transition
            .authentication_attempt
            .ok_or(Error::MissingAttemptToken)?;
        self.attempt = Some(attempt);
        actions.start_authentication = Some(attempt);
        actions.absorb(transition);
        Ok(())
    }

    fn handle_input(
        &mut self,
        input: DecodedKey,
        actions: &mut RuntimeActions,
    ) -> Result<(), Error> {
        if self.editor.is_some() {
            return self.apply_editor_input(input, actions);
        }
        match self.provider.phase() {
            Phase::Locked => {
                if self.authentication_failed_visible {
                    self.authentication_failed_visible = false;
                    actions.prompt_changed = true;
                }
                self.queue_input(input);
                if self.draining_cancelled_worker.is_none() {
                    self.start_authentication(actions)?;
                }
            }
            Phase::Authenticating => self.queue_input(input),
            Phase::Acquiring
            | Phase::UnlockAuthorized
            | Phase::Denied
            | Phase::FailedLocked
            | Phase::Finished => {}
        }
        Ok(())
    }

    fn replay_queued_inputs(&mut self, actions: &mut RuntimeActions) -> Result<(), Error> {
        while self.editor.is_some() {
            let Some(input) = self.queued_inputs.pop_front() else {
                break;
            };
            self.apply_editor_input(input, actions)?;
        }
        Ok(())
    }

    fn apply_editor_input(
        &mut self,
        input: DecodedKey,
        actions: &mut RuntimeActions,
    ) -> Result<(), Error> {
        let outcome = self
            .editor
            .as_mut()
            .ok_or(Error::PromptUnavailable)?
            .handle(input)
            .map_err(Error::Edit)?;
        match outcome {
            EditOutcome::Changed | EditOutcome::Ignored => {
                actions.prompt_changed |= outcome == EditOutcome::Changed;
            }
            EditOutcome::Completed => {
                self.editor = None;
                self.queued_inputs.clear();
                actions.prompt_changed = true;
            }
            EditOutcome::Cancelled => {
                self.editor = None;
                self.queued_inputs.clear();
                let attempt = self.attempt.take().ok_or(Error::MissingAttemptToken)?;
                self.apply_provider(ProviderEvent::AuthenticationCancelled(attempt), actions)?;
                self.draining_cancelled_worker = Some(attempt);
                self.authentication_failed_visible = false;
                actions.prompt_changed = true;
            }
        }
        Ok(())
    }

    fn authentication_finished(
        &mut self,
        attempt: AttemptId,
        outcome: AuthenticationOutcome,
        actions: &mut RuntimeActions,
    ) -> Result<(), Error> {
        if self.draining_cancelled_worker == Some(attempt) {
            self.draining_cancelled_worker = None;
            if !self.queued_inputs.is_empty() && self.provider.phase() == Phase::Locked {
                self.start_authentication(actions)?;
            }
            return Ok(());
        }
        if self.attempt != Some(attempt) {
            return Err(Error::StaleWorker);
        }
        if outcome == AuthenticationOutcome::Panicked {
            return Err(Error::AuthenticationWorkerPanicked);
        }
        self.attempt = None;
        self.editor = None;
        self.queued_inputs.clear();
        match outcome {
            AuthenticationOutcome::Succeeded => {
                self.authentication_failed_visible = false;
                self.apply_provider(ProviderEvent::AuthenticationSucceeded(attempt), actions)?;
            }
            AuthenticationOutcome::Failed => {
                self.authentication_failed_visible = true;
                self.apply_provider(ProviderEvent::AuthenticationFailed(attempt), actions)?;
                actions.authentication_failed = true;
            }
            AuthenticationOutcome::Panicked => unreachable!(),
        }
        actions.prompt_changed = true;
        Ok(())
    }

    fn queue_input(&mut self, input: DecodedKey) {
        if self.queued_inputs.len() >= MAX_QUEUED_INPUTS {
            return;
        }
        self.queued_inputs.push_back(input);
    }
}

impl fmt::Debug for Coordinator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockRuntimeCoordinator")
            .field("provider", &self.provider)
            .field("attempt", &self.attempt.map(|_| "<redacted>"))
            .field(
                "draining_cancelled_worker",
                &self.draining_cancelled_worker.map(|_| "<redacted>"),
            )
            .field("prompt", &self.editor.as_ref().map(|_| "<redacted>"))
            .field("queued_input", &"<redacted>")
            .field(
                "authentication_failed_visible",
                &self.authentication_failed_visible,
            )
            .finish()
    }
}

pub enum RuntimeEvent {
    OutputAdded(OutputId),
    OutputRemoved(OutputId),
    FrameCommitted(OutputId),
    LockAcquired,
    LockFinished,
    UnlockFlushed,
    Prompt(PendingPrompt),
    Input(DecodedKey),
    AuthenticationFinished {
        attempt: AttemptId,
        outcome: AuthenticationOutcome,
    },
}

impl fmt::Debug for RuntimeEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutputAdded(_) => "RuntimeEvent::OutputAdded(<redacted>)",
            Self::OutputRemoved(_) => "RuntimeEvent::OutputRemoved(<redacted>)",
            Self::FrameCommitted(_) => "RuntimeEvent::FrameCommitted(<redacted>)",
            Self::LockAcquired => "RuntimeEvent::LockAcquired",
            Self::LockFinished => "RuntimeEvent::LockFinished",
            Self::UnlockFlushed => "RuntimeEvent::UnlockFlushed",
            Self::Prompt(_) => "RuntimeEvent::Prompt(<redacted>)",
            Self::Input(_) => "RuntimeEvent::Input(<redacted>)",
            Self::AuthenticationFinished { .. } => {
                "RuntimeEvent::AuthenticationFinished(<redacted>)"
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticationOutcome {
    Succeeded,
    Failed,
    Panicked,
}

/// Why the Linux pump stopped asking the compositor for events.
///
/// Only `AuthenticatedUnlock` permits the process supervisor to clear
/// logind's advisory locked hint. Denial and failure remain distinct because a
/// failure after `locked` must preserve fail-closed recovery state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(any(target_os = "linux", test))]
pub(crate) enum ProviderExit {
    AuthenticatedUnlock,
    Denied,
    FailedLocked,
}

#[derive(Default, Eq, PartialEq)]
pub struct RuntimeActions {
    pub notify_ready: bool,
    pub start_authentication: Option<AttemptId>,
    pub unlock_authorization: Option<UnlockAuthorization>,
    pub exit_provider: bool,
    pub prompt_changed: bool,
    pub authentication_failed: bool,
}

impl RuntimeActions {
    fn absorb(&mut self, transition: Transition) {
        self.notify_ready |= transition.notify_ready;
        if self.start_authentication.is_none() {
            self.start_authentication = transition.authentication_attempt;
        }
        if self.unlock_authorization.is_none() {
            self.unlock_authorization = transition.unlock_authorization;
        }
        self.exit_provider |= transition.exit_provider;
    }
}

impl fmt::Debug for RuntimeActions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeActions")
            .field("notify_ready", &self.notify_ready)
            .field(
                "start_authentication",
                &self.start_authentication.map(|_| "<redacted>"),
            )
            .field(
                "unlock_authorization",
                &self.unlock_authorization.as_ref().map(|_| "<redacted>"),
            )
            .field("exit_provider", &self.exit_provider)
            .field("prompt_changed", &self.prompt_changed)
            .field("authentication_failed", &self.authentication_failed)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Provider(ProviderError),
    Edit(EditError),
    PromptWithoutAttempt,
    DuplicatePrompt,
    PromptUnavailable,
    MissingAttemptToken,
    WorkerAlreadyActive,
    StaleWorker,
    AuthenticationWorkerPanicked,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("secure lock runtime transition failed")
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use super::*;
    use std::time::Duration;

    use crate::pam::{spawn_authentication, Worker, WorkerError};
    use crate::pam_broker::{conversation_channel, ConversationUi, UiDisconnected};
    use crate::wayland::{LockConnection, PreparedConnection, PreparedEvent, WireError};

    const MAX_POLL_WAIT: Duration = Duration::from_millis(50);

    /// Crate-internal Linux pump. It is deliberately not installed or exported
    /// until systemd/logind recovery evidence exists.
    #[allow(dead_code)]
    pub(crate) struct LinuxRuntime {
        coordinator: Coordinator,
        wire: LockConnection,
        username: String,
        authentication: Option<ActiveAuthentication>,
    }

    struct ActiveAuthentication {
        attempt: AttemptId,
        worker: Worker,
        ui: ConversationUi,
    }

    #[allow(dead_code)]
    impl LinuxRuntime {
        pub(crate) fn connect(username: String) -> Result<Self, LinuxError> {
            if !valid_username(&username) {
                return Err(LinuxError::InvalidUsername);
            }
            let prepared = PreparedConnection::connect().map_err(LinuxError::Prepare)?;
            let wire = prepared.acquire_for_runtime().map_err(LinuxError::Wire)?;
            Ok(Self {
                coordinator: Coordinator::new(),
                wire,
                username,
                authentication: None,
            })
        }

        pub(crate) fn poll(&mut self, timeout: Duration) -> Result<RuntimeStatus, LinuxError> {
            self.wire
                .poll_dispatch(timeout.min(MAX_POLL_WAIT))
                .map_err(LinuxError::Wire)?;
            let events = self.wire.drain_events().collect::<Vec<_>>();
            let mut status = RuntimeStatus::default();
            for event in events {
                let Some(event) = runtime_event(event) else {
                    continue;
                };
                let actions = self.coordinator.apply(event).map_err(LinuxError::Runtime)?;
                self.execute(actions, &mut status)?;
                if status.exit.is_some() {
                    return Ok(status);
                }
            }
            self.poll_authentication(&mut status)?;
            Ok(status)
        }

        fn poll_authentication(&mut self, status: &mut RuntimeStatus) -> Result<(), LinuxError> {
            loop {
                let prompt = match self.authentication.as_ref() {
                    Some(active) => match active.ui.try_prompt() {
                        Ok(Some(prompt)) => Some(prompt),
                        Ok(None) => None,
                        Err(UiDisconnected) => None,
                    },
                    None => return Ok(()),
                };
                let Some(prompt) = prompt else {
                    break;
                };
                let actions = self
                    .coordinator
                    .apply(RuntimeEvent::Prompt(prompt))
                    .map_err(LinuxError::Runtime)?;
                self.execute(actions, status)?;
            }

            let finished = self
                .authentication
                .as_ref()
                .is_some_and(|active| active.worker.is_finished());
            if !finished {
                return Ok(());
            }
            let active = self
                .authentication
                .take()
                .ok_or(LinuxError::MissingWorker)?;
            let outcome = match active.worker.join() {
                Ok(()) => AuthenticationOutcome::Succeeded,
                Err(WorkerError::Pam(_)) => AuthenticationOutcome::Failed,
                Err(WorkerError::Panicked) => AuthenticationOutcome::Panicked,
            };
            let actions = self
                .coordinator
                .apply(RuntimeEvent::AuthenticationFinished {
                    attempt: active.attempt,
                    outcome,
                })
                .map_err(LinuxError::Runtime)?;
            self.execute(actions, status)
        }

        fn execute(
            &mut self,
            mut actions: RuntimeActions,
            status: &mut RuntimeStatus,
        ) -> Result<(), LinuxError> {
            if actions.prompt_changed || actions.authentication_failed {
                self.wire.set_visual_state(self.coordinator.visual_state());
            }
            if let Some(attempt) = actions.start_authentication.take() {
                if self.authentication.is_some() {
                    return Err(LinuxError::DuplicateWorker);
                }
                let (conversation, ui) = conversation_channel();
                let worker = spawn_authentication(self.username.clone(), conversation)
                    .map_err(LinuxError::SpawnWorker)?;
                self.authentication = Some(ActiveAuthentication {
                    attempt,
                    worker,
                    ui,
                });
            }
            if let Some(authorization) = actions.unlock_authorization.take() {
                self.wire
                    .unlock_and_flush(authorization)
                    .map_err(LinuxError::Wire)?;
            }
            status.notify_ready |= actions.notify_ready;
            status.prompt_changed |= actions.prompt_changed;
            status.authentication_failed |= actions.authentication_failed;
            if actions.exit_provider {
                let exit = match self.coordinator.phase() {
                    Phase::Finished => ProviderExit::AuthenticatedUnlock,
                    Phase::Denied => ProviderExit::Denied,
                    Phase::FailedLocked => ProviderExit::FailedLocked,
                    _ => return Err(LinuxError::InvalidExitPhase),
                };
                if status.exit.replace(exit).is_some() {
                    return Err(LinuxError::DuplicateExit);
                }
            }
            Ok(())
        }
    }

    impl fmt::Debug for LinuxRuntime {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("LinuxLockRuntime")
                .field("coordinator", &self.coordinator)
                .field("username", &"<redacted>")
                .field(
                    "authentication",
                    &self.authentication.as_ref().map(|_| "<redacted>"),
                )
                .finish_non_exhaustive()
        }
    }

    fn runtime_event(event: PreparedEvent) -> Option<RuntimeEvent> {
        match event {
            PreparedEvent::OutputAdded(output) => Some(RuntimeEvent::OutputAdded(output)),
            PreparedEvent::OutputRemoved(output) => Some(RuntimeEvent::OutputRemoved(output)),
            PreparedEvent::FrameCommitted(output) => Some(RuntimeEvent::FrameCommitted(output)),
            PreparedEvent::KeyboardInput { input, .. } => Some(RuntimeEvent::Input(input)),
            PreparedEvent::LockAcquired => Some(RuntimeEvent::LockAcquired),
            PreparedEvent::LockFinished => Some(RuntimeEvent::LockFinished),
            PreparedEvent::UnlockFlushed => Some(RuntimeEvent::UnlockFlushed),
            PreparedEvent::OutputScaleChanged { .. }
            | PreparedEvent::KeyboardAvailabilityChanged { .. }
            | PreparedEvent::KeyboardFocusChanged { .. }
            | PreparedEvent::KeyboardRepeat { .. } => None,
        }
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(crate) struct RuntimeStatus {
        pub(crate) notify_ready: bool,
        pub(crate) prompt_changed: bool,
        pub(crate) authentication_failed: bool,
        pub(crate) exit: Option<ProviderExit>,
    }

    #[allow(dead_code)]
    #[derive(Debug)]
    pub(crate) enum LinuxError {
        InvalidUsername,
        Prepare(crate::wayland::PrepareError),
        Wire(WireError),
        Runtime(Error),
        SpawnWorker(std::io::Error),
        MissingWorker,
        DuplicateWorker,
        InvalidExitPhase,
        DuplicateExit,
    }

    impl fmt::Display for LinuxError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("Linux secure lock runtime failed")
        }
    }

    impl std::error::Error for LinuxError {}

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn wire_events_map_without_exposing_seat_metadata() {
            let output = OutputId::new(7).unwrap();
            assert!(matches!(
                runtime_event(PreparedEvent::OutputAdded(output)),
                Some(RuntimeEvent::OutputAdded(candidate)) if candidate == output
            ));
            assert!(matches!(
                runtime_event(PreparedEvent::LockAcquired),
                Some(RuntimeEvent::LockAcquired)
            ));
            assert!(
                runtime_event(PreparedEvent::OutputScaleChanged { output, scale: 2 }).is_none()
            );
        }

        #[test]
        fn invalid_username_fails_before_connecting_or_locking() {
            assert!(matches!(
                LinuxRuntime::connect(String::new()),
                Err(LinuxError::InvalidUsername)
            ));
            assert!(matches!(
                LinuxRuntime::connect("x".repeat(MAX_USERNAME_BYTES + 1)),
                Err(LinuxError::InvalidUsername)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::DecodedText;
    use crate::pam_broker::conversation_channel;
    use crate::pam_conversation::{Conversation, ConversationError, Reply, Request};
    use std::thread;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(2);

    fn output(value: u64) -> OutputId {
        OutputId::new(value).unwrap()
    }

    fn text(value: &str) -> DecodedKey {
        DecodedKey::Text(DecodedText::new(value.to_owned()).unwrap())
    }

    #[test]
    fn pam_username_is_bounded_before_any_platform_connection() {
        assert!(valid_username("user"));
        assert!(!valid_username(""));
        assert!(!valid_username("bad\0name"));
        assert!(!valid_username(&"x".repeat(MAX_USERNAME_BYTES + 1)));
    }

    fn acquire(coordinator: &mut Coordinator) -> AttemptId {
        coordinator
            .apply(RuntimeEvent::OutputAdded(output(1)))
            .unwrap();
        coordinator
            .apply(RuntimeEvent::FrameCommitted(output(1)))
            .unwrap();
        let actions = coordinator.apply(RuntimeEvent::LockAcquired).unwrap();
        assert!(actions.notify_ready);
        actions.start_authentication.unwrap()
    }

    #[test]
    fn successful_prompt_produces_only_core_unlock_authority() {
        let mut coordinator = Coordinator::new();
        let attempt = acquire(&mut coordinator);
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        coordinator.apply(RuntimeEvent::Prompt(pending)).unwrap();
        assert!(matches!(
            coordinator.visual_state().prompt(),
            PromptVisual::Secret { dots: 0 }
        ));
        coordinator
            .apply(RuntimeEvent::Input(text("sécure")))
            .unwrap();
        assert!(matches!(
            coordinator.visual_state().prompt(),
            PromptVisual::Secret { dots: 6 }
        ));
        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Submit))
            .unwrap();

        let Reply::Secret(secret) = worker.join().unwrap().unwrap() else {
            panic!("wrong response style");
        };
        secret.expose(|value| assert_eq!(value, "sécure".as_bytes()));
        let actions = coordinator
            .apply(RuntimeEvent::AuthenticationFinished {
                attempt,
                outcome: AuthenticationOutcome::Succeeded,
            })
            .unwrap();
        assert!(actions.unlock_authorization.is_some());
        assert!(!actions.exit_provider);
        assert_eq!(coordinator.phase(), Phase::UnlockAuthorized);
    }

    #[test]
    fn cancellation_drains_old_worker_before_reusing_queued_input() {
        let mut coordinator = Coordinator::new();
        let first = acquire(&mut coordinator);
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        coordinator.apply(RuntimeEvent::Prompt(pending)).unwrap();
        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Cancel))
            .unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));

        let actions = coordinator.apply(RuntimeEvent::Input(text("n"))).unwrap();
        assert!(actions.start_authentication.is_none());
        let actions = coordinator
            .apply(RuntimeEvent::AuthenticationFinished {
                attempt: first,
                outcome: AuthenticationOutcome::Failed,
            })
            .unwrap();
        let second = actions.start_authentication.unwrap();
        assert_ne!(first, second);

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        coordinator.apply(RuntimeEvent::Prompt(pending)).unwrap();
        assert_eq!(coordinator.prompt_character_count(), Some(1));
        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Cancel))
            .unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
    }

    #[test]
    fn failure_waits_for_bounded_user_retry_and_rejects_stale_workers() {
        let mut coordinator = Coordinator::new();
        let first = acquire(&mut coordinator);
        let actions = coordinator
            .apply(RuntimeEvent::AuthenticationFinished {
                attempt: first,
                outcome: AuthenticationOutcome::Failed,
            })
            .unwrap();
        assert!(actions.authentication_failed);
        assert!(coordinator.visual_state().authentication_failed());
        assert!(actions.start_authentication.is_none());
        assert_eq!(coordinator.failed_attempts(), 1);

        let actions = coordinator.apply(RuntimeEvent::Input(text("x"))).unwrap();
        let second = actions.start_authentication.unwrap();
        assert_eq!(
            coordinator.apply(RuntimeEvent::AuthenticationFinished {
                attempt: first,
                outcome: AuthenticationOutcome::Succeeded,
            }),
            Err(Error::StaleWorker)
        );
        assert_ne!(first, second);
    }

    #[test]
    fn compositor_finish_and_worker_panic_never_authorize_unlock() {
        let mut coordinator = Coordinator::new();
        let attempt = acquire(&mut coordinator);
        assert_eq!(
            coordinator.apply(RuntimeEvent::AuthenticationFinished {
                attempt,
                outcome: AuthenticationOutcome::Panicked,
            }),
            Err(Error::AuthenticationWorkerPanicked)
        );

        let actions = coordinator.apply(RuntimeEvent::LockFinished).unwrap();
        assert!(actions.exit_provider);
        assert!(actions.unlock_authorization.is_none());
        assert_eq!(coordinator.phase(), Phase::FailedLocked);
    }

    #[test]
    fn input_queue_is_bounded_and_diagnostics_are_redacted() {
        let mut coordinator = Coordinator::new();
        acquire(&mut coordinator);
        for _ in 0..MAX_QUEUED_INPUTS {
            coordinator
                .apply(RuntimeEvent::Input(text("secret")))
                .unwrap();
        }
        coordinator
            .apply(RuntimeEvent::Input(text("overflow")))
            .unwrap();
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        coordinator.apply(RuntimeEvent::Prompt(pending)).unwrap();
        assert_eq!(
            coordinator.prompt_character_count(),
            Some(MAX_QUEUED_INPUTS * 6)
        );
        let debug = format!("{coordinator:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains(MAX_QUEUED_INPUTS.to_string().as_str()));
        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Cancel))
            .unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
    }
}
