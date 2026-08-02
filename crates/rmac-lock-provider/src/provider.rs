use std::collections::{btree_map::Entry, BTreeMap};
use std::fmt;

use crate::{AttemptId, Event, OutputId, Phase, Transition, UnlockAuthorization};

pub struct Provider {
    phase: Phase,
    outputs: BTreeMap<OutputId, bool>,
    active_attempt: Option<AttemptId>,
    pub(crate) next_attempt: u64,
    failed_attempts: u32,
    pub(crate) ready_notified: bool,
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
