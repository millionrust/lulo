//! Security state machine for a future rmac `ext-session-lock-v1` provider.
//!
//! This crate deliberately contains no Wayland, rendering, PAM, filesystem, or
//! D-Bus adapter. In particular, credentials can never enter this model. A
//! platform adapter owns each secret for one bounded PAM conversation and feeds
//! back only the matching attempt identifier plus success, failure, or cancel.

use std::collections::{btree_map::Entry, BTreeMap};
use std::fmt;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct OutputId(u64);

impl OutputId {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for OutputId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutputId(<redacted>)")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AttemptId(u64);

impl AttemptId {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttemptId(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Acquiring,
    Locked,
    Authenticating,
    UnlockAuthorized,
    Denied,
    FailedLocked,
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    OutputAdded(OutputId),
    OutputRemoved(OutputId),
    FrameCommitted(OutputId),
    CompositorLocked,
    BeginAuthentication,
    AuthenticationSucceeded(AttemptId),
    AuthenticationFailed(AttemptId),
    AuthenticationCancelled(AttemptId),
    CompositorFinished,
    UnlockCommitted,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Transition {
    /// Complete the systemd `Type=notify` start transaction. This is emitted
    /// only for the compositor's `locked` event, never merely for a frame.
    pub notify_ready: bool,
    /// Start one PAM conversation. The adapter must not retain its credential
    /// after returning an outcome for this exact identifier.
    pub authentication_attempt: Option<AttemptId>,
    /// Move-only authority to send `unlock_and_destroy`. Only a matching
    /// successful authentication can construct this token.
    pub unlock_authorization: Option<UnlockAuthorization>,
    /// Exit the provider after a compositor denial/failure or after the unlock
    /// request has been flushed. No exit transition itself unlocks a session.
    pub exit_provider: bool,
}

/// One-shot authority for the Linux wire to request unlock.
///
/// The private field prevents adapters from constructing this token directly;
/// it is intentionally neither `Clone` nor `Copy`.
#[derive(Eq, PartialEq)]
pub struct UnlockAuthorization {
    _private: (),
}

impl fmt::Debug for UnlockAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UnlockAuthorization(<redacted>)")
    }
}

pub struct Provider {
    phase: Phase,
    outputs: BTreeMap<OutputId, bool>,
    active_attempt: Option<AttemptId>,
    next_attempt: u64,
    failed_attempts: u32,
    ready_notified: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            phase: Phase::Acquiring,
            outputs: BTreeMap::new(),
            active_attempt: None,
            next_attempt: 1,
            failed_attempts: 0,
            ready_notified: false,
        }
    }
}

impl fmt::Debug for Provider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Provider")
            .field("phase", &self.phase)
            .field(
                "outputs",
                &format_args!("<{} redacted>", self.outputs.len()),
            )
            .field("active_attempt", &self.active_attempt.map(|_| "<redacted>"))
            .field("failed_attempts", &self.failed_attempts)
            .field("ready_notified", &self.ready_notified)
            .finish()
    }
}

impl Provider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Client-side commit completeness is not proof of presentation or the
    /// security readiness boundary. The compositor may securely blank an
    /// output while the client catches up; its `locked` event is authoritative.
    pub fn frames_committed(&self) -> bool {
        !self.outputs.is_empty() && self.outputs.values().all(|committed| *committed)
    }

    pub fn apply(&mut self, event: Event) -> Result<Transition, Error> {
        if matches!(
            self.phase,
            Phase::Denied | Phase::FailedLocked | Phase::Finished
        ) {
            return Err(Error::Terminal);
        }

        match event {
            Event::OutputAdded(output) => {
                match self.outputs.entry(output) {
                    Entry::Vacant(entry) => {
                        entry.insert(false);
                    }
                    Entry::Occupied(_) => return Err(Error::DuplicateOutput),
                }
                Ok(Transition::default())
            }
            Event::OutputRemoved(output) => {
                if self.outputs.remove(&output).is_none() {
                    return Err(Error::UnknownOutput);
                }
                Ok(Transition::default())
            }
            Event::FrameCommitted(output) => {
                let committed = self.outputs.get_mut(&output).ok_or(Error::UnknownOutput)?;
                *committed = true;
                Ok(Transition::default())
            }
            Event::CompositorLocked if self.phase == Phase::Acquiring => {
                self.phase = Phase::Locked;
                let notify_ready = !self.ready_notified;
                self.ready_notified = true;
                Ok(Transition {
                    notify_ready,
                    ..Transition::default()
                })
            }
            Event::BeginAuthentication if self.phase == Phase::Locked => {
                if self.next_attempt == 0 {
                    return Err(Error::AttemptExhausted);
                }
                let attempt = AttemptId(self.next_attempt);
                self.next_attempt = self.next_attempt.checked_add(1).unwrap_or_default();
                self.active_attempt = Some(attempt);
                self.phase = Phase::Authenticating;
                Ok(Transition {
                    authentication_attempt: Some(attempt),
                    ..Transition::default()
                })
            }
            Event::AuthenticationSucceeded(attempt) if self.phase == Phase::Authenticating => {
                self.require_attempt(attempt)?;
                self.active_attempt = None;
                self.phase = Phase::UnlockAuthorized;
                Ok(Transition {
                    unlock_authorization: Some(UnlockAuthorization { _private: () }),
                    ..Transition::default()
                })
            }
            Event::AuthenticationFailed(attempt) if self.phase == Phase::Authenticating => {
                self.require_attempt(attempt)?;
                self.active_attempt = None;
                self.failed_attempts = self.failed_attempts.saturating_add(1);
                self.phase = Phase::Locked;
                Ok(Transition::default())
            }
            Event::AuthenticationCancelled(attempt) if self.phase == Phase::Authenticating => {
                self.require_attempt(attempt)?;
                self.active_attempt = None;
                self.phase = Phase::Locked;
                Ok(Transition::default())
            }
            Event::CompositorFinished => {
                self.active_attempt = None;
                self.phase = if self.ready_notified {
                    Phase::FailedLocked
                } else {
                    Phase::Denied
                };
                Ok(Transition {
                    exit_provider: true,
                    ..Transition::default()
                })
            }
            Event::UnlockCommitted if self.phase == Phase::UnlockAuthorized => {
                self.phase = Phase::Finished;
                Ok(Transition {
                    exit_provider: true,
                    ..Transition::default()
                })
            }
            _ => Err(Error::InvalidTransition),
        }
    }

    fn require_attempt(&self, attempt: AttemptId) -> Result<(), Error> {
        if self.active_attempt == Some(attempt) {
            Ok(())
        } else {
            Err(Error::StaleAttempt)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidTransition,
    DuplicateOutput,
    UnknownOutput,
    StaleAttempt,
    AttemptExhausted,
    Terminal,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "secure lock provider transition failed ({self:?})"
        )
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(value: u64) -> OutputId {
        OutputId::new(value).unwrap()
    }

    fn lock(provider: &mut Provider) {
        let transition = provider.apply(Event::CompositorLocked).unwrap();
        assert!(transition.notify_ready);
        assert_eq!(provider.phase(), Phase::Locked);
    }

    #[test]
    fn readiness_comes_only_from_the_compositor_locked_event() {
        let mut provider = Provider::new();
        provider.apply(Event::OutputAdded(output(1))).unwrap();
        provider.apply(Event::FrameCommitted(output(1))).unwrap();
        assert!(provider.frames_committed());
        assert!(!provider.ready_notified);

        lock(&mut provider);
        assert_eq!(
            provider.apply(Event::CompositorLocked),
            Err(Error::InvalidTransition)
        );
    }

    #[test]
    fn hotplug_tracks_frame_commits_without_weakening_locked_readiness() {
        let mut provider = Provider::new();
        provider.apply(Event::OutputAdded(output(1))).unwrap();
        provider.apply(Event::FrameCommitted(output(1))).unwrap();
        lock(&mut provider);
        assert!(provider.frames_committed());

        provider.apply(Event::OutputAdded(output(2))).unwrap();
        assert!(!provider.frames_committed());
        provider.apply(Event::FrameCommitted(output(2))).unwrap();
        assert!(provider.frames_committed());
        provider.apply(Event::OutputRemoved(output(1))).unwrap();
        assert!(provider.frames_committed());
        assert_eq!(provider.output_count(), 1);
        assert_eq!(
            provider.apply(Event::OutputRemoved(output(1))),
            Err(Error::UnknownOutput)
        );
    }

    #[test]
    fn only_the_matching_successful_attempt_authorizes_unlock() {
        let mut provider = Provider::new();
        lock(&mut provider);
        assert_eq!(
            provider.apply(Event::AuthenticationSucceeded(AttemptId(99))),
            Err(Error::InvalidTransition)
        );
        let attempt = provider
            .apply(Event::BeginAuthentication)
            .unwrap()
            .authentication_attempt
            .unwrap();
        assert_eq!(
            provider.apply(Event::AuthenticationSucceeded(AttemptId(attempt.get() + 1))),
            Err(Error::StaleAttempt)
        );
        assert_eq!(provider.phase(), Phase::Authenticating);
        let transition = provider
            .apply(Event::AuthenticationSucceeded(attempt))
            .unwrap();
        let authorization = transition.unlock_authorization.unwrap();
        assert_eq!(
            format!("{authorization:?}"),
            "UnlockAuthorization(<redacted>)"
        );
        assert_eq!(provider.phase(), Phase::UnlockAuthorized);
        assert_eq!(
            provider.apply(Event::BeginAuthentication),
            Err(Error::InvalidTransition)
        );
        assert!(
            provider
                .apply(Event::UnlockCommitted)
                .unwrap()
                .exit_provider
        );
        assert_eq!(provider.phase(), Phase::Finished);
    }

    #[test]
    fn failure_and_cancel_return_to_locked_without_unlocking() {
        let mut provider = Provider::new();
        lock(&mut provider);
        let first = provider
            .apply(Event::BeginAuthentication)
            .unwrap()
            .authentication_attempt
            .unwrap();
        assert_eq!(
            provider.apply(Event::AuthenticationFailed(first)).unwrap(),
            Transition::default()
        );
        assert_eq!(provider.phase(), Phase::Locked);
        assert_eq!(provider.failed_attempts(), 1);

        let second = provider
            .apply(Event::BeginAuthentication)
            .unwrap()
            .authentication_attempt
            .unwrap();
        provider
            .apply(Event::AuthenticationCancelled(second))
            .unwrap();
        assert_eq!(provider.phase(), Phase::Locked);
        assert_eq!(provider.failed_attempts(), 1);
    }

    #[test]
    fn authentication_tokens_never_wrap_or_reuse() {
        let mut provider = Provider::new();
        lock(&mut provider);
        provider.next_attempt = u64::MAX;
        let final_attempt = provider
            .apply(Event::BeginAuthentication)
            .unwrap()
            .authentication_attempt
            .unwrap();
        assert_eq!(final_attempt.get(), u64::MAX);
        provider
            .apply(Event::AuthenticationFailed(final_attempt))
            .unwrap();
        assert_eq!(
            provider.apply(Event::BeginAuthentication),
            Err(Error::AttemptExhausted)
        );
        assert_eq!(provider.phase(), Phase::Locked);
    }

    #[test]
    fn compositor_finish_is_fail_closed_before_and_after_readiness() {
        let mut denied = Provider::new();
        let transition = denied.apply(Event::CompositorFinished).unwrap();
        assert!(transition.exit_provider);
        assert!(transition.unlock_authorization.is_none());
        assert_eq!(denied.phase(), Phase::Denied);

        let mut failed_locked = Provider::new();
        lock(&mut failed_locked);
        let transition = failed_locked.apply(Event::CompositorFinished).unwrap();
        assert!(transition.exit_provider);
        assert!(transition.unlock_authorization.is_none());
        assert_eq!(failed_locked.phase(), Phase::FailedLocked);
        assert_eq!(
            failed_locked.apply(Event::UnlockCommitted),
            Err(Error::Terminal)
        );
    }

    #[test]
    fn diagnostics_redact_output_and_attempt_identity() {
        let mut provider = Provider::new();
        provider.apply(Event::OutputAdded(output(8675309))).unwrap();
        lock(&mut provider);
        provider.next_attempt = 424_242;
        let attempt = provider
            .apply(Event::BeginAuthentication)
            .unwrap()
            .authentication_attempt
            .unwrap();
        let debug = format!("{provider:?} {attempt:?} {:?}", output(8675309));
        assert!(!debug.contains("8675309"));
        assert!(!debug.contains("424242"));
        assert!(debug.contains("<redacted>"));
        assert!(OutputId::new(0).is_none());
    }
}
